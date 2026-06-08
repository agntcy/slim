// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;

use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::NameId;

use crate::api::proto::controller::proto::v1::ConnectionDirection;
use crate::db::{LinkStatus, RouteStatus};

use super::connection_config::{ReportedConnection, find_reported_connection};
use super::*;

impl super::RouteService {
    /// Called when a new node registers. Syncs link state against the connections
    /// the data plane reported, then triggers reconciliation as needed.
    pub async fn node_registered(
        &self,
        node_id: &str,
        conn_details_updated: bool,
        dp_connections: Vec<crate::api::proto::controller::proto::v1::ConnectionEntry>,
        dp_routes: Vec<crate::api::proto::controller::proto::v1::Route>,
    ) {
        // Serialize with node_deregistered for the same node to prevent a
        // rapid disconnect-reconnect race from leaving stale link records.
        let node_lock = {
            let mut locks = self.0.node_locks.lock().await;
            locks
                .entry(node_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _node_guard = node_lock.lock().await;

        // Build the set of link IDs the DP still has active.
        let active_link_ids: HashSet<String> = dp_connections
            .iter()
            .filter_map(|e| e.link_id.as_deref().filter(|id| !id.is_empty()))
            .map(str::to_string)
            .collect();

        // Store reported outgoing connections for static connection adoption.
        let mut reported: Vec<ReportedConnection> = Vec::new();
        for entry in &dp_connections {
            if entry.direction != ConnectionDirection::Outgoing as i32 {
                continue;
            }
            let Some(link_id) = entry.link_id.as_deref().filter(|id| !id.is_empty()) else {
                continue;
            };
            if let Ok(config) = serde_json::from_str::<ClientConfig>(&entry.config_data) {
                reported.push(ReportedConnection {
                    endpoint: config.endpoint.clone(),
                    link_id: link_id.to_string(),
                    config_data: config,
                });
            }
        }
        let mut link_reconcile_nodes: HashSet<String> = HashSet::new();

        // True only when the DP explicitly sent a non-empty connections list.
        let dp_reported_connections = !dp_connections.is_empty();

        // Single pass over all links involving this node.
        let links = match self.0.db.get_links_for_node(node_id).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("node_registered: get_links_for_node: {e}");
                return;
            }
        };
        for mut link in links {
            if link.status == LinkStatus::Deleted {
                self.restore_deleted_link(node_id, &mut link, &mut link_reconcile_nodes)
                    .await;
            }

            if link.dest_node_id == node_id {
                self.handle_incoming_link_on_register(
                    node_id,
                    &mut link,
                    conn_details_updated,
                    dp_reported_connections,
                    &active_link_ids,
                    &mut link_reconcile_nodes,
                )
                .await;
            } else if link.source_node_id == node_id {
                self.adopt_static_connection(node_id, &mut link, &reported)
                    .await;
            }
        }

        let all_nodes = match self.0.db.list_nodes().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("node_registered: list_nodes: {e}");
                return;
            }
        };

        self.rebuild_link_graph(&all_nodes).await;

        let mut node_links = match self.0.db.get_links_for_node(node_id).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("node_registered: get_links_for_node: {e}");
                return;
            }
        };
        let (affected_nodes, new_links) = self
            .ensure_links_for_node(node_id, &node_links, &all_nodes, &reported)
            .await;
        for nid in affected_nodes {
            link_reconcile_nodes.insert(nid);
        }
        node_links.extend(new_links);

        // Restore soft-deleted routes where this node is the destination.
        let dest_routes = match self.0.db.get_routes_for_dest_node_id(node_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("node_registered: get_routes_for_dest_node_id: {e}");
                return;
            }
        };
        for route in dest_routes {
            if route.status != RouteStatus::Deleted {
                continue;
            }
            let link_id = node_links
                .iter()
                .find(|l| {
                    l.status != LinkStatus::Deleted
                        && ((l.source_node_id == route.source_node_id && l.dest_node_id == node_id)
                            || (l.source_node_id == node_id
                                && l.dest_node_id == route.source_node_id))
                })
                .map(|l| l.link_id.as_str())
                .or(route.link_id.as_deref())
                .unwrap_or_default();
            if let Err(e) = self.0.db.restore_route(&route.id, link_id).await {
                tracing::warn!("node_registered: failed to restore route {}: {e}", route.id);
            } else {
                self.0.queue.add(route.source_node_id.clone());
            }
        }

        // Re-expand wildcard routes via SPT. This is idempotent — duplicates
        // are rejected by add_single_route.
        let all_links = self.0.db.list_all_links().await.unwrap_or_default();
        self.expand_all_wildcard_routes(&all_nodes, &all_links)
            .await;

        // Mark DP-reported routes as Applied to avoid redundant reconciler pushes.
        if !dp_routes.is_empty() {
            let db_routes = match self.0.db.get_routes_for_node(node_id).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("node_registered: get_routes_for_node: {e}");
                    return;
                }
            };
            for db_route in &db_routes {
                if db_route.status != RouteStatus::Pending {
                    continue;
                }
                let matched = dp_routes.iter().any(|dp| {
                    let (c0, c1, c2) = dp.name.as_ref().unwrap().str_components();
                    let comp_id = if dp.name.as_ref().unwrap().id() == NameId::NULL_COMPONENT {
                        None
                    } else {
                        Some(dp.name.as_ref().unwrap().string_id())
                    };
                    c0 == db_route.component0
                        && c1 == db_route.component1
                        && c2 == db_route.component2
                        && comp_id == db_route.component_id
                        && dp.link_id.as_deref() == db_route.link_id.as_deref()
                });
                if matched && let Err(e) = self.0.db.mark_route_applied(&db_route.id).await {
                    tracing::warn!(
                        "node_registered: failed to mark route {} applied: {e}",
                        db_route.id
                    );
                }
            }
        }

        // Enqueue all affected nodes for reconciliation.
        link_reconcile_nodes.insert(node_id.to_string());
        for nid in link_reconcile_nodes {
            self.0.queue.add(nid);
        }
    }

    /// Handle an incoming link (dest == registering node) during registration.
    /// Syncs the link status based on DP connection reports.
    pub(super) async fn handle_incoming_link_on_register(
        &self,
        node_id: &str,
        link: &mut crate::db::Link,
        conn_details_updated: bool,
        dp_reported_connections: bool,
        active_link_ids: &HashSet<String>,
        link_reconcile_nodes: &mut HashSet<String>,
    ) {
        // DP confirms this link is still active and endpoint hasn't changed.
        if !conn_details_updated
            && dp_reported_connections
            && active_link_ids.contains(&link.link_id)
        {
            if link.status != LinkStatus::Applied {
                save_link(
                    &self.0.db,
                    link,
                    LinkStatus::Applied,
                    &format!("failed to mark link {} applied", link.link_id),
                )
                .await;
            }
            self.0.queue.add(link.source_node_id.clone());
            return;
        }

        // Nothing changed and link is already Applied — just enqueue reconciliation.
        if !conn_details_updated && link.status == LinkStatus::Applied {
            self.0.queue.add(link.source_node_id.clone());
            return;
        }

        // Endpoint changed or DP says link is dead — refresh config and reset to Pending.
        if conn_details_updated {
            match self
                .get_client_config(&link.source_node_id, node_id)
                .await
            {
                Ok((endpoint, config_data)) => {
                    link.dest_endpoint = endpoint;
                    link.conn_config_data = config_data;
                }
                Err(e) => {
                    tracing::error!(
                        "node_registered: failed to get connection details for {} -> {node_id}: {e}",
                        link.source_node_id
                    );
                }
            }
        }
        save_link(
            &self.0.db,
            link,
            LinkStatus::Pending,
            &format!("failed to reset link {} to pending", link.link_id),
        )
        .await;
        link_reconcile_nodes.insert(link.source_node_id.clone());
    }

    /// Try to adopt a static DP connection for an outgoing link from this node.
    async fn adopt_static_connection(
        &self,
        node_id: &str,
        link: &mut crate::db::Link,
        reported: &[ReportedConnection],
    ) {
        let Some(rc) = find_reported_connection(reported, &link.dest_endpoint) else {
            return;
        };
        if link.link_id != rc.link_id || link.conn_config_data != rc.config_data {
            link.link_id = rc.link_id.clone();
            link.conn_config_data = rc.config_data.clone();
            save_link(
                &self.0.db,
                link,
                LinkStatus::Applied,
                "failed to adopt static connection for link",
            )
            .await;
        } else if link.status != LinkStatus::Applied {
            link.status = LinkStatus::Applied;
            link.last_updated = SystemTime::now();
            let _ = self.0.db.update_link(link.clone()).await;
        }
        self.0.queue.add(node_id.to_string());
    }

    /// Restore a soft-deleted link during node registration.
    async fn restore_deleted_link(
        &self,
        node_id: &str,
        link: &mut crate::db::Link,
        link_reconcile_nodes: &mut HashSet<String>,
    ) {
        let (source_id, reconcile_id) = if link.source_node_id == node_id {
            (node_id, node_id.to_string())
        } else if link.dest_node_id == node_id {
            (link.source_node_id.as_str(), link.source_node_id.clone())
        } else {
            return;
        };

        if let Ok((endpoint, config_data)) = self
            .get_client_config(&link.source_node_id, &link.dest_node_id)
            .await
        {
            link.dest_endpoint = endpoint;
            link.conn_config_data = config_data;
        }

        if save_link(
            &self.0.db,
            link,
            LinkStatus::Pending,
            &format!("failed to restore link {} (src={source_id})", link.link_id),
        )
        .await
        {
            link_reconcile_nodes.insert(reconcile_id);
        }
    }

    /// Called when a node gracefully deregisters. Cleans up all DB state owned by
    /// or pointing to `node_id`, and triggers reconciliation on affected peers.
    pub async fn node_deregistered(&self, node_id: &str) {
        // Serialize with node_registered for the same node.
        let node_lock = {
            let mut locks = self.0.node_locks.lock().await;
            locks
                .entry(node_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _node_guard = node_lock.lock().await;

        tracing::info!("route service: cleaning up state for deregistered node {node_id}");

        // Fetch both route sets concurrently.
        let (src_routes, dest_routes) = tokio::join!(
            self.0.db.get_routes_for_node(node_id),
            self.0.db.get_routes_for_dest_node_id(node_id),
        );
        let src_routes = match src_routes {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("node_deregistered: failed to get routes for node {node_id}: {e}");
                vec![]
            }
        };
        let dest_routes = match dest_routes {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("node_deregistered: failed to get dest routes for {node_id}: {e}");
                vec![]
            }
        };

        for route in src_routes {
            if let Err(e) = self.0.db.delete_route(&route.id).await {
                tracing::warn!(
                    "node_deregistered: failed to delete route {}: {e}",
                    route.id
                );
            }
        }

        // For routes where this node is the destination: mark deleted and
        // re-trigger source nodes so the subscription is cleaned up on them.
        for route in &dest_routes {
            if route.source_node_id == ALL_NODES_ID {
                // Wildcard template routes are intentionally preserved: they represent
                // operator intent and will be re-expanded when the node re-registers.
                continue;
            }
            if let Err(e) = self.0.db.mark_route_deleted(&route.id).await {
                tracing::warn!(
                    "node_deregistered: failed to mark route {} deleted: {e}",
                    route.id
                );
            } else {
                self.0.queue.add(route.source_node_id.clone());
            }
        }

        // Links: process all links involving this node.
        let links = self
            .0
            .db
            .get_links_for_node(node_id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("node_deregistered: failed to get links for {node_id}: {e}");
                vec![]
            });
        for mut link in links {
            if link.status == LinkStatus::Deleted {
                continue;
            }
            if link.source_node_id == node_id {
                // Soft-delete outgoing links.  The link reconciler cannot reach
                // this node to tear down the connection (it's gone), but keeping
                // the record allows re-registration to restore the same link_id
                // and avoid invalidating route references.
                link.status = LinkStatus::Deleted;
                link.last_updated = std::time::SystemTime::now();
                if let Err(e) = self.0.db.update_link(link.clone()).await {
                    tracing::warn!(
                        "node_deregistered: failed to mark outgoing link {} deleted: {e}",
                        link.link_id
                    );
                }
            } else {
                // Incoming links from peer nodes: soft-delete to preserve the
                // link_id for re-registration.  We intentionally do NOT enqueue
                // the link reconciler here — doing so would hard-delete the
                // record after the peer ACKs the connection removal, making it
                // impossible to restore the link on re-register.  The stale
                // connection on the peer is harmless (no routes reference it
                // while this node is down) and will be replaced when the link
                // is restored.
                link.status = LinkStatus::Deleted;
                link.last_updated = std::time::SystemTime::now();
                if let Err(e) = self.0.db.update_link(link.clone()).await {
                    tracing::warn!(
                        "node_deregistered: failed to mark link {} deleted: {e}",
                        link.link_id
                    );
                }
            }
        }

        // Remove the node record itself.
        if let Err(e) = self.0.db.delete_node(node_id).await {
            tracing::warn!("node_deregistered: failed to delete node {node_id}: {e}");
        }

        // Rebuild link graph since the set of groups may have changed.
        // If a group disappeared, re-expand all wildcard routes so stale
        // per-gateway routes pointing at this node get cleaned up.
        if let Ok(remaining_nodes) = self.0.db.list_nodes().await {
            let groups_changed = self.rebuild_link_graph(&remaining_nodes).await;
            if groups_changed {
                let all_links = self.0.db.list_all_links().await.unwrap_or_default();
                self.expand_all_wildcard_routes(&remaining_nodes, &all_links)
                    .await;
            }
        }

        // Remove the map entry while _node_guard is still held.  Any
        // concurrent node_registered is either waiting for this guard (and
        // will create a fresh entry after we remove it) or hasn't started
        // yet.  Dropping the guard first would open a window where a racing
        // thread re-inserts an entry that we then remove, leaving it without
        // a lock.
        // Lock-ordering note: the outer node_locks Mutex is always released
        // before a per-node Mutex is awaited, so taking the outer lock here
        // (while holding the inner guard) does not introduce a deadlock.
        self.0.node_locks.lock().await.remove(node_id);
        // _node_guard drops here naturally.
    }
}

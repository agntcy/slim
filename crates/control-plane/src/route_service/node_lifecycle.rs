// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;

use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::NameId;

use crate::api::proto::controller::proto::v1::ConnectionDirection;
use crate::db::{LinkStatus, RouteStatus};
use crate::node_transport::NodeStatus;

use super::connection_config::{ReportedConnection, find_reported_connection};
use super::*;

impl super::RouteService {
    /// Called when a new node registers. Syncs link state against the connections
    /// the data plane reported, then triggers reconciliation as needed.
    pub async fn node_registered(
        &self,
        node_id: &str,
        group_name: &str,
        conn_details_updated: bool,
        dp_connections: Vec<crate::api::proto::controller::proto::v1::ConnectionEntry>,
        dp_routes: Vec<crate::api::proto::controller::proto::v1::Route>,
    ) {
        // Serialize link creation and lifecycle operations across all nodes in the
        // same group. This prevents: (1) concurrent registrations from creating
        // duplicate inter-group links, and (2) a rapid disconnect-reconnect race
        // from leaving stale link records.
        let group_lock = {
            let mut locks = self.0.group_locks.lock().await;
            locks
                .entry(group_name.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _group_guard = group_lock.lock().await;

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
        let mut node_links: Vec<crate::db::Link> = Vec::with_capacity(links.len());
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
            node_links.push(link);
        }

        let all_nodes = match self.0.db.list_nodes().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("node_registered: list_nodes: {e}");
                return;
            }
        };

        self.rebuild_link_graph(&all_nodes).await;
        let allowed_pairs = self.allowed_link_pairs().await;

        let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
            tracing::error!("failed to list all links: {e}");
            vec![]
        });
        let (affected_nodes, new_links) = self
            .ensure_links_for_node(
                node_id,
                &node_links,
                &all_nodes,
                &all_links,
                &reported,
                &allowed_pairs,
            )
            .await;
        for nid in affected_nodes {
            link_reconcile_nodes.insert(nid);
        }
        node_links.extend(new_links);

        // Re-expand wildcard routes via SPT. This is idempotent — duplicates
        // are rejected by add_single_route.
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
                    let Some(dp_name) = dp.name.as_ref() else {
                        return false;
                    };
                    let (c0, c1, c2) = dp_name.str_components();
                    let comp_id = if dp_name.id() == NameId::NULL_COMPONENT {
                        None
                    } else {
                        Some(dp_name.string_id())
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
            match self.get_client_config(&link.source_node_id, node_id).await {
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

        // Handle link failover first — this moves routes to the new gateway
        // before cleanup_routes_for_node would delete them.
        self.handle_links_for_departing_node(node_id).await;

        // Clean up any remaining routes.
        self.cleanup_routes_for_node(node_id).await;

        // Remove the node record itself.
        if let Err(e) = self.0.db.delete_node(node_id).await {
            tracing::warn!("node_deregistered: failed to delete node {node_id}: {e}");
        }

        // Remove the map entry while _node_guard is still held.
        self.0.node_locks.lock().await.remove(node_id);
        // _node_guard drops here naturally.
    }

    /// Called when a node disconnects ungracefully (stream error, crash).
    /// Unlike `node_deregistered`, keeps the node record (expecting reconnection)
    /// and attempts gateway failover for inter-group links.
    pub async fn node_disconnected(&self, node_id: &str) {
        // Serialize with node_registered/node_deregistered for the same node.
        let node_lock = {
            let mut locks = self.0.node_locks.lock().await;
            locks
                .entry(node_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _node_guard = node_lock.lock().await;

        tracing::info!("route service: handling disconnect for node {node_id}");

        // Handle link failover first — this moves routes to the new gateway
        // before cleanup_routes_for_node would delete them.
        self.handle_links_for_departing_node(node_id).await;

        // Clean up any remaining routes (those not moved by link reassignment).
        self.cleanup_routes_for_node(node_id).await;

        // Node record and node_lock are kept — node may reconnect.
    }

    // ── Shared helpers ──────────────────────────────────────────────────────

    /// Delete routes where this node is the source, mark-delete routes where
    /// this node is the destination and re-trigger their source nodes.
    async fn cleanup_routes_for_node(&self, node_id: &str) {
        let (src_routes, dest_routes, wildcard_routes) = tokio::join!(
            self.0.db.get_routes_for_node(node_id),
            self.0.db.get_routes_for_dest_node_id(node_id),
            self.0.db.get_routes_for_node(ALL_NODES_ID),
        );

        let src_routes = src_routes.unwrap_or_else(|e| {
            tracing::error!("cleanup_routes: failed to get routes for {node_id}: {e}");
            vec![]
        });
        let dest_routes = dest_routes.unwrap_or_else(|e| {
            tracing::error!("cleanup_routes: failed to get dest routes for {node_id}: {e}");
            vec![]
        });
        // Wildcard templates where this node is the destination.
        let wildcard_dest: Vec<_> = wildcard_routes
            .unwrap_or_else(|e| {
                tracing::error!("cleanup_routes: failed to get wildcard routes: {e}");
                vec![]
            })
            .into_iter()
            .filter(|r| r.dest_node_id == node_id)
            .collect();

        for route in src_routes {
            if let Err(e) = self.0.db.delete_route(&route.id).await {
                tracing::warn!("cleanup_routes: failed to delete route {}: {e}", route.id);
            }
        }

        // For routes where this node is the destination: mark deleted and
        // re-trigger source nodes so the subscription is cleaned up on them.
        for route in &dest_routes {
            if let Err(e) = self.0.db.mark_route_deleted(&route.id).await {
                tracing::warn!(
                    "cleanup_routes: failed to mark route {} deleted: {e}",
                    route.id
                );
            } else {
                self.0.queue.add(route.source_node_id.clone());
            }
        }

        // Wildcard templates are also deleted — the app will re-subscribe
        // when it reconnects, creating a fresh wildcard.
        for route in &wildcard_dest {
            if let Err(e) = self.0.db.mark_route_deleted(&route.id).await {
                tracing::warn!(
                    "cleanup_routes: failed to mark wildcard route {} deleted: {e}",
                    route.id
                );
            }
        }
    }

    /// Handle inter-group links for a departing node. If other connected nodes
    /// exist in the group, reassigns links to a new gateway. Otherwise
    /// soft-deletes links and rebuilds the link graph.
    async fn handle_links_for_departing_node(&self, node_id: &str) {
        let links: Vec<crate::db::Link> = self
            .0
            .db
            .get_links_for_node(node_id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(
                    "handle_links_for_departing_node: failed to get links for {node_id}: {e}"
                );
                vec![]
            })
            .into_iter()
            .filter(|l| l.status != LinkStatus::Deleted)
            .filter(|l| l.source_node_id == node_id || l.dest_node_id == node_id)
            .collect();

        if links.is_empty() {
            return;
        }

        // Determine the group from the link records (node may already be deleted).
        let group = links
            .iter()
            .map(|l| {
                if l.source_node_id == node_id {
                    l.source_group.as_str()
                } else {
                    l.dest_group.as_str()
                }
            })
            .next()
            .unwrap_or("");

        let all_nodes = self.0.db.list_nodes().await.unwrap_or_else(|e| {
            tracing::error!("failed to list nodes: {e}");
            vec![]
        });
        let other_nodes = self
            .find_connected_nodes_in_group(group, node_id, &all_nodes)
            .await;

        if other_nodes.is_empty() {
            self.handle_last_node_in_group(&links, node_id).await;
        } else {
            self.reassign_gateway_links(node_id, &links, &other_nodes, &all_nodes)
                .await;
        }
    }

    /// Find connected nodes in a group, excluding a specific node.
    async fn find_connected_nodes_in_group(
        &self,
        group: &str,
        exclude_node: &str,
        all_nodes: &[crate::db::Node],
    ) -> Vec<String> {
        let mut result = Vec::new();
        for node in all_nodes {
            if node.id == exclude_node {
                continue;
            }
            if node.group_name.as_deref() == Some(group)
                && self.0.cmd_handler.get_connection_status(&node.id).await == NodeStatus::Connected
            {
                result.push(node.id.clone());
            }
        }
        result
    }

    /// Reassign inter-group links from a departing node to another connected
    /// node in the same group.
    async fn reassign_gateway_links(
        &self,
        departing_node: &str,
        links: &[crate::db::Link],
        available_nodes: &[String],
        all_nodes: &[crate::db::Node],
    ) {
        use rand::seq::IndexedRandom;
        let new_gateway = available_nodes
            .choose(&mut rand::rng())
            .expect("available_nodes must not be empty");
        tracing::info!(
            "reassign_gateway_links: moving links from {departing_node} to {new_gateway}"
        );

        let mut sources_needing_links: Vec<String> = Vec::new();

        for link in links {
            if link.source_node_id == departing_node {
                // Outgoing link: move source to new gateway, reset to Pending
                // so reconciler re-establishes the connection from new node.
                let mut updated = link.clone();
                updated.source_node_id = new_gateway.clone();
                updated.status = LinkStatus::Pending;
                updated.last_updated = SystemTime::now();

                // storage_key includes source/dest fields, so changing them
                // requires delete + re-add rather than an in-place update.
                if let Err(e) = self.0.db.delete_link(link).await {
                    tracing::warn!(
                        "reassign_gateway_links: failed to delete old link {}: {e}",
                        link.link_id
                    );
                    continue;
                }
                if let Err(e) = self.0.db.add_link(updated).await {
                    tracing::warn!(
                        "reassign_gateway_links: failed to re-add link {}: {e}",
                        link.link_id
                    );
                }
                self.0.queue.add(new_gateway.clone());
            } else if link.dest_node_id == departing_node {
                // Incoming link: the dest_endpoint points to the dead node.
                // Delete the link entirely and let ensure_links_for_node on
                // the source side recreate it targeting the new gateway.
                if let Err(e) = self.0.db.delete_link(link).await {
                    tracing::warn!(
                        "reassign_gateway_links: failed to delete incoming link {}: {e}",
                        link.link_id
                    );
                    continue;
                }
                // Also delete any routes referencing this link — they'll be
                // re-expanded once the new link is claimed.
                if let Ok(stale_routes) = self.0.db.get_routes_by_link_id(&link.link_id).await {
                    for route in &stale_routes {
                        if let Err(e) = self.0.db.delete_route(&route.id).await {
                            tracing::warn!(
                                "reassign_gateway_links: failed to delete stale route {}: {e}",
                                route.id
                            );
                        }
                    }
                }
                tracing::info!(
                    "reassign_gateway_links: deleted incoming link {} (source {}), will be recreated targeting {new_gateway}",
                    link.link_id,
                    link.source_node_id
                );
                sources_needing_links.push(link.source_node_id.clone());
            } else {
                continue;
            }
        }

        // Recreate links from source nodes that lost their dest-side gateway.
        if !sources_needing_links.is_empty() {
            // Exclude the departing node so ensure_links_for_node picks
            // the sibling's endpoint instead of the dead node's.
            let filtered_nodes: Vec<_> = all_nodes
                .iter()
                .filter(|n| n.id != departing_node)
                .cloned()
                .collect();
            let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
                tracing::error!("failed to list links for reassignment: {e}");
                vec![]
            });
            self.rebuild_link_graph(&filtered_nodes).await;
            let allowed_pairs = self.allowed_link_pairs().await;
            for src_node_id in &sources_needing_links {
                let (affected, _new_links) = self
                    .ensure_links_for_node(
                        src_node_id,
                        &all_links,
                        &filtered_nodes,
                        &all_links,
                        &[],
                        &allowed_pairs,
                    )
                    .await;
                for nid in affected {
                    self.0.queue.add(nid);
                }
            }
        }

        // Handle routes on the departing node.
        // For each route, check whether its link_id belongs to a preserved outgoing
        // link (can be moved to new gateway) or a deleted incoming link (must be
        // dropped and re-expanded after the new link is claimed).
        let preserved_link_ids: HashSet<String> = links
            .iter()
            .filter(|l| l.source_node_id == departing_node)
            .map(|l| l.link_id.clone())
            .collect();

        let src_routes = self
            .0
            .db
            .get_routes_for_node(departing_node)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("failed to get routes for departing node: {e}");
                vec![]
            });
        for route in &src_routes {
            if route
                .link_id
                .as_ref()
                .is_some_and(|id| preserved_link_ids.contains(id))
            {
                // Outgoing-link case: link_id is preserved, move route to new gateway.
                let mut moved = route.clone();
                moved.source_node_id = new_gateway.clone();
                moved.status = RouteStatus::Pending;
                if let Err(e) = self.0.db.delete_route(&route.id).await {
                    tracing::warn!(
                        "reassign_gateway_links: failed to delete route {}: {e}",
                        route.id
                    );
                    continue;
                }
                match self.0.db.add_route(moved).await {
                    Ok(r) => {
                        tracing::info!(
                            "reassign_gateway_links: moved route to {new_gateway}: {}",
                            r.id
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "reassign_gateway_links: failed to re-add route {}: {e}",
                            route.id
                        );
                    }
                }
            } else {
                // Incoming-link case: old link_id is gone, delete route.
                // It will be re-expanded after the new link is claimed.
                if let Err(e) = self.0.db.delete_route(&route.id).await {
                    tracing::warn!(
                        "reassign_gateway_links: failed to delete route {}: {e}",
                        route.id
                    );
                }
            }
        }
        self.0.queue.add(new_gateway.clone());

        // Re-expand wildcard routes with new gateway (excluding departing node).
        let filtered_nodes: Vec<_> = all_nodes
            .iter()
            .filter(|n| n.id != departing_node)
            .cloned()
            .collect();
        let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
            tracing::error!("failed to list links for route expansion: {e}");
            vec![]
        });
        self.expand_all_wildcard_routes(&filtered_nodes, &all_links)
            .await;
    }

    /// Last node in group — soft-delete all links, rebuild graph, re-expand routes.
    async fn handle_last_node_in_group(&self, links: &[crate::db::Link], node_id: &str) {
        tracing::info!(
            "handle_last_node_in_group: soft-deleting {} links",
            links.len()
        );

        for link in links {
            if link.source_node_id == node_id {
                // The departing node is the source — we can never send it the
                // "delete connection" command, so hard-delete the record now.
                if let Err(e) = self.0.db.delete_link(link).await {
                    tracing::warn!(
                        "handle_last_node_in_group: failed to delete link {}: {e}",
                        link.link_id
                    );
                }
            } else {
                // The departing node is the dest — soft-delete so the reconciler
                // can notify the source to remove its outgoing connection.
                let mut updated = link.clone();
                updated.status = LinkStatus::Deleted;
                updated.last_updated = SystemTime::now();
                if let Err(e) = self.0.db.update_link(updated).await {
                    tracing::warn!(
                        "handle_last_node_in_group: failed to mark link {} deleted: {e}",
                        link.link_id
                    );
                }
            }
        }

        // Rebuild link graph and re-expand wildcard routes.
        let remaining_nodes = self.0.db.list_nodes().await.unwrap_or_else(|e| {
            tracing::error!("failed to list remaining nodes: {e}");
            vec![]
        });
        let groups_changed = self.rebuild_link_graph(&remaining_nodes).await;
        if groups_changed {
            let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
                tracing::error!("failed to list links for route expansion: {e}");
                vec![]
            });
            self.expand_all_wildcard_routes(&remaining_nodes, &all_links)
                .await;
        }
    }
}

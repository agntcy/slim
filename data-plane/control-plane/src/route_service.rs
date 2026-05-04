// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod reconciler;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::json;
use uuid::Uuid;

use crate::config::ReconcilerConfig;
use crate::error::{Error, Result};
use crate::workqueue::WorkQueue;

use crate::api::proto::controller::proto::v1::{
    ConnectionListResponse, ControlMessage, SubscriptionListResponse, control_message::Payload,
};
use crate::db::{LinkStatus, RouteStatus, SharedDb, SubscriptionName};
use crate::node_transport::{DefaultNodeCommandHandler, ResponseKind};

pub const ALL_NODES_ID: &str = crate::db::ALL_NODES_ID;

/// A lightweight route descriptor used by the service layer.
#[derive(Debug, Clone)]
pub struct Route {
    pub source_node_id: String,
    pub dest_node_id: String,
    pub link_id: String,
    pub component0: String,
    pub component1: String,
    pub component2: String,
    pub component_id: Option<u64>,
}

impl From<&Route> for crate::db::Route {
    fn from(route: &Route) -> Self {
        crate::db::Route {
            id: 0,
            source_node_id: route.source_node_id.clone(),
            dest_node_id: route.dest_node_id.clone(),
            link_id: String::new(),
            component0: route.component0.clone(),
            component1: route.component1.clone(),
            component2: route.component2.clone(),
            component_id: route.component_id.map(|v| v as i64),
            status: if route.source_node_id != ALL_NODES_ID {
                RouteStatus::Pending
            } else {
                RouteStatus::Applied
            },
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        }
    }
}

#[derive(Clone, Debug)]
struct ReportedConnection {
    endpoint: String,
    link_id: String,
    config_data: String,
}

struct Inner {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    /// Single work queue for reconciliation (connections + subscriptions).
    queue: WorkQueue<String>,
    /// Signals the periodic sweep task to stop.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Per-node mutex that serializes node_registered and node_deregistered for
    /// the same node.  Without this, a rapid disconnect-reconnect sequence can
    /// race: node_deregistered deletes links while node_registered is recreating
    /// them, leaving stale link records for a disconnected node.
    node_locks: tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Outgoing connections reported by DP nodes during registration.
    /// Used to adopt static connections instead of generating new config.
    reported_connections: parking_lot::RwLock<HashMap<String, Vec<ReportedConnection>>>,
}

#[derive(Clone)]
pub struct RouteService(Arc<Inner>);

impl RouteService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        reconciler_config: ReconcilerConfig,
    ) -> Self {
        let queue: WorkQueue<String> = WorkQueue::new();

        let reconciler = reconciler::Reconciler::new(
            db.clone(),
            cmd_handler.clone(),
            queue.clone(),
            reconciler_config.clone(),
        );
        let workers = reconciler_config.workers.max(1);
        for _ in 0..workers {
            tokio::spawn(reconciler.clone().run());
        }

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let svc = Self(Arc::new(Inner {
            db,
            cmd_handler,
            queue,
            shutdown_tx,
            node_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            reported_connections: parking_lot::RwLock::new(HashMap::new()),
        }));

        // Periodic full-sweep reconciliation with clean shutdown support.
        let reconcile_period = std::time::Duration::from(reconciler_config.reconcile_period);
        if reconcile_period > std::time::Duration::ZERO {
            let period = reconcile_period;
            let svc_clone = svc.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        biased;
                        _ = shutdown_rx.changed() => break,
                        _ = interval.tick() => {}
                    }
                    let nodes = svc_clone.0.db.list_nodes().await;
                    for node in nodes {
                        if svc_clone
                            .0
                            .cmd_handler
                            .get_connection_status(&node.id)
                            .await
                            == crate::node_transport::NodeStatus::Connected
                        {
                            svc_clone.0.queue.add(node.id);
                        }
                    }
                }
                tracing::debug!("route service: periodic sweep task stopped");
            });
        }

        svc
    }

    /// Stop the reconciler workers and wait for any in-flight reconciliations
    /// to finish before returning.
    pub async fn shutdown(&self) {
        tracing::info!("route service: shutting down reconcilers");
        let _ = self.0.shutdown_tx.send(true);
        self.0.queue.shutdown_with_drain().await;
        tracing::info!("route service: reconcilers stopped");
    }

    pub async fn add_route(&self, route: Route) -> Result<String> {
        if route.source_node_id.is_empty() {
            return Err(Error::InvalidInput(
                "source node ID cannot be empty".to_string(),
            ));
        }
        if route.dest_node_id.is_empty() {
            return Err(Error::InvalidInput(
                "destination node ID cannot be empty".to_string(),
            ));
        }
        if route.source_node_id == route.dest_node_id {
            return Err(Error::InvalidInput(
                "destination node ID cannot be the same as source node ID".to_string(),
            ));
        }

        let db_route = crate::db::Route::from(&route);

        let route_id = self.add_single_route(db_route).await?;

        // For wildcard source, create per-node routes for all existing nodes.
        if route.source_node_id == ALL_NODES_ID {
            let all_nodes = self.0.db.list_nodes().await;
            for n in all_nodes {
                if n.id == route.dest_node_id {
                    continue;
                }
                let per_node = crate::db::Route {
                    source_node_id: n.id.clone(),
                    link_id: String::new(),
                    status: RouteStatus::Pending,
                    ..crate::db::Route::from(&route)
                };
                if let Err(e) = self.add_single_route(per_node).await {
                    tracing::debug!("route expansion for node {} skipped: {e}", n.id);
                }
            }
        }

        Ok(route_id)
    }

    async fn add_single_route(&self, mut db_route: crate::db::Route) -> Result<String> {
        // Try to resolve the link_id now; if no link exists yet, store the
        // route with an empty link_id — the reconciler will resolve it later.
        if db_route.source_node_id != ALL_NODES_ID {
            db_route.link_id = self
                .find_matching_link(&db_route.source_node_id, &db_route.dest_node_id)
                .await
                .unwrap_or_default();
        }

        let route_str;
        match self.0.db.add_route(db_route.clone()).await {
            Ok(r) => {
                route_str = r.to_string();
                tracing::info!("route added: {route_str}");
            }
            Err(e) => {
                // If the route already exists and is marked deleted, clean it up and retry.
                let unique_id = db_route.compute_id();
                if let Some(existing) = self.0.db.get_route_by_id(unique_id).await {
                    if existing.deleted {
                        tracing::warn!("removing stale deleted route {} to allow re-add", existing);
                        match self.0.db.delete_route(existing.id).await {
                            Ok(()) => {}
                            Err(Error::RouteNotFound { .. }) => {
                                // Another concurrent task already deleted it — desired state reached.
                                tracing::debug!(
                                    "stale deleted route {} already removed by concurrent task",
                                    existing.id
                                );
                            }
                            Err(e) => return Err(e),
                        }
                        let r = self.0.db.add_route(db_route.clone()).await?;
                        route_str = r.to_string();
                    } else {
                        return Err(Error::InvalidInput(format!("failed to add route: {e}")));
                    }
                } else {
                    return Err(Error::InvalidInput(format!("failed to add route: {e}")));
                }
            }
        }

        if db_route.source_node_id != ALL_NODES_ID {
            self.0.queue.add(db_route.source_node_id);
        }
        Ok(route_str)
    }

    pub async fn delete_route(&self, route: Route) -> Result<()> {
        if route.dest_node_id.is_empty() {
            return Err(Error::InvalidInput("destNodeID must be set".to_string()));
        }

        if route.source_node_id == ALL_NODES_ID {
            // Delete the wildcard route itself.
            let db_route = self
                .0
                .db
                .get_route_for_src_dest_name(
                    &route.source_node_id,
                    &SubscriptionName {
                        component0: &route.component0,
                        component1: &route.component1,
                        component2: &route.component2,
                        component_id: route.component_id.map(|v| v as i64),
                    },
                    &route.dest_node_id,
                    "",
                )
                .await
                .ok_or(Error::InvalidInput("route not found".to_string()))?;
            self.0.db.delete_route(db_route.id).await?;

            // Also delete all per-node expansions.
            let per_node = self
                .0
                .db
                .get_routes_for_dest_node_id_and_name(
                    &route.dest_node_id,
                    &route.component0,
                    &route.component1,
                    &route.component2,
                    route.component_id.map(|v| v as i64),
                )
                .await;
            for r in per_node {
                self.delete_single_route(&r.source_node_id, r.id, &r.to_string())
                    .await?;
            }
            return Ok(());
        }

        let link_id = if route.link_id.is_empty() {
            self.find_matching_link(&route.source_node_id, &route.dest_node_id)
                .await?
        } else {
            route.link_id.clone()
        };

        let db_route = self
            .0
            .db
            .get_route_for_src_dest_name(
                &route.source_node_id,
                &SubscriptionName {
                    component0: &route.component0,
                    component1: &route.component1,
                    component2: &route.component2,
                    component_id: route.component_id.map(|v| v as i64),
                },
                &route.dest_node_id,
                &link_id,
            )
            .await
            .ok_or(Error::InvalidInput("route not found".to_string()))?;

        self.delete_single_route(&route.source_node_id, db_route.id, &db_route.to_string())
            .await
    }

    async fn delete_single_route(
        &self,
        node_id: &str,
        route_id: i64,
        route_key: &str,
    ) -> Result<()> {
        self.0.db.mark_route_deleted(route_id).await?;
        tracing::info!("route marked for delete: {route_key}");
        if node_id != ALL_NODES_ID {
            self.0.queue.add(node_id.to_string());
        }
        Ok(())
    }

    /// Called when a new node registers. Syncs link state against the connections
    /// the data plane reported, then triggers reconciliation as needed.
    pub async fn node_registered(
        &self,
        node_id: &str,
        conn_details_updated: bool,
        dp_connections: Vec<crate::api::proto::controller::proto::v1::ConnectionEntry>,
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
        let active_link_ids: std::collections::HashSet<String> = dp_connections
            .iter()
            .filter_map(|e| e.link_id.as_deref().filter(|id| !id.is_empty()))
            .map(str::to_string)
            .collect();

        // Store reported outgoing connections for static connection adoption.
        {
            use crate::api::proto::controller::proto::v1::ConnectionDirection;
            let mut reported: Vec<ReportedConnection> = Vec::new();
            for entry in &dp_connections {
                if entry.direction != ConnectionDirection::Outgoing as i32 {
                    continue;
                }
                let Some(link_id) = entry.link_id.as_deref().filter(|id| !id.is_empty()) else {
                    continue;
                };
                if let Ok(config) =
                    serde_json::from_str::<serde_json::Value>(&entry.config_data)
                {
                    if let Some(endpoint) = config.get("endpoint").and_then(|v| v.as_str()) {
                        reported.push(ReportedConnection {
                            endpoint: endpoint.to_string(),
                            link_id: link_id.to_string(),
                            config_data: entry.config_data.clone(),
                        });
                    }
                }
            }
            self.0
                .reported_connections
                .write()
                .insert(node_id.to_string(), reported);
        }

        let mut link_reconcile_nodes: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // True only when the DP explicitly sent a non-empty connections list.  An
        // empty list may mean the DP just started and hasn't established any
        // connections yet, or that it doesn't support the connections field.
        let dp_reported_connections = !dp_connections.is_empty();

        for mut link in self.0.db.get_links_for_node(node_id).await {
            if link.deleted || link.dest_node_id != node_id {
                continue;
            }

            let dp_says_alive = !conn_details_updated
                && dp_reported_connections
                && active_link_ids.contains(&link.link_id);
            let dp_says_dead = !conn_details_updated
                && dp_reported_connections
                && !active_link_ids.contains(&link.link_id);

            if dp_says_alive {
                // DP explicitly reports this connection is alive — mark Applied and
                // re-trigger routes only, no link reconciliation needed.
                if link.status != LinkStatus::Applied {
                    link.status = LinkStatus::Applied;
                    link.status_msg = String::new();
                    link.last_updated = SystemTime::now();
                    if let Err(e) = self.0.db.update_link(link.clone()).await {
                        tracing::warn!(
                            "node_registered: failed to mark link {} applied: {e}",
                            link.link_id
                        );
                    }
                }
                self.0.queue.add(link.source_node_id.clone());
            } else if !conn_details_updated && !dp_says_dead && link.status == LinkStatus::Applied {
                // Endpoint unchanged, DP did not explicitly report the link as dead
                // (either doesn't support connections reporting or just started up).
                // Optimistically trust the existing Applied link and only re-verify
                // routes — this avoids triggering a needless connection recreation on
                // the relay DP when the other side has a CP-only reconnect.
                tracing::debug!(
                    "node_registered: link {} still Applied and endpoint unchanged, skipping link reconcile",
                    link.link_id
                );
                self.0.queue.add(link.source_node_id.clone());
            } else {
                // Connection lost, DP says dead, or endpoint changed — update config
                // in-place and reset to Pending for full link reconciliation.
                if conn_details_updated {
                    match self
                        .get_connection_details(&link.source_node_id, node_id)
                        .await
                    {
                        Ok((endpoint, config_data)) => {
                            link.dest_endpoint = endpoint;
                            link.conn_config_data = config_data;
                        }
                        Err(e) => {
                            tracing::error!(
                                "node_registered: failed to get updated connection details \
                                 for {} -> {node_id}: {e}",
                                link.source_node_id
                            );
                        }
                    }
                }
                if link.status != LinkStatus::Pending || conn_details_updated {
                    link.status = LinkStatus::Pending;
                    link.status_msg = String::new();
                    link.last_updated = SystemTime::now();
                    if let Err(e) = self.0.db.update_link(link.clone()).await {
                        tracing::warn!(
                            "node_registered: failed to reset link {} to pending: {e}",
                            link.link_id
                        );
                    }
                }
                link_reconcile_nodes.insert(link.source_node_id.clone());
            }
        }

        // Adopt static connections for existing outgoing links from this node.
        for mut link in self.0.db.get_links_for_node(node_id).await {
            if link.deleted || link.source_node_id != node_id {
                continue;
            }
            if let Some(rc) = self.find_reported_connection(node_id, &link.dest_endpoint) {
                if link.link_id != rc.link_id || link.conn_config_data != rc.config_data {
                    link.link_id = rc.link_id.clone();
                    link.conn_config_data = rc.config_data.clone();
                    link.status = LinkStatus::Applied;
                    link.status_msg = String::new();
                    link.last_updated = SystemTime::now();
                    if let Err(e) = self.0.db.update_link(link.clone()).await {
                        tracing::warn!(
                            "node_registered: failed to adopt static connection for link: {e}"
                        );
                    }
                } else if link.status != LinkStatus::Applied {
                    link.status = LinkStatus::Applied;
                    link.last_updated = SystemTime::now();
                    let _ = self.0.db.update_link(link.clone()).await;
                }
                self.0.queue.add(node_id.to_string());
            }
        }

        // Restore soft-deleted links involving this node.
        // These were soft-deleted during deregister; restoring them preserves
        // the link_id so that routes referencing it remain valid.
        for mut link in self.0.db.get_links_for_node(node_id).await {
            if !link.deleted {
                continue;
            }
            if link.source_node_id == node_id {
                // Outgoing link: update endpoint if destination address changed.
                if let Some(dest_node) = self.0.db.get_node(&link.dest_node_id).await {
                    if let Ok((endpoint, config_data)) =
                        self.get_connection_details(node_id, &dest_node.id).await
                    {
                        link.dest_endpoint = endpoint;
                        link.conn_config_data = config_data;
                    }
                }
                link.deleted = false;
                link.status = LinkStatus::Pending;
                link.status_msg = String::new();
                link.last_updated = SystemTime::now();
                if let Err(e) = self.0.db.update_link(link.clone()).await {
                    tracing::warn!(
                        "node_registered: failed to restore outgoing link {}: {e}",
                        link.link_id
                    );
                } else {
                    link_reconcile_nodes.insert(node_id.to_string());
                }
            } else if link.dest_node_id == node_id {
                // Incoming link from a peer: update endpoint (this node's
                // address may have changed) and enqueue the peer for link
                // reconciliation to re-establish the connection.
                if let Ok((endpoint, config_data)) = self
                    .get_connection_details(&link.source_node_id, node_id)
                    .await
                {
                    link.dest_endpoint = endpoint;
                    link.conn_config_data = config_data;
                }
                link.deleted = false;
                link.status = LinkStatus::Pending;
                link.status_msg = String::new();
                link.last_updated = SystemTime::now();
                if let Err(e) = self.0.db.update_link(link.clone()).await {
                    tracing::warn!(
                        "node_registered: failed to restore incoming link {}: {e}",
                        link.link_id
                    );
                } else {
                    link_reconcile_nodes.insert(link.source_node_id.clone());
                }
            }
        }

        let all_nodes = self.0.db.list_nodes().await;

        let mut node_links = self.0.db.get_links_for_node(node_id).await;
        let (affected_nodes, new_links) = self
            .ensure_links_for_node(node_id, &node_links, &all_nodes)
            .await;
        for nid in affected_nodes {
            link_reconcile_nodes.insert(nid);
        }
        node_links.extend(new_links);

        // Restore soft-deleted routes where this node is the destination.
        // These routes were marked deleted when the node deregistered; now that
        // it's back, restore them.  The link_id is preserved (same link was
        // soft-deleted and restored above).
        for route in self.0.db.get_routes_for_dest_node_id(node_id).await {
            if !route.deleted {
                continue;
            }
            // Use the route's existing link_id — it should match the restored link.
            // If the link doesn't exist (e.g. source node also deregistered), look
            // up the current link between the pair.
            let link_id = node_links
                .iter()
                .find(|l| {
                    !l.deleted
                        && ((l.source_node_id == route.source_node_id && l.dest_node_id == node_id)
                            || (l.source_node_id == node_id
                                && l.dest_node_id == route.source_node_id))
                })
                .map(|l| l.link_id.clone())
                .unwrap_or_else(|| route.link_id.clone());
            if let Err(e) = self.0.db.restore_route(route.id, &link_id).await {
                tracing::warn!("node_registered: failed to restore route {}: {e}", route.id);
            } else {
                self.0.queue.add(route.source_node_id.clone());
            }
        }

        self.ensure_routes_for_node(node_id, &node_links, &all_nodes)
            .await;
        // Always enqueue the reconnecting node for route reconciliation so that
        // any pending deletes (deleted=true routes) and pending applies are
        // pushed to the data plane immediately, rather than waiting for the link
        // reconciler to trigger it later.
        self.0.queue.add(node_id.to_string());
        link_reconcile_nodes.insert(node_id.to_string());

        for nid in link_reconcile_nodes {
            self.0.queue.add(nid);
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

        self.0.reported_connections.write().remove(node_id);

        // Fetch both route sets concurrently.
        let (src_routes, dest_routes) = tokio::join!(
            self.0.db.get_routes_for_node_id(node_id),
            self.0.db.get_routes_for_dest_node_id(node_id),
        );

        // Hard-delete all routes where this node is the source.  This includes
        // both wildcard expansions (auto-created) and operator-added concrete
        // routes.  Operator-added routes are permanently lost and must be
        // re-added after the node re-registers; wildcard *template* routes
        // (source=ALL_NODES_ID) are preserved below and re-expanded on
        // re-registration.
        for route in src_routes {
            if let Err(e) = self.0.db.delete_route(route.id).await {
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
            if let Err(e) = self.0.db.mark_route_deleted(route.id).await {
                tracing::warn!(
                    "node_deregistered: failed to mark route {} deleted: {e}",
                    route.id
                );
            } else {
                self.0.queue.add(route.source_node_id.clone());
            }
        }

        // Links: process all links involving this node.
        for mut link in self.0.db.get_links_for_node(node_id).await {
            if link.deleted {
                continue;
            }
            if link.source_node_id == node_id {
                // Soft-delete outgoing links.  The link reconciler cannot reach
                // this node to tear down the connection (it's gone), but keeping
                // the record allows re-registration to restore the same link_id
                // and avoid invalidating route references.
                link.deleted = true;
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
                link.deleted = true;
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

    /// Ensure direct or group links exist between `node_id` and every other node.
    /// Returns (affected_node_ids, newly_created_links).
    async fn ensure_links_for_node(
        &self,
        node_id: &str,
        existing_links: &[crate::db::Link],
        all_nodes: &[crate::db::Node],
    ) -> (Vec<String>, Vec<crate::db::Link>) {
        let src_node = match all_nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n.clone(),
            None => {
                tracing::error!("ensure_links: node {node_id} not found");
                return (vec![node_id.to_string()], vec![]);
            }
        };
        let mut affected: std::collections::HashSet<String> =
            [node_id.to_string()].into_iter().collect();
        let mut new_links: Vec<crate::db::Link> = Vec::new();

        let connected_peers: std::collections::HashSet<String> = existing_links
            .iter()
            .filter(|l| !l.deleted)
            .map(|l| {
                if l.source_node_id == node_id {
                    l.dest_node_id.clone()
                } else {
                    l.source_node_id.clone()
                }
            })
            .collect();

        let has_src_external = src_node.conn_details.iter().any(|d| {
            d.external_endpoint
                .as_deref()
                .map(|e| !e.is_empty())
                .unwrap_or(false)
        });

        for other in all_nodes {
            if other.id == node_id {
                continue;
            }
            if connected_peers.contains(&other.id) {
                continue;
            }
            let same_group = src_node.group_name == other.group_name;
            if same_group {
                if let Some((src, link)) = self.ensure_direct_link(&src_node, other).await {
                    affected.insert(src);
                    new_links.extend(link);
                }
                continue;
            }
            let has_dst_external = other.conn_details.iter().any(|d| {
                d.external_endpoint
                    .as_deref()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
            });
            if has_dst_external {
                if let Some((src, link)) = self.ensure_group_link(&src_node, other).await {
                    affected.insert(src);
                    new_links.extend(link);
                }
                continue;
            }
            if has_src_external {
                if let Some((src, link)) = self.ensure_group_link(other, &src_node).await {
                    affected.insert(src);
                    new_links.extend(link);
                }
                continue;
            }
            tracing::error!(
                "cannot create link between {node_id} and {}: no external endpoint available",
                other.id
            );
        }
        (affected.into_iter().collect(), new_links)
    }

    async fn ensure_direct_link(
        &self,
        src_node: &crate::db::Node,
        dst_node: &crate::db::Node,
    ) -> Option<(String, Option<crate::db::Link>)> {
        let (endpoint, config_data, link_id, status) =
            if let Some(rc) = self.find_reported_connection_for_dest(&src_node.id, dst_node) {
                (
                    rc.endpoint.clone(),
                    rc.config_data.clone(),
                    rc.link_id.clone(),
                    LinkStatus::Applied,
                )
            } else {
                match compute_connection_details(src_node, dst_node) {
                    Ok((ep, cd)) => (ep, cd, Uuid::new_v4().to_string(), LinkStatus::Pending),
                    Err(e) => {
                        tracing::error!(
                            "ensure_direct_link: failed to get connection details: {e}"
                        );
                        return None;
                    }
                }
            };

        let link = crate::db::Link {
            link_id,
            source_node_id: src_node.id.clone(),
            dest_node_id: dst_node.id.clone(),
            dest_endpoint: endpoint,
            conn_config_data: config_data,
            status,
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        };

        match self.0.db.find_or_create_link(link).await {
            Ok((link, created)) => {
                if created {
                    Some((src_node.id.clone(), Some(link)))
                } else {
                    Some((src_node.id.clone(), None))
                }
            }
            Err(e) => {
                tracing::error!(
                    "ensure_direct_link: failed to ensure link {}->{}:  {e}",
                    src_node.id,
                    dst_node.id
                );
                None
            }
        }
    }

    async fn ensure_group_link(
        &self,
        src_node: &crate::db::Node,
        dst_node: &crate::db::Node,
    ) -> Option<(String, Option<crate::db::Link>)> {
        let (endpoint, config_data, link_id, status) =
            if let Some(rc) = self.find_reported_connection_for_dest(&src_node.id, dst_node) {
                (
                    rc.endpoint.clone(),
                    rc.config_data.clone(),
                    rc.link_id.clone(),
                    LinkStatus::Applied,
                )
            } else {
                let (ep, cd) = match compute_connection_details(src_node, dst_node) {
                    Ok(cd) => cd,
                    Err(e) => {
                        tracing::error!(
                            "ensure_group_link: failed to get connection details: {e}"
                        );
                        return None;
                    }
                };
                // Reuse an existing link_id only when its destination matches dst_node.
                let lid = self
                    .0
                    .db
                    .get_link_for_source_and_endpoint(&src_node.id, &ep)
                    .await
                    .filter(|l| l.dest_node_id == dst_node.id)
                    .map(|l| l.link_id)
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                (ep, cd, lid, LinkStatus::Pending)
            };

        let link = crate::db::Link {
            link_id,
            source_node_id: src_node.id.clone(),
            dest_node_id: dst_node.id.clone(),
            dest_endpoint: endpoint,
            conn_config_data: config_data,
            status,
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        };

        match self.0.db.find_or_create_link(link).await {
            Ok((link, created)) => {
                if created {
                    Some((src_node.id.clone(), Some(link)))
                } else {
                    Some((src_node.id.clone(), None))
                }
            }
            Err(e) => {
                tracing::error!(
                    "ensure_group_link: failed to ensure link {}->{}:  {e}",
                    src_node.id,
                    dst_node.id
                );
                None
            }
        }
    }

    /// For each wildcard route, create a per-node route for `node_id` if one
    /// does not exist yet.
    ///
    /// Two passes are made:
    /// 1. `node_id` as SOURCE — expand each wildcard `(*, dest_X)` into a
    ///    concrete route `node_id → dest_X`.
    /// 2. `node_id` as DEST — for each wildcard `(*, dest=node_id)`, expand
    ///    into a route `peer → node_id` for every currently-registered peer.
    ///    This handles the case where `node_id` registers *after* the wildcard
    ///    was added and after some peer nodes were already registered.
    async fn ensure_routes_for_node(
        &self,
        node_id: &str,
        node_links: &[crate::db::Link],
        all_nodes: &[crate::db::Node],
    ) {
        // Map peer_node_id → link_id covering BOTH outgoing (source=node_id)
        // and incoming (dest=node_id) links.  With the one-link-per-pair design,
        // a node can subscribe via an incoming connection, so both directions must
        // be included here.
        let link_by_peer: std::collections::HashMap<&str, &str> = node_links
            .iter()
            .filter(|l| !l.deleted)
            .map(|l| {
                let peer = if l.source_node_id == node_id {
                    l.dest_node_id.as_str()
                } else {
                    l.source_node_id.as_str()
                };
                (peer, l.link_id.as_str())
            })
            .collect();

        let wildcard_routes = self.0.db.get_routes_for_node_id(ALL_NODES_ID).await;

        // Pre-fetch all existing routes where node_id is source or destination
        // to avoid O(W×N) individual DB lookups in the loops below.
        let (routes_as_src, routes_as_dest) = tokio::join!(
            self.0.db.get_routes_for_node_id(node_id),
            self.0.db.get_routes_for_dest_node_id(node_id),
        );

        type RouteKey = (String, String, String, String, String, Option<i64>, String);
        let existing_routes: std::collections::HashSet<RouteKey> = routes_as_src
            .iter()
            .chain(routes_as_dest.iter())
            .filter(|r| !r.deleted)
            .map(|r| {
                (
                    r.source_node_id.clone(),
                    r.dest_node_id.clone(),
                    r.component0.clone(),
                    r.component1.clone(),
                    r.component2.clone(),
                    r.component_id,
                    r.link_id.clone(),
                )
            })
            .collect();

        // Pass 1: node_id as source.
        for r in &wildcard_routes {
            if r.dest_node_id == node_id {
                continue;
            }
            let link_id = match link_by_peer.get(r.dest_node_id.as_str()) {
                Some(id) => id.to_string(),
                None => {
                    tracing::debug!(
                        "ensure_routes: no link found for {node_id}->{}, skipping",
                        r.dest_node_id
                    );
                    continue;
                }
            };
            let key: RouteKey = (
                node_id.to_string(),
                r.dest_node_id.clone(),
                r.component0.clone(),
                r.component1.clone(),
                r.component2.clone(),
                r.component_id,
                link_id.clone(),
            );
            if existing_routes.contains(&key) {
                continue;
            }
            let new_route = crate::db::Route {
                id: 0,
                source_node_id: node_id.to_string(),
                dest_node_id: r.dest_node_id.clone(),
                link_id,
                component0: r.component0.clone(),
                component1: r.component1.clone(),
                component2: r.component2.clone(),
                component_id: r.component_id,
                status: RouteStatus::Pending,
                status_msg: String::new(),
                deleted: false,
                last_updated: SystemTime::now(),
            };
            match self.0.db.add_route(new_route).await {
                Ok(added) => tracing::info!("generic route created: {added}"),
                Err(e) => tracing::debug!("generic route already exists or cannot be added: {e}"),
            }
        }

        // Pass 2: node_id as destination — expand wildcards targeting node_id for
        // every peer that doesn't already have a route pointing here.
        let wildcard_for_self: Vec<&crate::db::Route> = wildcard_routes
            .iter()
            .filter(|r| r.dest_node_id == node_id)
            .collect();

        if wildcard_for_self.is_empty() {
            return;
        }

        for r in wildcard_for_self {
            for other in all_nodes {
                if other.id == node_id {
                    continue;
                }
                let link_id = match link_by_peer.get(other.id.as_str()) {
                    Some(id) => id.to_string(),
                    None => {
                        tracing::debug!(
                            "ensure_routes: no link found for {}→{node_id}, skipping",
                            other.id
                        );
                        continue;
                    }
                };
                let key: RouteKey = (
                    other.id.clone(),
                    node_id.to_string(),
                    r.component0.clone(),
                    r.component1.clone(),
                    r.component2.clone(),
                    r.component_id,
                    link_id.clone(),
                );
                if existing_routes.contains(&key) {
                    continue;
                }
                let new_route = crate::db::Route {
                    id: 0,
                    source_node_id: other.id.clone(),
                    dest_node_id: node_id.to_string(),
                    link_id,
                    component0: r.component0.clone(),
                    component1: r.component1.clone(),
                    component2: r.component2.clone(),
                    component_id: r.component_id,
                    status: RouteStatus::Pending,
                    status_msg: String::new(),
                    deleted: false,
                    last_updated: SystemTime::now(),
                };
                match self.0.db.add_route(new_route).await {
                    Ok(added) => {
                        tracing::info!("generic route created: {added}");
                        self.0.queue.add(other.id.clone());
                    }
                    Err(e) => {
                        tracing::debug!("generic route already exists or cannot be added: {e}");
                    }
                }
            }
        }
    }

    /// Remove the per-node lock entry to prevent `node_locks` from growing
    /// without bound for crash-disconnected nodes.  For graceful deregisters
    /// `node_deregistered` already removes the entry; this covers the crash path.
    pub async fn remove_node_lock(&self, node_id: &str) {
        self.0.node_locks.lock().await.remove(node_id);
    }

    async fn find_matching_link(&self, source: &str, dest: &str) -> Result<String> {
        match self.0.db.find_link_between_nodes(source, dest).await {
            Some(l) if !l.deleted => Ok(l.link_id),
            _ => Err(Error::InvalidInput(format!(
                "no matching link found for source={source} destination={dest}"
            ))),
        }
    }

    pub async fn list_subscriptions(&self, node_id: &str) -> Result<SubscriptionListResponse> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::SubscriptionListRequest(
                crate::api::proto::controller::proto::v1::SubscriptionListRequest {},
            )),
        };
        let chunks = self
            .0
            .cmd_handler
            .send_and_wait_chunked(node_id, msg, ResponseKind::SubscriptionListResponse)
            .await?;
        let mut entries = Vec::new();
        for chunk in chunks {
            if let Some(Payload::SubscriptionListResponse(r)) = chunk.payload {
                entries.extend(r.entries);
            }
        }
        Ok(SubscriptionListResponse {
            original_message_id: message_id,
            entries,
            done: true,
        })
    }

    pub async fn list_connections(&self, node_id: &str) -> Result<ConnectionListResponse> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::ConnectionListRequest(
                crate::api::proto::controller::proto::v1::ConnectionListRequest {},
            )),
        };
        let chunks = self
            .0
            .cmd_handler
            .send_and_wait_chunked(node_id, msg, ResponseKind::ConnectionListResponse)
            .await?;
        let mut entries = Vec::new();
        for chunk in chunks {
            if let Some(Payload::ConnectionListResponse(r)) = chunk.payload {
                entries.extend(r.entries);
            }
        }
        Ok(ConnectionListResponse {
            original_message_id: message_id,
            entries,
            done: true,
        })
    }

    /// Compute the effective endpoint and serialised JSON config data for a
    /// link from `source_node_id` to `dest_node_id`.
    pub async fn get_connection_details(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<(String, String)> {
        let dest_node =
            self.0
                .db
                .get_node(dest_node_id)
                .await
                .ok_or_else(|| Error::NodeNotFound {
                    id: dest_node_id.to_string(),
                })?;
        if dest_node.conn_details.is_empty() {
            return Err(Error::InvalidInput(format!(
                "no connections for destination node {dest_node_id}"
            )));
        }
        let src_node =
            self.0
                .db
                .get_node(source_node_id)
                .await
                .ok_or_else(|| Error::NodeNotFound {
                    id: source_node_id.to_string(),
                })?;

        compute_connection_details(&src_node, &dest_node)
    }

    fn find_reported_connection(
        &self,
        node_id: &str,
        dest_endpoint: &str,
    ) -> Option<ReportedConnection> {
        let conns = self.0.reported_connections.read();
        let node_conns = conns.get(node_id)?;
        node_conns
            .iter()
            .find(|rc| endpoint_matches(&rc.endpoint, dest_endpoint))
            .cloned()
    }

    fn find_reported_connection_for_dest(
        &self,
        source_node_id: &str,
        dst_node: &crate::db::Node,
    ) -> Option<ReportedConnection> {
        let conns = self.0.reported_connections.read();
        let node_conns = conns.get(source_node_id)?;
        for rc in node_conns {
            for detail in &dst_node.conn_details {
                if endpoint_matches(&rc.endpoint, &detail.endpoint) {
                    return Some(rc.clone());
                }
                if let Some(ext) = &detail.external_endpoint {
                    if !ext.is_empty() && endpoint_matches(&rc.endpoint, ext) {
                        return Some(rc.clone());
                    }
                }
            }
        }
        None
    }
}

/// Compute the effective endpoint and serialised JSON config data from
/// already-loaded node objects, without hitting the DB.
fn compute_connection_details(
    src_node: &crate::db::Node,
    dst_node: &crate::db::Node,
) -> Result<(String, String)> {
    if dst_node.conn_details.is_empty() {
        return Err(Error::InvalidInput(format!(
            "no connections for destination node {}",
            dst_node.id
        )));
    }
    let (conn, local_connection) = select_connection(dst_node, src_node);
    generate_config_data(conn, local_connection, dst_node, src_node)
}

/// Select the best connection detail from `dst_node` relative to `src_node`.
///
/// # Precondition
/// `dst_node.conn_details` must be non-empty.  The caller
/// (`compute_connection_details`) is responsible for enforcing this by
/// returning an error when `conn_details` is empty before calling here.
fn select_connection<'a>(
    dst_node: &'a crate::db::Node,
    src_node: &crate::db::Node,
) -> (&'a crate::db::ConnectionDetails, bool) {
    debug_assert!(
        !dst_node.conn_details.is_empty(),
        "select_connection called with empty conn_details for node {}",
        dst_node.id
    );
    let same_group = dst_node.group_name == src_node.group_name;
    if same_group {
        return (&dst_node.conn_details[0], true);
    }
    for conn in &dst_node.conn_details {
        if conn
            .external_endpoint
            .as_deref()
            .map(|e| !e.is_empty())
            .unwrap_or(false)
        {
            return (conn, false);
        }
    }
    (&dst_node.conn_details[0], false)
}

/// Generate the serialised JSON connection config data for a link.
fn generate_config_data(
    detail: &crate::db::ConnectionDetails,
    local_connection: bool,
    dest_node: &crate::db::Node,
    src_node: &crate::db::Node,
) -> Result<(String, String)> {
    // Start with client_config as a mutable JSON object.
    let mut config: serde_json::Map<String, serde_json::Value> = match &detail.client_config {
        serde_json::Value::Object(m) => m.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            return Err(Error::InvalidInput(format!(
                "unexpected client_config type: expected object, got {}",
                other
            )));
        }
    };

    // Set a default backoff if not present.
    if !config.contains_key("backoff") {
        config.insert(
            "backoff".to_string(),
            json!({
                "type": "fixed_interval",
                "interval": "2000ms"
            }),
        );
    }

    // Determine effective endpoint.
    let endpoint = if local_connection {
        detail.endpoint.clone()
    } else {
        detail
            .external_endpoint
            .clone()
            .filter(|e| !e.is_empty())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no external endpoint for connection {}",
                    detail.endpoint
                ))
            })?
    };

    // Set TLS / endpoint scheme.
    if config.contains_key("tls") {
        config.insert("endpoint".to_string(), json!(format!("https://{endpoint}")));
    } else if !detail.mtls_required {
        config.insert("endpoint".to_string(), json!(format!("http://{endpoint}")));
        config.insert("tls".to_string(), json!({ "insecure": true }));
    } else {
        // MTLS via SPIRE.
        let spire_socket = src_node
            .conn_details
            .iter()
            .find_map(|cd| {
                cd.client_config
                    .get("tls")
                    .and_then(|t| t.get("source"))
                    .and_then(|s| s.get("socket_path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no SPIRE socket path found for source node {}",
                    src_node.id
                ))
            })?;

        config.insert("endpoint".to_string(), json!(format!("https://{endpoint}")));

        let trust_domain = detail
            .trust_domain
            .as_deref()
            .or(dest_node.group_name.as_deref());

        let mut ca_source = json!({
            "type": "spire",
            "socket_path": spire_socket
        });
        if let Some(td) = trust_domain {
            ca_source["trust_domains"] = json!([td]);
        }
        config.insert(
            "tls".to_string(),
            json!({
                "insecure": false,
                "insecure_skip_verify": local_connection,
                "source": {
                    "type": "spire",
                    "socket_path": spire_socket
                },
                "ca_source": ca_source
            }),
        );
    }

    if !config.contains_key("headers") {
        config.insert("headers".to_string(), json!({}));
    }
    if !config.contains_key("keepalive") {
        config.insert(
            "keepalive".to_string(),
            json!({
                "http2_keepalive": "20s",
                "keep_alive_while_idle": false,
                "tcp_keepalive": "20s",
                "timeout": "20s"
            }),
        );
    }

    let config_data = serde_json::to_string(&config)
        .map_err(|e| Error::InvalidInput(format!("failed to encode connection config: {e}")))?;

    let effective_endpoint = config
        .get("endpoint")
        .and_then(|v| v.as_str())
        .unwrap_or(&endpoint)
        .to_string();

    Ok((effective_endpoint, config_data))
}

pub(crate) fn is_connection_not_found(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("connection not found")
        || lower.contains("no such connection")
        || (lower.contains("connection") && lower.contains("not found"))
}

fn endpoint_matches(reported: &str, node_endpoint: &str) -> bool {
    let normalize = |ep: &str| ep.trim().trim_end_matches('/').to_lowercase();
    let r = normalize(reported);
    let n = normalize(node_endpoint);
    if r == n {
        return true;
    }
    if r == format!("https://{n}") || r == format!("http://{n}") {
        return true;
    }
    if n == format!("https://{r}") || n == format!("http://{r}") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ConnectionDetails;
    use crate::db::inmemory::InMemoryDb;
    use crate::node_transport::DefaultNodeCommandHandler;
    use std::time::SystemTime;

    fn make_conn_details(ep: &str, external: Option<&str>, mtls: bool) -> ConnectionDetails {
        ConnectionDetails {
            endpoint: ep.to_string(),
            external_endpoint: external.map(|s| s.to_string()),
            trust_domain: None,
            mtls_required: mtls,
            client_config: serde_json::Value::Null,
        }
    }

    fn make_node(
        id: &str,
        group: Option<&str>,
        details: Vec<ConnectionDetails>,
    ) -> crate::db::Node {
        crate::db::Node {
            id: id.to_string(),
            group_name: group.map(|s| s.to_string()),
            conn_details: details,
            last_updated: SystemTime::now(),
        }
    }

    fn make_route_service(db: crate::db::SharedDb) -> RouteService {
        let handler = DefaultNodeCommandHandler::new();
        RouteService::new(
            db,
            handler,
            ReconcilerConfig {
                max_requeues: 3,
                ..Default::default()
            },
        )
    }

    // ── select_connection ──────────────────────────────────────────────────

    #[test]
    fn select_connection_same_group_returns_first() {
        let dst = make_node(
            "dst",
            Some("grp"),
            vec![make_conn_details("dst:8080", Some("ext:9090"), false)],
        );
        let src = make_node("src", Some("grp"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    #[test]
    fn select_connection_different_group_prefers_external() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![
                make_conn_details("dst:8080", None, false),
                make_conn_details("dst:8081", Some("ext:9090"), false),
            ],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.external_endpoint.as_deref(), Some("ext:9090"));
    }

    #[test]
    fn select_connection_different_group_no_external_falls_back() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![make_conn_details("dst:8080", None, false)],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    // ── generate_config_data ───────────────────────────────────────────────

    #[test]
    fn generate_config_data_local_http() {
        let cd = make_conn_details("host:8080", None, false);
        let dest = make_node("dst", Some("g"), vec![cd.clone()]);
        let src = make_node("src", Some("g"), vec![]);
        let (ep, data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        assert!(ep.starts_with("http://"));
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["tls"]["insecure"], true);
        assert!(v["backoff"].is_object());
        assert!(v["keepalive"].is_object());
    }

    #[test]
    fn generate_config_data_external_no_mtls() {
        let cd = make_conn_details("host:8080", Some("ext:9090"), false);
        let dest = make_node("dst", Some("g1"), vec![cd.clone()]);
        let src = make_node("src", Some("g2"), vec![]);
        let (ep, data) = generate_config_data(&cd, false, &dest, &src).unwrap();
        assert!(ep.contains("ext:9090"));
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["tls"]["insecure"], true);
    }

    #[test]
    fn generate_config_data_no_external_endpoint_remote_returns_error() {
        let cd = make_conn_details("host:8080", None, false);
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        // local_connection=false + no external endpoint → error.
        assert!(generate_config_data(&cd, false, &dest, &src).is_err());
    }

    #[test]
    fn generate_config_data_non_object_client_config_returns_error() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!([1, 2, 3]);
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        assert!(generate_config_data(&cd, true, &dest, &src).is_err());
    }

    #[test]
    fn generate_config_data_preserves_existing_tls() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!({ "tls": { "insecure": false } });
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        let (ep, _data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        assert!(ep.starts_with("https://"));
    }

    #[test]
    fn generate_config_data_preserves_existing_backoff() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!({ "backoff": { "type": "exponential" } });
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        let (_ep, data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["backoff"]["type"], "exponential");
    }

    // ── add_route validation ───────────────────────────────────────────────

    #[tokio::test]
    async fn add_route_empty_source_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: String::new(),
                dest_node_id: "dst".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("source"));
    }

    #[tokio::test]
    async fn add_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: String::new(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("destination"));
    }

    #[tokio::test]
    async fn add_route_same_src_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: "node1".to_string(),
                dest_node_id: "node1".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("same"));
    }

    // ── delete_route validation ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .delete_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: String::new(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("destNodeID"));
    }

    #[tokio::test]
    async fn delete_route_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .delete_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: "dst".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
    }

    // ── get_connection_details ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_connection_details_dest_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("src", "ghost_dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost_dst"));
    }

    #[tokio::test]
    async fn get_connection_details_src_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![make_conn_details("dst:8080", None, false)],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("ghost_src", "dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost_src"));
    }

    #[tokio::test]
    async fn get_connection_details_no_conn_details_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: None,
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("src", "dst").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_connection_details_same_group_local() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![make_conn_details("dst:8080", None, false)],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let (ep, _) = svc.get_connection_details("src", "dst").await.unwrap();
        assert!(ep.starts_with("http://dst:8080"));
    }

    // ── is_connection_not_found ───────────────────────────────────────────

    #[test]
    fn connection_not_found_exact() {
        assert!(is_connection_not_found("connection not found"));
        assert!(is_connection_not_found("Connection Not Found"));
    }

    #[test]
    fn connection_not_found_no_such() {
        assert!(is_connection_not_found("no such connection"));
        assert!(is_connection_not_found("No Such Connection"));
    }

    #[test]
    fn connection_not_found_split_words() {
        assert!(is_connection_not_found("the connection was not found"));
    }

    #[test]
    fn connection_not_found_rejects_unrelated() {
        assert!(!is_connection_not_found("subscription not found"));
        assert!(!is_connection_not_found("route not found"));
        assert!(!is_connection_not_found("node not found"));
        assert!(!is_connection_not_found("not found"));
    }

    // ── endpoint_matches ──────────────────────────────────────────────────

    #[test]
    fn endpoint_matches_exact() {
        assert!(endpoint_matches("https://host:8080", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_scheme_added() {
        assert!(endpoint_matches("https://host:8080", "host:8080"));
        assert!(endpoint_matches("http://host:8080", "host:8080"));
    }

    #[test]
    fn endpoint_matches_scheme_on_node_side() {
        assert!(endpoint_matches("host:8080", "https://host:8080"));
        assert!(endpoint_matches("host:8080", "http://host:8080"));
    }

    #[test]
    fn endpoint_matches_trailing_slash() {
        assert!(endpoint_matches("https://host:8080/", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_case_insensitive() {
        assert!(endpoint_matches("HTTPS://Host:8080", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_no_false_positive() {
        assert!(!endpoint_matches("https://host:8080", "https://other:8080"));
        assert!(!endpoint_matches("host:9090", "host:8080"));
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod reconciler;
pub mod spt;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use petgraph::graph::UnGraph;
use petgraph::visit::EdgeRef;
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::NameId;
use uuid::Uuid;

use crate::config::{ReconcilerConfig, TopologyConfig};
use crate::error::{Error, Result};
use crate::workqueue::WorkQueue;

use crate::api::proto::controller::proto::v1::{
    ConnectionDirection, ConnectionListResponse, ControlMessage, Route as ProtoRoute,
    RouteListResponse, control_message::Payload,
};
use crate::db::{LinkStatus, Route, RouteName, RouteStatus, SharedDb};
use crate::node_transport::{DefaultNodeCommandHandler, ResponseKind};

pub use crate::types::ALL_NODES_ID;
use crate::types::validate_route_nodes;

#[derive(Clone, Debug)]
struct ReportedConnection {
    endpoint: String,
    link_id: String,
    config_data: ClientConfig,
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
    node_locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Topology configuration for link and route filtering.
    topology: TopologyConfig,
    /// Runtime link graph at the group level. Rebuilt when nodes join/leave.
    link_graph: tokio::sync::RwLock<UnGraph<String, u32>>,
}

#[derive(Clone)]
pub struct RouteService(Arc<Inner>);

async fn save_link(
    db: &SharedDb,
    link: &mut crate::db::Link,
    status: LinkStatus,
    ctx: &str,
) -> bool {
    link.status = status;
    link.status_msg = String::new();
    link.last_updated = SystemTime::now();
    if let Err(e) = db.update_link(link.clone()).await {
        tracing::warn!("node_registered: {ctx}: {e}");
        return false;
    }
    true
}

impl RouteService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        reconciler_config: ReconcilerConfig,
        topology: TopologyConfig,
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
            node_locks: tokio::sync::Mutex::new(HashMap::new()),
            topology,
            link_graph: tokio::sync::RwLock::new(UnGraph::new_undirected()),
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
                    let Ok(nodes) = svc_clone.0.db.list_nodes().await else {
                        continue;
                    };
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

    /// Rebuild the runtime link graph from the given set of nodes.
    /// Extracts distinct group names and calls `build_graph()` on the topology config.
    /// Returns true if the set of groups changed (a group was added or removed).
    async fn rebuild_link_graph(&self, nodes: &[crate::db::Node]) -> bool {
        let new_groups: HashSet<&str> = nodes
            .iter()
            .map(|n| n.group_name.as_deref().unwrap_or(""))
            .collect();

        // Check if the group set changed compared to the current graph.
        let current_graph = self.0.link_graph.read().await;
        let current_groups: HashSet<&str> = current_graph
            .node_indices()
            .map(|idx| current_graph[idx].as_str())
            .collect();
        let groups_changed = new_groups != current_groups;
        drop(current_graph);

        if groups_changed {
            let group_vec: Vec<&str> = new_groups.into_iter().collect();
            let graph = self.0.topology.build_graph(&group_vec);
            *self.0.link_graph.write().await = graph;
        }

        groups_changed
    }

    /// Find the inter-group link between two groups.
    ///
    /// Searches for an existing (non-deleted) link between any node in `group_a`
    /// and any node in `group_b`. Returns the node_id in `group_a` (the gateway)
    /// and the link_id connecting them.
    async fn find_inter_group_link(
        &self,
        group_a: &str,
        group_b: &str,
        all_nodes: &[crate::db::Node],
    ) -> Option<(String, String)> {
        let nodes_a: Vec<&str> = all_nodes
            .iter()
            .filter(|n| n.group_name.as_deref() == Some(group_a))
            .map(|n| n.id.as_str())
            .collect();
        let nodes_b: Vec<&str> = all_nodes
            .iter()
            .filter(|n| n.group_name.as_deref() == Some(group_b))
            .map(|n| n.id.as_str())
            .collect();

        for &na in &nodes_a {
            for &nb in &nodes_b {
                if let Ok(link_id) = self.find_matching_link(na, nb).await {
                    return Some((na.to_string(), link_id));
                }
            }
        }
        None
    }

    /// Expand a wildcard route using the Shortest Path Tree.
    ///
    /// Given a route template (dest_node_id + name components), computes the SPT
    /// rooted at the destination's group. For each non-root group in the tree,
    /// selects a gateway node and installs a route pointing toward the parent group.
    async fn expand_route_via_spt(
        &self,
        dest_node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
        all_nodes: &[crate::db::Node],
    ) {
        // Resolve the destination node's group — this is the root of the SPT.
        let dest_group = all_nodes
            .iter()
            .find(|n| n.id == dest_node_id)
            .and_then(|n| n.group_name.as_deref())
            .unwrap_or("");

        // Read the current link graph and find the root group's index.
        let graph = self.0.link_graph.read().await;
        let root_idx = match graph.node_indices().find(|&idx| graph[idx] == dest_group) {
            Some(idx) => idx,
            None => {
                tracing::debug!(
                    "expand_route_via_spt: dest group '{dest_group}' not in link graph"
                );
                return;
            }
        };

        // Compute the SPT rooted at the destination group.
        let spt = match spt::compute_spt(root_idx, &graph) {
            Some(t) => t,
            None => return,
        };

        // For each non-root group in the SPT, install a route on the gateway
        // node pointing toward the parent group.
        for (&orig_idx, &tree_idx) in &spt.index_map {
            if orig_idx == root_idx {
                continue;
            }

            let child_group = &graph[orig_idx];

            // Find the parent group in the directed tree (incoming edge = from parent).
            let parent_tree_idx = match spt
                .tree
                .edges_directed(tree_idx, petgraph::Direction::Incoming)
                .next()
            {
                Some(edge) => edge.source(),
                None => continue,
            };
            let parent_group = &spt.tree[parent_tree_idx];

            // Find the inter-group link. The gateway node is the node in
            // child_group that holds this link.
            let (source_node_id, link_id) =
                match self.find_inter_group_link(child_group, parent_group, all_nodes).await {
                    Some(pair) => pair,
                    None => {
                        tracing::debug!(
                            "expand_route_via_spt: no link between '{child_group}' and '{parent_group}', skipping"
                        );
                        continue;
                    }
                };

            // Create the per-gateway route pointing toward the parent group.
            let per_node = Route {
                id: String::new(),
                source_node_id: source_node_id.clone(),
                dest_node_id: dest_node_id.to_string(),
                link_id: Some(link_id),
                component0: component0.to_string(),
                component1: component1.to_string(),
                component2: component2.to_string(),
                component_id: component_id.map(|s| s.to_string()),
                status: RouteStatus::Pending,
                status_msg: String::new(),
                created_at: SystemTime::now(),
                last_updated: SystemTime::now(),
            };
            if let Err(e) = self.add_single_route(per_node).await {
                tracing::debug!(
                    "expand_route_via_spt: route for {source_node_id} skipped: {e}"
                );
            }
        }
    }

    pub async fn add_route(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
        route: &ProtoRoute,
    ) -> Result<String> {
        validate_route_nodes(source_node_id, dest_node_id)?;

        let db_route = route.to_db_route(source_node_id, dest_node_id);
        let route_id = self.add_single_route(db_route).await?;

        // For wildcard source, expand the route using the SPT rooted at the
        // destination node's group. Each non-root group gets a route on its
        // gateway node pointing toward the parent group in the tree.
        if source_node_id == ALL_NODES_ID {
            let all_nodes = self.0.db.list_nodes().await?;
            let n = route.name.as_ref().unwrap();
            let (c0, c1, c2) = n.str_components();
            let comp_id = if n.id() == NameId::NULL_COMPONENT {
                None
            } else {
                Some(n.string_id())
            };
            self.expand_route_via_spt(
                dest_node_id,
                c0,
                c1,
                c2,
                comp_id.as_deref(),
                &all_nodes,
            )
            .await;
        }

        Ok(route_id)
    }

    async fn add_single_route(&self, mut db_route: Route) -> Result<String> {
        // If link_id was not pre-resolved by the caller (e.g. expand_route_via_spt),
        // try to find a direct link between source and dest. If none exists yet,
        // store with link_id=None — the reconciler will resolve it later.
        if db_route.source_node_id != ALL_NODES_ID && db_route.link_id.is_none() {
            db_route.link_id = self
                .find_matching_link(&db_route.source_node_id, &db_route.dest_node_id)
                .await
                .ok();
        }

        // Retry once if a stale soft-deleted route blocks insertion. The
        // get_route_by_id + delete_route + add_route sequence is not atomic, so
        // a concurrent caller can race; bound retries to 2 attempts.
        for attempt in 0..2 {
            match self.0.db.add_route(db_route.clone()).await {
                Ok(r) => {
                    let route_str = r.to_string();
                    tracing::info!("route added: {route_str}");
                    if db_route.source_node_id != ALL_NODES_ID {
                        self.0.queue.add(db_route.source_node_id);
                    }
                    return Ok(route_str);
                }
                Err(e) => {
                    let unique_id = db_route.compute_id();
                    match self.0.db.get_route_by_id(&unique_id).await? {
                        Some(existing) if existing.status == RouteStatus::Deleted => {
                            tracing::warn!(
                                "removing stale deleted route {} to allow re-add (attempt {})",
                                existing,
                                attempt + 1
                            );
                            match self.0.db.delete_route(&existing.id).await {
                                Ok(()) | Err(Error::RouteNotFound { .. }) => {}
                                Err(e) => return Err(e),
                            }
                            continue;
                        }
                        _ => {
                            return Err(Error::InvalidInput(format!("failed to add route: {e}")));
                        }
                    }
                }
            }
        }
        Err(Error::InvalidInput(
            "failed to add route after retries".into(),
        ))
    }

    pub async fn delete_route(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
        route: &ProtoRoute,
    ) -> Result<()> {
        if dest_node_id.is_empty() {
            return Err(Error::EmptyDestNodeId);
        }

        let (c0, c1, c2) = route.name.as_ref().unwrap().str_components();
        let comp_id = if route.name.as_ref().unwrap().id() == NameId::NULL_COMPONENT {
            None
        } else {
            Some(route.name.as_ref().unwrap().string_id())
        };

        let name = RouteName {
            component0: c0,
            component1: c1,
            component2: c2,
            component_id: comp_id.as_deref(),
        };

        if source_node_id == ALL_NODES_ID {
            // Delete the wildcard route itself.
            let db_route = self
                .0
                .db
                .get_route_for_src_dest_name(source_node_id, &name, dest_node_id, None)
                .await?
                .ok_or(Error::InvalidInput("route not found".to_string()))?;
            self.0.db.delete_route(&db_route.id).await?;

            // Also delete all per-node expansions.
            let per_node = self
                .0
                .db
                .get_routes_for_dest_node_id_and_name(dest_node_id, c0, c1, c2, comp_id.as_deref())
                .await?;
            for r in per_node {
                self.delete_single_route(&r.source_node_id, &r.id, &r.to_string())
                    .await?;
            }
            return Ok(());
        }

        let link_id = match route.link_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => {
                self.find_matching_link(source_node_id, dest_node_id)
                    .await?
            }
        };

        let db_route = self
            .0
            .db
            .get_route_for_src_dest_name(source_node_id, &name, dest_node_id, Some(&link_id))
            .await?
            .ok_or(Error::InvalidInput("route not found".to_string()))?;

        self.delete_single_route(source_node_id, &db_route.id, &db_route.to_string())
            .await
    }

    async fn delete_single_route(
        &self,
        node_id: &str,
        route_id: &str,
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
        self.reexpand_all_wildcard_routes(&all_nodes).await;

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
    async fn handle_incoming_link_on_register(
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
                .get_connection_details(&link.source_node_id, node_id)
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
            .get_connection_details(&link.source_node_id, &link.dest_node_id)
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
                self.reexpand_all_wildcard_routes(&remaining_nodes).await;
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

    /// Ensure direct or group links exist between `node_id` and every other
    /// node allowed by the topology link policy.
    /// Returns (affected_node_ids, newly_created_links).
    async fn ensure_links_for_node(
        &self,
        node_id: &str,
        existing_links: &[crate::db::Link],
        all_nodes: &[crate::db::Node],
        reported: &[ReportedConnection],
    ) -> (Vec<String>, Vec<crate::db::Link>) {
        let src_node = match all_nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n.clone(),
            None => {
                tracing::error!("ensure_links: node {node_id} not found");
                return (vec![node_id.to_string()], vec![]);
            }
        };
        let mut affected: HashSet<String> = [node_id.to_string()].into_iter().collect();
        let mut new_links: Vec<crate::db::Link> = Vec::new();

        let connected_peers: HashSet<String> = existing_links
            .iter()
            .filter(|l| l.status != LinkStatus::Deleted)
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
            // Topology policy: skip link creation if the groups are not allowed to link.
            let src_group = src_node.group_name.as_deref().unwrap_or("");
            let dst_group = other.group_name.as_deref().unwrap_or("");
            if !self.0.topology.can_link(src_group, dst_group) {
                tracing::debug!(
                    "topology policy: skipping link between {node_id} ({src_group}) and {} ({dst_group})",
                    other.id
                );
                continue;
            }
            let same_group = src_node.group_name == other.group_name;
            if same_group {
                if let Some((src, link)) = self.ensure_direct_link(&src_node, other, reported).await
                {
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
                if let Some((src, link)) = self.ensure_group_link(&src_node, other, reported).await
                {
                    affected.insert(src);
                    new_links.extend(link);
                }
                continue;
            }
            if has_src_external {
                if let Some((src, link)) = self.ensure_group_link(other, &src_node, reported).await
                {
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
        reported: &[ReportedConnection],
    ) -> Option<(String, Option<crate::db::Link>)> {
        let (endpoint, config_data, link_id, status) = if let Some(rc) =
            find_reported_connection_for_dest(reported, dst_node)
        {
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
                    tracing::error!("ensure_direct_link: failed to get connection details: {e}");
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
            created_at: SystemTime::now(),
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
        reported: &[ReportedConnection],
    ) -> Option<(String, Option<crate::db::Link>)> {
        let (endpoint, config_data, link_id, status) =
            if let Some(rc) = find_reported_connection_for_dest(reported, dst_node) {
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
                        tracing::error!("ensure_group_link: failed to get connection details: {e}");
                        return None;
                    }
                };
                // Reuse an existing link_id only when its destination matches dst_node.
                let lid = self
                    .0
                    .db
                    .get_link_for_source_and_endpoint(&src_node.id, &ep)
                    .await
                    .ok()
                    .flatten()
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
            created_at: SystemTime::now(),
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

    /// Re-expand every wildcard route template via SPT.
    /// Called when the group topology changes (group added/removed) or when a
    /// node registers and needs its routes populated.
    /// `add_single_route` rejects duplicates so this is idempotent.
    async fn reexpand_all_wildcard_routes(&self, all_nodes: &[crate::db::Node]) {
        let wildcard_routes = match self.0.db.get_routes_for_node(ALL_NODES_ID).await {
            Ok(r) => r,
            Err(_) => return,
        };

        for r in &wildcard_routes {
            self.expand_route_via_spt(
                &r.dest_node_id,
                &r.component0,
                &r.component1,
                &r.component2,
                r.component_id.as_deref(),
                all_nodes,
            )
            .await;
        }
    }

    async fn find_matching_link(&self, source: &str, dest: &str) -> Result<String> {
        match self.0.db.find_link_between_nodes(source, dest).await? {
            Some(l) if l.status != LinkStatus::Deleted => Ok(l.link_id),
            _ => Err(Error::InvalidInput(format!(
                "no matching link found for source={source} destination={dest}"
            ))),
        }
    }

    pub async fn list_node_routes(&self, node_id: &str) -> Result<RouteListResponse> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::RouteListRequest(
                crate::api::proto::controller::proto::v1::RouteListRequest {},
            )),
        };
        let chunks = self
            .0
            .cmd_handler
            .send_and_wait(node_id, msg, ResponseKind::RouteListResponse)
            .await?;
        let mut entries = Vec::new();
        for chunk in chunks {
            if let Some(Payload::RouteListResponse(r)) = chunk.payload {
                entries.extend(r.entries);
            }
        }
        Ok(RouteListResponse {
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
            .send_and_wait(node_id, msg, ResponseKind::ConnectionListResponse)
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
    ) -> Result<(String, ClientConfig)> {
        let dest_node =
            self.0
                .db
                .get_node(dest_node_id)
                .await?
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
                .await?
                .ok_or_else(|| Error::NodeNotFound {
                    id: source_node_id.to_string(),
                })?;

        compute_connection_details(&src_node, &dest_node)
    }
}

/// Compute the effective endpoint and serialised JSON config data from
/// already-loaded node objects, without hitting the DB.
fn compute_connection_details(
    src_node: &crate::db::Node,
    dst_node: &crate::db::Node,
) -> Result<(String, ClientConfig)> {
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
) -> Result<(String, ClientConfig)> {
    use slim_config::grpc::client::{BackoffConfig, KeepaliveConfig};
    use slim_config::tls::client::TlsClientConfig;
    use slim_config::tls::common::{CaSource, Config as TlsConfig, TlsSource};
    use std::time::Duration;

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

    let (effective_endpoint, tls_setting) = if let Some(ref spire) = detail.spire_mtls {
        let spire_socket = src_node
            .conn_details
            .iter()
            .find_map(|cd| cd.spire_mtls.as_ref().map(|s| s.socket_path.clone()))
            .unwrap_or_else(|| spire.socket_path.clone());

        let trust_domain = spire
            .trust_domain
            .as_deref()
            .or(dest_node.group_name.as_deref());

        let mut trust_domains = Vec::new();
        if let Some(td) = trust_domain {
            trust_domains.push(td.to_string());
        }

        let spire_config = slim_config::auth::spire::SpireConfig {
            socket_path: Some(spire_socket.clone()),
            trust_domains: trust_domains.clone(),
            ..Default::default()
        };

        let tls = TlsClientConfig {
            insecure: false,
            insecure_skip_verify: local_connection,
            config: TlsConfig {
                source: TlsSource::Spire {
                    config: slim_config::auth::spire::SpireConfig {
                        socket_path: Some(spire_socket),
                        ..Default::default()
                    },
                },
                ca_source: CaSource::Spire {
                    config: spire_config,
                },
                ..Default::default()
            },
        };

        (format!("https://{endpoint}"), tls)
    } else {
        let tls = TlsClientConfig {
            insecure: true,
            ..Default::default()
        };
        (format!("http://{endpoint}"), tls)
    };

    let client_config = ClientConfig {
        endpoint: effective_endpoint.clone(),
        tls_setting,
        backoff: BackoffConfig::new_fixed_interval(Duration::from_millis(2000), usize::MAX),
        keepalive: Some(KeepaliveConfig {
            tcp_keepalive: Duration::from_secs(20).into(),
            http2_keepalive: Duration::from_secs(20).into(),
            timeout: Duration::from_secs(20).into(),
            keep_alive_while_idle: false,
        }),
        link_id: String::new(),
        ..Default::default()
    };

    Ok((effective_endpoint, client_config))
}

fn find_reported_connection<'a>(
    reported: &'a [ReportedConnection],
    dest_endpoint: &str,
) -> Option<&'a ReportedConnection> {
    reported
        .iter()
        .find(|rc| endpoint_matches(&rc.endpoint, dest_endpoint))
}

fn find_reported_connection_for_dest<'a>(
    reported: &'a [ReportedConnection],
    dst_node: &crate::db::Node,
) -> Option<&'a ReportedConnection> {
    for rc in reported {
        for detail in &dst_node.conn_details {
            if endpoint_matches(&rc.endpoint, &detail.endpoint) {
                return Some(rc);
            }
            if let Some(ext) = &detail.external_endpoint
                && !ext.is_empty()
                && endpoint_matches(&rc.endpoint, ext)
            {
                return Some(rc);
            }
        }
    }
    None
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
    use slim_datapath::api::ProtoName;

    use super::*;
    use crate::db::ConnectionDetails;
    use crate::db::inmemory::InMemoryDb;
    use crate::node_transport::DefaultNodeCommandHandler;
    use std::time::SystemTime;

    fn make_conn_details(ep: &str, external: Option<&str>) -> ConnectionDetails {
        ConnectionDetails {
            endpoint: ep.to_string(),
            external_endpoint: external.map(|s| s.to_string()),
            spire_mtls: None,
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
            created_at: SystemTime::now(),
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
            TopologyConfig::default(),
        )
    }

    // ── select_connection ──────────────────────────────────────────────────

    #[test]
    fn select_connection_same_group_returns_first() {
        let dst = make_node(
            "dst",
            Some("grp"),
            vec![make_conn_details("dst:8080", Some("ext:9090"))],
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
                make_conn_details("dst:8080", None),
                make_conn_details("dst:8081", Some("ext:9090")),
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
            vec![make_conn_details("dst:8080", None)],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    // ── generate_config_data ───────────────────────────────────────────────

    #[test]
    fn generate_config_data_local_http() {
        let cd = make_conn_details("host:8080", None);
        let dest = make_node("dst", Some("g"), vec![cd.clone()]);
        let src = make_node("src", Some("g"), vec![]);
        let (ep, config) = generate_config_data(&cd, true, &dest, &src).unwrap();
        assert!(ep.starts_with("http://"));
        assert!(config.tls_setting.insecure);
        assert!(config.keepalive.is_some());
    }

    #[test]
    fn generate_config_data_external_no_mtls() {
        let cd = make_conn_details("host:8080", Some("ext:9090"));
        let dest = make_node("dst", Some("g1"), vec![cd.clone()]);
        let src = make_node("src", Some("g2"), vec![]);
        let (ep, config) = generate_config_data(&cd, false, &dest, &src).unwrap();
        assert!(ep.contains("ext:9090"));
        assert!(config.tls_setting.insecure);
    }

    #[test]
    fn generate_config_data_no_external_endpoint_remote_returns_error() {
        let cd = make_conn_details("host:8080", None);
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        // local_connection=false + no external endpoint → error.
        assert!(generate_config_data(&cd, false, &dest, &src).is_err());
    }

    // ── add_route validation ───────────────────────────────────────────────

    #[tokio::test]
    async fn add_route_empty_source_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("", "dst", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("source"));
    }

    #[tokio::test]
    async fn add_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("src", "", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("destination"));
    }

    #[tokio::test]
    async fn add_route_same_src_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("node1", "node1", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("same"));
    }

    // ── delete_route validation ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.delete_route("src", "", &route).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("destination node ID")
        );
    }

    #[tokio::test]
    async fn delete_route_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.delete_route("src", "dst", &route).await;
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
            conn_details: vec![make_conn_details("dst:8080", None)],
            created_at: SystemTime::now(),
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
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: None,
            conn_details: vec![],
            created_at: SystemTime::now(),
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
            conn_details: vec![make_conn_details("dst:8080", None)],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
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

    // ── topology: ensure_links_for_node ───────────────────────────────────

    use crate::config::AdjacencyEntry;

    fn make_route_service_with_topology(
        db: crate::db::SharedDb,
        topology: TopologyConfig,
    ) -> RouteService {
        let handler = DefaultNodeCommandHandler::new();
        RouteService::new(
            db,
            handler,
            ReconcilerConfig {
                max_requeues: 3,
                ..Default::default()
            },
            topology,
        )
    }

    fn star_topology() -> TopologyConfig {
        TopologyConfig {
            links: vec![
                AdjacencyEntry {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                AdjacencyEntry {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
        }
    }

    #[tokio::test]
    async fn ensure_links_star_skips_spoke_to_spoke() {
        let db = InMemoryDb::shared();
        let hub = make_node(
            "hub-node",
            Some("platform"),
            vec![make_conn_details("hub:8080", Some("hub-ext:9090"))],
        );
        let spoke_a = make_node(
            "spoke-a",
            Some("customer-a"),
            vec![make_conn_details("a:8080", Some("a-ext:9090"))],
        );
        let spoke_b = make_node(
            "spoke-b",
            Some("customer-b"),
            vec![make_conn_details("b:8080", Some("b-ext:9090"))],
        );
        db.save_node(hub.clone()).await.unwrap();
        db.save_node(spoke_a.clone()).await.unwrap();
        db.save_node(spoke_b.clone()).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());
        let all_nodes = db.list_nodes().await.unwrap();
        let (affected, new_links) = svc
            .ensure_links_for_node("spoke-a", &[], &all_nodes, &[])
            .await;

        // spoke-a should link to hub but NOT to spoke-b
        assert!(affected.contains(&"spoke-a".to_string()));
        assert!(new_links.iter().any(|l| l.dest_node_id == "hub-node"
            || l.source_node_id == "hub-node"));
        assert!(!new_links
            .iter()
            .any(|l| l.dest_node_id == "spoke-b" || l.source_node_id == "spoke-b"));
    }

    #[tokio::test]
    async fn ensure_links_full_mesh_connects_all() {
        let db = InMemoryDb::shared();
        let node_a = make_node(
            "node-a",
            Some("group-a"),
            vec![make_conn_details("a:8080", Some("a-ext:9090"))],
        );
        let node_b = make_node(
            "node-b",
            Some("group-b"),
            vec![make_conn_details("b:8080", Some("b-ext:9090"))],
        );
        let node_c = make_node(
            "node-c",
            Some("group-c"),
            vec![make_conn_details("c:8080", Some("c-ext:9090"))],
        );
        db.save_node(node_a.clone()).await.unwrap();
        db.save_node(node_b.clone()).await.unwrap();
        db.save_node(node_c.clone()).await.unwrap();

        // Default topology = full mesh
        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();
        let (_, new_links) = svc
            .ensure_links_for_node("node-a", &[], &all_nodes, &[])
            .await;

        // node-a should link to both node-b and node-c
        assert_eq!(new_links.len(), 2);
    }

    // ── topology: add_route wildcard expansion ────────────────────────────

    #[tokio::test]
    async fn add_route_wildcard_expands_via_spt() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke_a = make_node("spoke-a", Some("customer-a"), vec![]);
        let spoke_b = make_node("spoke-b", Some("customer-b"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke_a).await.unwrap();
        db.save_node(spoke_b).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());

        // Build the link graph (normally done in node_registered).
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create inter-group links (star: hub↔spoke-a, hub↔spoke-b).
        let link_hub_a = crate::db::Link {
            link_id: "link-hub-a".to_string(),
            source_node_id: "hub-node".to_string(),
            dest_node_id: "spoke-a".to_string(),
            dest_endpoint: "spoke-a:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        let link_hub_b = crate::db::Link {
            link_id: "link-hub-b".to_string(),
            source_node_id: "hub-node".to_string(),
            dest_node_id: "spoke-b".to_string(),
            dest_endpoint: "spoke-b:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_hub_a).await.unwrap();
        db.add_link(link_hub_b).await.unwrap();

        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "name", "type"])),
            ..Default::default()
        };

        // Wildcard route: all nodes → spoke-b.
        // SPT rooted at "customer-b": customer-b → platform → customer-a.
        // Hub (platform) gets route via link-hub-b, spoke-a (customer-a) via link to hub.
        svc.add_route(ALL_NODES_ID, "spoke-b", &route)
            .await
            .unwrap();

        let hub_routes = db.get_routes_for_node("hub-node").await.unwrap();
        let spoke_a_routes = db.get_routes_for_node("spoke-a").await.unwrap();

        // Hub should have a route to spoke-b (parent of hub in SPT is spoke-b itself,
        // so hub is a direct child of root and gets a route via link-hub-b).
        assert!(
            hub_routes
                .iter()
                .any(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "hub-node"),
            "hub should have a route to spoke-b"
        );

        // Spoke-a should have a route to spoke-b pointing toward hub (its parent in SPT).
        assert!(
            spoke_a_routes
                .iter()
                .any(|r| r.dest_node_id == "spoke-b"),
            "spoke-a should have a route to spoke-b via hub"
        );
    }

    // ── topology: SPT-based route expansion ──────────────────────────────

    #[tokio::test]
    async fn ensure_routes_spt_creates_route_via_parent() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke_a = make_node("spoke-a", Some("customer-a"), vec![]);
        let spoke_b = make_node("spoke-b", Some("customer-b"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke_a).await.unwrap();
        db.save_node(spoke_b).await.unwrap();

        // Star topology: platform links to all, spokes link only to platform.
        let topology = TopologyConfig {
            links: vec![
                AdjacencyEntry {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                AdjacencyEntry {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
        };

        let svc = make_route_service_with_topology(db.clone(), topology);

        // Add a wildcard route targeting spoke-b
        let wildcard_route = crate::db::Route {
            id: "wildcard-1".to_string(),
            source_node_id: ALL_NODES_ID.to_string(),
            dest_node_id: "spoke-b".to_string(),
            link_id: None,
            component0: "org".to_string(),
            component1: "name".to_string(),
            component2: "type".to_string(),
            component_id: None,
            status: RouteStatus::Pending,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_route(wildcard_route).await.unwrap();

        // spoke-a has a link to hub-node (star topology)
        let link_a_hub = crate::db::Link {
            link_id: "link-a-hub".to_string(),
            source_node_id: "spoke-a".to_string(),
            dest_node_id: "hub-node".to_string(),
            dest_endpoint: "hub:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_a_hub.clone()).await.unwrap();

        // hub-node has a link to spoke-b
        let link_hub_b = crate::db::Link {
            link_id: "link-hub-b".to_string(),
            source_node_id: "hub-node".to_string(),
            dest_node_id: "spoke-b".to_string(),
            dest_endpoint: "spoke-b:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_hub_b).await.unwrap();

        // Build the link graph (normally done in node_registered).
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Re-expand wildcard routes via SPT.
        // SPT rooted at "customer-b": customer-b → platform → customer-a
        // So spoke-a should get a route pointing to its parent (platform/hub-node).
        svc.reexpand_all_wildcard_routes(&all_nodes).await;

        // spoke-a should have a route to spoke-b via the hub link (toward parent).
        let spoke_a_routes = db.get_routes_for_node("spoke-a").await.unwrap();
        let transit_route = spoke_a_routes
            .iter()
            .find(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "spoke-a");
        assert!(
            transit_route.is_some(),
            "spoke-a should have a route to spoke-b"
        );
        assert_eq!(
            transit_route.unwrap().link_id.as_deref(),
            Some("link-a-hub"),
            "route should point to the hub link (parent in SPT)"
        );

        // hub-node should also have a route to spoke-b via the direct link.
        let hub_routes = db.get_routes_for_node("hub-node").await.unwrap();
        let hub_route = hub_routes
            .iter()
            .find(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "hub-node");
        assert!(
            hub_route.is_some(),
            "hub-node should have a route to spoke-b"
        );
        assert_eq!(
            hub_route.unwrap().link_id.as_deref(),
            Some("link-hub-b"),
            "hub route should point directly to spoke-b"
        );
    }

    #[tokio::test]
    async fn rebuild_link_graph_returns_false_when_groups_unchanged() {
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();

        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();

        // First call: groups change (empty → {group-a, group-b}).
        assert!(svc.rebuild_link_graph(&all_nodes).await);

        // Second call with same nodes: no change.
        assert!(!svc.rebuild_link_graph(&all_nodes).await);
    }

    #[tokio::test]
    async fn reexpand_is_idempotent() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke = make_node("spoke-a", Some("customer-a"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create a link hub↔spoke.
        let link = crate::db::Link {
            link_id: "link-1".to_string(),
            source_node_id: "hub-node".to_string(),
            dest_node_id: "spoke-a".to_string(),
            dest_endpoint: "spoke-a:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link).await.unwrap();

        // Add a wildcard route.
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "name", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "hub-node", &route)
            .await
            .unwrap();

        let count_before = db.get_routes_for_node("spoke-a").await.unwrap().len();

        // Re-expand again — should not create duplicates.
        svc.reexpand_all_wildcard_routes(&all_nodes).await;

        let count_after = db.get_routes_for_node("spoke-a").await.unwrap().len();
        assert_eq!(count_before, count_after, "re-expansion should be idempotent");
    }

    #[tokio::test]
    async fn spt_expansion_full_mesh_all_nodes_get_routes() {
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        let node_c = make_node("node-c", Some("group-c"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();
        db.save_node(node_c).await.unwrap();

        // Default topology = full mesh.
        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create all inter-group links (full mesh).
        for (src, dst, lid) in [
            ("node-a", "node-b", "link-ab"),
            ("node-a", "node-c", "link-ac"),
            ("node-b", "node-c", "link-bc"),
        ] {
            let link = crate::db::Link {
                link_id: lid.to_string(),
                source_node_id: src.to_string(),
                dest_node_id: dst.to_string(),
                dest_endpoint: format!("{dst}:8080"),
                conn_config_data: ClientConfig::default(),
                status: LinkStatus::Applied,
                status_msg: String::new(),
                created_at: SystemTime::now(),
                last_updated: SystemTime::now(),
            };
            db.add_link(link).await.unwrap();
        }

        // Add wildcard route to node-a (root = group-a).
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "svc", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "node-a", &route)
            .await
            .unwrap();

        // In full mesh SPT rooted at group-a, both group-b and group-c are
        // direct children. So node-b and node-c should each get a route.
        let routes_b = db.get_routes_for_node("node-b").await.unwrap();
        let routes_c = db.get_routes_for_node("node-c").await.unwrap();

        assert!(
            routes_b.iter().any(|r| r.dest_node_id == "node-a"),
            "node-b should have route to node-a"
        );
        assert!(
            routes_c.iter().any(|r| r.dest_node_id == "node-a"),
            "node-c should have route to node-a"
        );
    }

    #[tokio::test]
    async fn spt_expansion_chain_routes_through_intermediate() {
        // Chain: group-a — group-b — group-c
        // Route to node-a (root=group-a). node-c should route via node-b.
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        let node_c = make_node("node-c", Some("group-c"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();
        db.save_node(node_c).await.unwrap();

        // Chain topology: a↔b, b↔c (no direct a↔c).
        let topology = TopologyConfig {
            links: vec![
                AdjacencyEntry {
                    name: "group-a".to_string(),
                    peers: vec!["group-b".to_string()],
                },
                AdjacencyEntry {
                    name: "group-b".to_string(),
                    peers: vec!["group-a".to_string(), "group-c".to_string()],
                },
                AdjacencyEntry {
                    name: "group-c".to_string(),
                    peers: vec!["group-b".to_string()],
                },
            ],
        };
        let svc = make_route_service_with_topology(db.clone(), topology);
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Links: a↔b and b↔c.
        let link_ab = crate::db::Link {
            link_id: "link-ab".to_string(),
            source_node_id: "node-a".to_string(),
            dest_node_id: "node-b".to_string(),
            dest_endpoint: "node-b:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        let link_bc = crate::db::Link {
            link_id: "link-bc".to_string(),
            source_node_id: "node-b".to_string(),
            dest_node_id: "node-c".to_string(),
            dest_endpoint: "node-c:8080".to_string(),
            conn_config_data: ClientConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_ab).await.unwrap();
        db.add_link(link_bc).await.unwrap();

        // Add wildcard route to node-a.
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "svc", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "node-a", &route)
            .await
            .unwrap();

        // node-b (child of root group-a) should route via link-ab.
        let routes_b = db.get_routes_for_node("node-b").await.unwrap();
        let route_b = routes_b.iter().find(|r| r.dest_node_id == "node-a");
        assert!(route_b.is_some(), "node-b should have route to node-a");
        assert_eq!(route_b.unwrap().link_id.as_deref(), Some("link-ab"));

        // node-c (child of group-b in the chain) should route via link-bc
        // toward its parent (group-b), NOT directly to group-a.
        let routes_c = db.get_routes_for_node("node-c").await.unwrap();
        let route_c = routes_c.iter().find(|r| r.dest_node_id == "node-a");
        assert!(route_c.is_some(), "node-c should have route to node-a");
        assert_eq!(
            route_c.unwrap().link_id.as_deref(),
            Some("link-bc"),
            "node-c should route toward group-b (its parent in the SPT)"
        );
    }
}

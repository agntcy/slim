// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod commands;
mod connection_config;
mod links;
mod node_lifecycle;
pub mod reconciler;
mod routes;
pub mod spt;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use petgraph::graph::UnGraph;

use crate::config::{ReconcilerConfig, TopologyConfig};
use crate::db::{LinkStatus, SharedDb};
use crate::node_transport::DefaultNodeCommandHandler;
use crate::workqueue::WorkQueue;

pub use crate::types::ALL_NODES_ID;
pub use crate::types::DEFAULT_SEGMENT;
pub(crate) use connection_config::is_connection_not_found;

struct Inner {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    /// Single work queue for reconciliation (connections + subscriptions).
    queue: WorkQueue<String>,
    /// Signals the periodic sweep task to stop.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Per-node mutex that serializes node_deregistered and node_disconnected
    /// for the same node, preventing concurrent cleanup from corrupting state.
    node_locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Per-group mutex that serializes link creation for nodes in the same group.
    /// Without this, two nodes from the same group registering concurrently can
    /// both read the link table before either writes, causing duplicate inter-group
    /// links for the same group pair.
    group_locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Topology configuration for link and route filtering.
    topology: TopologyConfig,
    /// Runtime segment graphs at the group level. Rebuilt when nodes join/leave.
    /// Each entry is (segment_name, graph). For Links, there's a single "default" entry.
    /// For Segments, one entry per segment. For ApiManaged, loaded from DB.
    segment_graphs: tokio::sync::RwLock<Vec<(String, UnGraph<String, u32>)>>,
}

#[derive(Clone)]
pub struct RouteService(Arc<Inner>);

pub(super) async fn save_link(
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
            group_locks: tokio::sync::Mutex::new(HashMap::new()),
            topology,
            segment_graphs: tokio::sync::RwLock::new(Vec::new()),
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

    /// Returns an error if the topology is config-managed (not API-managed).
    /// Used as a guard by mutation APIs that are only available in API mode.
    pub fn ensure_api_mode(&self) -> Result<(), tonic::Status> {
        if self.0.topology.is_config_managed() {
            return Err(tonic::Status::failed_precondition(
                "topology is config-managed; modify the config file and restart to change topology",
            ));
        }
        Ok(())
    }

    /// Load segment graphs from the topology DB tables.
    ///
    /// **API mode only.** Reads all segments and their links from the DB,
    /// builds an undirected graph per segment, and stores them in
    /// `segment_graphs`. Called on startup and after topology mutations.
    pub async fn load_topology_from_db(&self) -> anyhow::Result<()> {
        let segments = self
            .0
            .db
            .list_topology_segments()
            .await
            .map_err(|e| anyhow::anyhow!("failed to load topology segments: {e}"))?;

        let mut graphs = Vec::new();
        for seg in &segments {
            let links = self
                .0
                .db
                .get_links_for_segment(&seg.id)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("failed to load links for segment {}: {e}", seg.name)
                })?;

            let mut graph = UnGraph::<String, u32>::new_undirected();
            let mut node_map: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();

            for (src, dst) in &links {
                let src_idx = *node_map
                    .entry(src.clone())
                    .or_insert_with(|| graph.add_node(src.clone()));
                let dst_idx = *node_map
                    .entry(dst.clone())
                    .or_insert_with(|| graph.add_node(dst.clone()));
                if !graph.contains_edge(src_idx, dst_idx) {
                    graph.add_edge(src_idx, dst_idx, 1);
                }
            }

            graphs.push((seg.name.clone(), graph));
        }

        *self.0.segment_graphs.write().await = graphs;
        Ok(())
    }

    /// After a topology mutation (add/remove link), re-evaluate links for all
    /// registered nodes. Creates new links where the topology now allows them,
    /// deletes links and associated routes that are no longer allowed, and queues
    /// affected nodes for reconciliation.
    async fn reconcile_topology_change(&self) {
        let all_nodes = match self.0.db.list_nodes().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("reconcile_topology_change: list_nodes: {e}");
                return;
            }
        };
        if all_nodes.is_empty() {
            return;
        }

        let allowed_pairs = self.allowed_link_pairs().await;
        let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
            tracing::error!("reconcile_topology_change: list_all_links: {e}");
            vec![]
        });

        let mut reconcile_nodes: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Delete links whose group pair is no longer allowed by the topology,
        // along with any routes that referenced those links.
        for link in &all_links {
            if link.status == crate::db::LinkStatus::Deleted {
                continue;
            }
            let pair = (link.source_group.clone(), link.dest_group.clone());
            if !allowed_pairs.contains(&pair) {
                tracing::info!(
                    "reconcile_topology_change: removing disallowed link {}↔{} (groups {}↔{})",
                    link.source_node_id,
                    link.dest_node_id,
                    link.source_group,
                    link.dest_group,
                );
                // Delete routes that depended on this link.
                match self.0.db.get_routes_by_link_id(&link.link_id).await {
                    Ok(routes) => {
                        for route in &routes {
                            if let Err(e) = self.0.db.delete_route(&route.id).await {
                                tracing::error!("reconcile_topology_change: delete_route: {e}");
                            }
                            reconcile_nodes.insert(route.source_node_id.clone());
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            link_id = %link.link_id,
                            "reconcile_topology_change: get_routes_by_link_id: {e}; \
                             skipping link deletion to retry on next cycle"
                        );
                        continue;
                    }
                }
                if let Err(e) = self.0.db.delete_link(link).await {
                    tracing::error!("reconcile_topology_change: delete_link: {e}");
                }
                reconcile_nodes.insert(link.source_node_id.clone());
                reconcile_nodes.insert(link.dest_node_id.clone());
            }
        }

        // Create new links where the topology now allows them.
        // Re-read links from DB since we may have deleted some above.
        let mut current_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
            tracing::error!("reconcile_topology_change: list_all_links (post-delete): {e}");
            vec![]
        });

        // Only call ensure_links for one node per group to avoid creating
        // duplicate inter-group links (the gateway pattern: one link per group pair).
        // Re-read links after each call so the next group sees newly created links.
        let mut seen_groups: std::collections::HashSet<String> = std::collections::HashSet::new();
        for node in &all_nodes {
            let group = node.group_name.as_deref().unwrap_or("").to_string();
            if !seen_groups.insert(group) {
                continue;
            }
            let node_links: Vec<_> = current_links
                .iter()
                .filter(|l| l.source_node_id == node.id || l.dest_node_id == node.id)
                .cloned()
                .collect();

            let (affected, new_links) = self
                .ensure_links_for_node(
                    &node.id,
                    &node_links,
                    &all_nodes,
                    &current_links,
                    &[],
                    &allowed_pairs,
                )
                .await;
            reconcile_nodes.extend(affected);

            // Update snapshot so the next group sees newly created links.
            if !new_links.is_empty() {
                current_links.extend(new_links);
            }
        }

        // Re-expand wildcard routes to pick up new paths via newly created links.
        self.expand_all_wildcard_routes(&all_nodes, &current_links)
            .await;

        for nid in &reconcile_nodes {
            self.0.queue.add(nid.clone());
        }
    }

    /// Add a segment (idempotent — succeeds silently if it already exists).
    pub async fn add_segment(&self, name: &str) -> Result<(), tonic::Status> {
        self.ensure_api_mode()?;
        self.0
            .db
            .create_segment(name)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to create segment: {e}")))?;
        self.load_topology_from_db()
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to reload topology: {e}")))?;
        Ok(())
    }

    /// Remove a segment by name. The default segment cannot be removed.
    pub async fn remove_segment(&self, name: &str) -> Result<(), tonic::Status> {
        self.ensure_api_mode()?;
        if name == DEFAULT_SEGMENT {
            return Err(tonic::Status::failed_precondition(
                "the default segment cannot be removed; remove its links instead",
            ));
        }
        let seg = self
            .0
            .db
            .get_segment_by_name(name)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to query segment: {e}")))?
            .ok_or_else(|| tonic::Status::not_found(format!("segment '{name}' not found")))?;
        self.0
            .db
            .delete_segment(&seg.id)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to delete segment: {e}")))?;
        self.load_topology_from_db()
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to reload topology: {e}")))?;
        self.reconcile_topology_change().await;
        Ok(())
    }

    /// Add a bidirectional topology link between two groups in a segment.
    /// The segment must already exist (use `add_segment` first).
    /// Both groups must have at least one registered node.
    pub async fn add_topology_link(
        &self,
        group_a: &str,
        group_b: &str,
        segment: &str,
    ) -> Result<Vec<String>, tonic::Status> {
        self.ensure_api_mode()?;

        // Check which groups have registered nodes (for warnings).
        let all_nodes = self
            .0
            .db
            .list_nodes()
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to list nodes: {e}")))?;
        let has_group_a = all_nodes
            .iter()
            .any(|n| n.group_name.as_deref() == Some(group_a));
        let has_group_b = all_nodes
            .iter()
            .any(|n| n.group_name.as_deref() == Some(group_b));
        let mut warnings = Vec::new();
        if !has_group_a {
            warnings.push(format!(
                "group '{}' has no registered nodes yet; physical link will be created when nodes register",
                group_a
            ));
        }
        if !has_group_b {
            warnings.push(format!(
                "group '{}' has no registered nodes yet; physical link will be created when nodes register",
                group_b
            ));
        }

        let seg = self
            .0
            .db
            .get_segment_by_name(segment)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to query segment: {e}")))?
            .ok_or_else(|| tonic::Status::not_found(format!("segment '{segment}' not found")))?;
        self.0
            .db
            .add_link_to_segment(&seg.id, group_a, group_b)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to add link: {e}")))?;
        self.load_topology_from_db()
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to reload topology: {e}")))?;
        self.reconcile_topology_change().await;
        Ok(warnings)
    }

    /// Remove a bidirectional topology link between two groups in a segment.
    pub async fn remove_topology_link(
        &self,
        group_a: &str,
        group_b: &str,
        segment: &str,
    ) -> Result<(), tonic::Status> {
        self.ensure_api_mode()?;
        let seg = self
            .0
            .db
            .get_segment_by_name(segment)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to query segment: {e}")))?
            .ok_or_else(|| tonic::Status::not_found(format!("segment '{segment}' not found")))?;
        // Check if the link exists before deleting.
        let links = self
            .0
            .db
            .get_links_for_segment(&seg.id)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to query links: {e}")))?;
        let exists = links.iter().any(|(src, dst)| {
            (src == group_a && dst == group_b) || (src == group_b && dst == group_a)
        });
        if !exists {
            return Err(tonic::Status::not_found(format!(
                "link {}↔{} not found in segment '{segment}'",
                group_a, group_b
            )));
        }
        self.0
            .db
            .delete_link_from_segment(&seg.id, group_a, group_b)
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to remove link: {e}")))?;
        self.load_topology_from_db()
            .await
            .map_err(|e| tonic::Status::internal(format!("failed to reload topology: {e}")))?;
        self.reconcile_topology_change().await;
        Ok(())
    }

    /// Stop the reconciler workers and wait for any in-flight reconciliations
    /// to finish before returning.
    pub async fn shutdown(&self) {
        tracing::info!("route service: shutting down reconcilers");
        let _ = self.0.shutdown_tx.send(true);
        self.0.queue.shutdown_with_drain().await;
        tracing::info!("route service: reconcilers stopped");
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use std::time::SystemTime;

    use crate::config::AdjacencyEntry;
    use crate::config::{ReconcilerConfig, TopologyConfig};
    use crate::db::ConnectionDetails;
    use crate::node_transport::DefaultNodeCommandHandler;

    use super::RouteService;

    pub(super) fn make_conn_details(ep: &str, external: Option<&str>) -> ConnectionDetails {
        ConnectionDetails {
            endpoint: ep.to_string(),
            external_endpoint: external.map(|s| s.to_string()),
            tls_required: false,
            auth_method: "none".to_string(),
            spire_trust_domain: None,
        }
    }

    pub(super) fn make_node(
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

    pub(super) fn make_route_service(db: crate::db::SharedDb) -> RouteService {
        use crate::config::AdjacencyEntry;
        let handler = DefaultNodeCommandHandler::new();
        RouteService::new(
            db,
            handler,
            ReconcilerConfig {
                max_requeues: 3,
                ..Default::default()
            },
            TopologyConfig::Links(vec![AdjacencyEntry {
                group: "*".to_string(),
                neighbors: vec!["*".to_string()],
            }]),
        )
    }

    pub(super) fn make_route_service_with_topology(
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

    pub(super) fn star_topology() -> TopologyConfig {
        TopologyConfig::Links(vec![
            AdjacencyEntry {
                group: "platform".to_string(),
                neighbors: vec!["*".to_string()],
            },
            AdjacencyEntry {
                group: "*".to_string(),
                neighbors: vec!["platform".to_string()],
            },
        ])
    }
}

#[cfg(test)]
mod topology_mutation_tests {
    use super::test_utils::make_route_service_with_topology;
    use crate::config::TopologyConfig;
    use crate::db::inmemory::InMemoryDb;
    use crate::db::model::{ConnectionDetails, Node};
    use std::time::SystemTime;

    fn api_managed_service() -> super::RouteService {
        let db = InMemoryDb::shared();
        make_route_service_with_topology(db, TopologyConfig::ApiManaged)
    }

    /// Helper to register nodes so group validation passes.
    async fn register_groups(svc: &super::RouteService, groups: &[&str]) {
        for (i, group) in groups.iter().enumerate() {
            let node = Node {
                id: format!("{group}/node-{i}"),
                group_name: Some(group.to_string()),
                conn_details: vec![ConnectionDetails {
                    endpoint: format!("127.0.0.1:{}", 9000 + i),
                    external_endpoint: None,
                    tls_required: false,
                    auth_method: "none".to_string(),
                    spire_trust_domain: None,
                }],
                created_at: SystemTime::now(),
                last_updated: SystemTime::now(),
            };
            svc.0.db.save_node(node).await.unwrap();
        }
    }

    fn config_managed_service() -> super::RouteService {
        let db = InMemoryDb::shared();
        make_route_service_with_topology(db, super::test_utils::star_topology())
    }

    #[tokio::test]
    async fn add_segment_succeeds() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        let segments = svc.list_segments().await;
        assert!(segments.iter().any(|(name, _, _)| name == "prod"));
    }

    #[tokio::test]
    async fn add_segment_rejects_in_config_mode() {
        let svc = config_managed_service();
        let err = svc.add_segment("prod").await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn add_segment_idempotent() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        // Second add should succeed silently
        svc.add_segment("prod").await.unwrap();
    }

    #[tokio::test]
    async fn remove_segment_succeeds() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        svc.remove_segment("prod").await.unwrap();
        let segments = svc.list_segments().await;
        assert!(!segments.iter().any(|(name, _, _)| name == "prod"));
    }

    #[tokio::test]
    async fn remove_segment_not_found() {
        let svc = api_managed_service();
        let err = svc.remove_segment("nonexistent").await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn add_topology_link_succeeds() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        register_groups(&svc, &["cloud", "customer-a"]).await;
        svc.add_topology_link("cloud", "customer-a", "prod")
            .await
            .unwrap();
        let segments = svc.list_segments().await;
        let prod = segments.iter().find(|(name, _, _)| name == "prod").unwrap();
        assert!(
            prod.2
                .contains(&("cloud".to_string(), "customer-a".to_string()))
        );
    }

    #[tokio::test]
    async fn add_topology_link_segment_not_found() {
        let svc = api_managed_service();
        register_groups(&svc, &["cloud", "customer-a"]).await;
        let err = svc
            .add_topology_link("cloud", "customer-a", "nonexistent")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn add_topology_link_idempotent() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        register_groups(&svc, &["cloud", "customer-a"]).await;
        svc.add_topology_link("cloud", "customer-a", "prod")
            .await
            .unwrap();
        // Second add should not error
        svc.add_topology_link("cloud", "customer-a", "prod")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn remove_topology_link_succeeds() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        register_groups(&svc, &["cloud", "customer-a"]).await;
        svc.add_topology_link("cloud", "customer-a", "prod")
            .await
            .unwrap();
        svc.remove_topology_link("cloud", "customer-a", "prod")
            .await
            .unwrap();
        let segments = svc.list_segments().await;
        let prod = segments.iter().find(|(name, _, _)| name == "prod").unwrap();
        assert!(prod.2.is_empty());
    }

    #[tokio::test]
    async fn remove_topology_link_segment_not_found() {
        let svc = api_managed_service();
        let err = svc
            .remove_topology_link("cloud", "customer-a", "nonexistent")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn add_topology_link_warns_on_missing_group() {
        let svc = api_managed_service();
        svc.add_segment("prod").await.unwrap();
        register_groups(&svc, &["cloud"]).await;
        let warnings = svc
            .add_topology_link("cloud", "nonexistent", "prod")
            .await
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("nonexistent"));
    }

    #[tokio::test]
    async fn remove_default_segment_rejected() {
        let svc = api_managed_service();
        svc.add_segment(super::DEFAULT_SEGMENT).await.unwrap();
        let err = svc
            .remove_segment(super::DEFAULT_SEGMENT)
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("cannot be removed"));
    }
}

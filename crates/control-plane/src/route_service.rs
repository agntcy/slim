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
    /// Runtime link graph at the group level. Rebuilt when nodes join/leave.
    link_graph: tokio::sync::RwLock<UnGraph<String, u32>>,
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
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_string::DurationString;
use serde::Deserialize;
use std::time::Duration;

use slim_config::grpc::server::ServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_tracing::TracingConfiguration;

/// Top-level control-plane configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Northbound gRPC API (management / ControlPlaneService).
    pub northbound: ServerConfig,
    /// Southbound gRPC API (node registration / ControllerService).
    pub southbound: ServerConfig,
    /// Settings for the route and link reconcilers.
    pub reconciler: ReconcilerConfig,
    /// Database backend configuration.
    pub database: DatabaseConfig,
    /// Tracing / logging configuration.
    pub tracing: TracingConfiguration,
    /// Topology configuration: controls link creation and route visibility
    /// between node groups.
    pub topology: TopologyConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            northbound: ServerConfig {
                endpoint: "0.0.0.0:50051".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            },
            southbound: ServerConfig {
                endpoint: "0.0.0.0:50052".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            },
            reconciler: ReconcilerConfig::default(),
            database: DatabaseConfig::default(),
            tracing: TracingConfiguration::default(),
            topology: TopologyConfig::default(),
        }
    }
}

/// Database backend selection.
#[derive(Debug, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseConfig {
    /// Pure in-memory store (default). All state is lost on restart.
    #[default]
    InMemory,
    /// SQLite-backed persistent store.
    Sqlite {
        /// Path to the SQLite database file.
        path: String,
    },
}

/// Reconciler tuning parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReconcilerConfig {
    /// Maximum number of times a failed reconcile request is requeued.
    pub max_requeues: usize,
    /// Base delay for the first retry. Subsequent retries use exponential
    /// backoff (base × 2^(attempt-1)) capped at 30 s.
    /// Accepts any duration string understood by the `duration-string` crate
    /// (e.g. `"200ms"`, `"1s"`, `"1m30s"`).
    pub base_retry_delay: DurationString,
    /// How often all connected nodes are re-enqueued for a full reconciliation
    /// sweep. Set to `"0s"` to disable.
    pub reconcile_period: DurationString,
    /// When true, the link reconciler will delete outgoing connections found on
    /// a data-plane node whose link_id is not present in the control-plane DB.
    ///
    /// Disable this (the default) when data-plane nodes may have connections
    /// that were established outside the control plane (e.g. connections created
    /// by a previous CP instance, or manually configured connections). Enabling
    /// it is useful in greenfield deployments where the CP is the sole source of
    /// truth for all data-plane connections.
    pub enable_orphan_detection: bool,
    /// Number of concurrent worker tasks spawned for each reconciler (link and
    /// route). All workers consume from the same work queue; the queue ensures
    /// a given node is never processed by more than one worker at a time.
    /// Must be at least 1; values below 1 are clamped to 1 at runtime.
    pub workers: usize,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            max_requeues: 15,
            base_retry_delay: Duration::from_millis(200).into(),
            reconcile_period: Duration::from_secs(60).into(),
            enable_orphan_detection: false,
            workers: 4,
        }
    }
}

/// Topology configuration: defines the physical link graph between node groups.
///
/// The topology is expressed as an adjacency list. Each entry declares
/// a group name and the peers it connects to. All links are **bidirectional**:
/// if group A lists B as a peer, then B↔A is implied.
///
/// The wildcard `"*"` matches all registered groups and is resolved dynamically
/// at node registration time.
///
/// If no topology is configured (empty links), the controller defaults to
/// **full mesh**: every group links to every other group.
///
/// # Example
///
/// ```yaml
/// topology:
///   links:
///     - name: hub
///       peers: ["*"]      # hub connects to all groups
///     - name: node-b
///       peers: [node-c]   # explicit link between node-b and node-c
/// ```
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TopologyConfig {
    /// Adjacency list: defines which groups create links to each other.
    /// If empty, all groups can link to all groups (full mesh).
    pub links: Vec<AdjacencyEntry>,
}

/// An adjacency list entry: nodes in group `name` connect to nodes
/// in any of the groups listed in `peers`. Links are bidirectional.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdjacencyEntry {
    /// Group name (or `"*"` to match any group).
    pub name: String,
    /// Groups this group connects to. `"*"` matches any group.
    pub peers: Vec<String>,
}

impl TopologyConfig {
    /// Returns true if group `a` is allowed to create a link to group `b`.
    /// Links are bidirectional: if any entry allows a→b or b→a, both are linked.
    /// If no entries are configured, defaults to allow (full mesh).
    pub fn can_link(&self, a: &str, b: &str) -> bool {
        if self.links.is_empty() {
            return true;
        }
        self.links.iter().any(|entry| {
            (matches_group(&entry.name, a)
                && entry.peers.iter().any(|p| matches_group(p, b)))
                || (matches_group(&entry.name, b)
                    && entry.peers.iter().any(|p| matches_group(p, a)))
        })
    }

    /// Resolve the link graph for a set of known groups.
    /// Returns a petgraph `UnGraph` with group names as node weights and
    /// edge weight 1 (uniform cost for now, future-proofed for weighted links).
    ///
    /// Wildcard `"*"` in entries is expanded to all groups in `known_groups`.
    pub fn build_graph(
        &self,
        known_groups: &[&str],
    ) -> petgraph::graph::UnGraph<String, u32> {
        use petgraph::graph::UnGraph;
        use std::collections::HashMap;

        let mut graph = UnGraph::<String, u32>::new_undirected();
        let mut indices: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();

        // Add all known groups as nodes
        for &group in known_groups {
            let idx = graph.add_node(group.to_string());
            indices.insert(group, idx);
        }

        if self.links.is_empty() {
            // Full mesh: connect everything to everything
            for (i, &a) in known_groups.iter().enumerate() {
                for &b in &known_groups[i + 1..] {
                    graph.add_edge(indices[a], indices[b], 1);
                }
            }
        } else {
            // Apply adjacency entries with wildcard expansion
            for entry in &self.links {
                let sources: Vec<&str> = if entry.name == "*" {
                    known_groups.to_vec()
                } else {
                    known_groups
                        .iter()
                        .filter(|&&g| g == entry.name)
                        .copied()
                        .collect()
                };

                for &src in &sources {
                    for peer_pattern in &entry.peers {
                        let targets: Vec<&str> = if peer_pattern == "*" {
                            known_groups.to_vec()
                        } else {
                            known_groups
                                .iter()
                                .filter(|&&g| g == *peer_pattern)
                                .copied()
                                .collect()
                        };

                        for &dst in &targets {
                            if src == dst {
                                continue;
                            }
                            let src_idx = indices[src];
                            let dst_idx = indices[dst];
                            if graph.find_edge(src_idx, dst_idx).is_none() {
                                graph.add_edge(src_idx, dst_idx, 1);
                            }
                        }
                    }
                }
            }
        }

        graph
    }
}

// ─── Deprecated stubs (to be removed in Step 1.6) ────────────────────────────
// These methods maintain backward compatibility with route_service and reconciler
// code that will be rewritten in Steps 1.2–1.7.


/// Returns true if `pattern` matches `group`. `"*"` matches any group.
fn matches_group(pattern: &str, group: &str) -> bool {
    pattern == "*" || pattern == group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciler_config_defaults() {
        let c = ReconcilerConfig::default();
        assert_eq!(c.max_requeues, 15);
        assert_eq!(
            Duration::from(c.base_retry_delay),
            Duration::from_millis(200)
        );
        assert_eq!(Duration::from(c.reconcile_period), Duration::from_secs(60));
        assert_eq!(c.workers, 4);
    }

    #[test]
    fn config_defaults() {
        let c = Config::default();
        assert_eq!(c.northbound.endpoint, "0.0.0.0:50051");
        assert_eq!(c.southbound.endpoint, "0.0.0.0:50052");
        assert_eq!(c.topology, TopologyConfig::default());
    }

    #[test]
    fn topology_default_is_full_mesh() {
        let t = TopologyConfig::default();
        assert!(t.can_link("a", "b"));
        assert!(t.can_link("x", "y"));
    }

    #[test]
    fn topology_can_link_star() {
        let t = TopologyConfig {
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
        assert!(t.can_link("platform", "customer-a"));
        assert!(t.can_link("customer-a", "platform"));
        assert!(!t.can_link("customer-a", "customer-b"));
    }

    #[test]
    fn topology_can_link_explicit_pair() {
        let t = TopologyConfig {
            links: vec![AdjacencyEntry {
                name: "node-a".to_string(),
                peers: vec!["node-b".to_string()],
            }],
        };
        // Bidirectional
        assert!(t.can_link("node-a", "node-b"));
        assert!(t.can_link("node-b", "node-a"));
        // No link to others
        assert!(!t.can_link("node-a", "node-c"));
        assert!(!t.can_link("node-b", "node-c"));
    }

    #[test]
    fn build_graph_full_mesh() {
        let t = TopologyConfig::default();
        let groups = vec!["a", "b", "c", "d"];
        let graph = t.build_graph(&groups);

        assert_eq!(graph.node_count(), 4);
        // Full mesh with 4 nodes = 6 edges
        assert_eq!(graph.edge_count(), 6);
    }

    #[test]
    fn build_graph_star() {
        let t = TopologyConfig {
            links: vec![AdjacencyEntry {
                name: "hub".to_string(),
                peers: vec!["*".to_string()],
            }],
        };
        let groups = vec!["hub", "a", "b", "c"];
        let graph = t.build_graph(&groups);

        assert_eq!(graph.node_count(), 4);
        // Star: hub connects to a, b, c = 3 edges
        assert_eq!(graph.edge_count(), 3);
    }

    #[test]
    fn build_graph_chain() {
        let t = TopologyConfig {
            links: vec![
                AdjacencyEntry {
                    name: "a".to_string(),
                    peers: vec!["b".to_string()],
                },
                AdjacencyEntry {
                    name: "b".to_string(),
                    peers: vec!["c".to_string()],
                },
                AdjacencyEntry {
                    name: "c".to_string(),
                    peers: vec!["d".to_string()],
                },
            ],
        };
        let groups = vec!["a", "b", "c", "d"];
        let graph = t.build_graph(&groups);

        assert_eq!(graph.node_count(), 4);
        // Chain: a-b, b-c, c-d = 3 edges
        assert_eq!(graph.edge_count(), 3);
    }

    #[test]
    fn build_graph_no_self_links() {
        let t = TopologyConfig {
            links: vec![AdjacencyEntry {
                name: "*".to_string(),
                peers: vec!["*".to_string()],
            }],
        };
        let groups = vec!["a", "b"];
        let graph = t.build_graph(&groups);

        // 2 nodes, 1 edge (no self-links)
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn build_graph_no_duplicate_edges() {
        // Both entries create a↔b, but should only be 1 edge
        let t = TopologyConfig {
            links: vec![
                AdjacencyEntry {
                    name: "a".to_string(),
                    peers: vec!["b".to_string()],
                },
                AdjacencyEntry {
                    name: "b".to_string(),
                    peers: vec!["a".to_string()],
                },
            ],
        };
        let groups = vec!["a", "b"];
        let graph = t.build_graph(&groups);

        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn build_graph_unknown_group_ignored() {
        let t = TopologyConfig {
            links: vec![AdjacencyEntry {
                name: "a".to_string(),
                peers: vec!["unknown".to_string()],
            }],
        };
        let groups = vec!["a", "b"];
        let graph = t.build_graph(&groups);

        // "unknown" not in known_groups, so no edge created
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
    }
}

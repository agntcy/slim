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

/// Topology configuration: controls physical connectivity (links) and
/// logical visibility (routing policy) between node groups.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TopologyConfig {
    /// Link policy: defines which groups create links to each other.
    /// If empty, all groups can link to all groups (full mesh).
    pub links: Vec<LinkPolicy>,
    /// Routing policy: defines which groups can discover each other's agents.
    /// If empty, all groups can see all groups (full mesh).
    /// If any rules are present, only explicitly listed visibility is allowed.
    pub routing_policy: Vec<RoutingPolicy>,
}

/// A link policy entry: nodes in group `name` will create links to nodes
/// in any of the groups listed in `peers`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LinkPolicy {
    /// Group name (or `"*"` to match any group).
    pub name: String,
    /// Groups this group will create links to. `"*"` matches any group.
    pub peers: Vec<String>,
}

/// A routing policy entry: nodes in group `from` can discover agents on nodes
/// in any of the groups listed in `can_reach`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RoutingPolicy {
    /// Source group name (or `"*"` to match any group).
    pub from: String,
    /// Destination groups that `from` can reach. `"*"` matches any group.
    pub can_reach: Vec<String>,
}

impl TopologyConfig {
    /// Returns true if group `src` is allowed to create a link to group `dst`.
    /// If no link policies are configured, defaults to allow (full mesh).
    pub fn can_link(&self, src_group: &str, dst_group: &str) -> bool {
        if self.links.is_empty() {
            return true;
        }
        self.links.iter().any(|policy| {
            matches_group(&policy.name, src_group)
                && policy.peers.iter().any(|p| matches_group(p, dst_group))
        }) || self.links.iter().any(|policy| {
            matches_group(&policy.name, dst_group)
                && policy.peers.iter().any(|p| matches_group(p, src_group))
        })
    }

    /// Returns true if group `src` is allowed to see routes from group `dst`.
    /// Same-group visibility is always allowed.
    /// If no routing policies are configured, defaults to allow (full mesh).
    /// If any policies are present, only explicitly listed visibility is allowed.
    pub fn can_see(&self, src_group: &str, dst_group: &str) -> bool {
        if src_group == dst_group {
            return true;
        }
        if self.routing_policy.is_empty() {
            return true;
        }
        for policy in &self.routing_policy {
            if matches_group(&policy.from, src_group)
                && policy.can_reach.iter().any(|g| matches_group(g, dst_group))
            {
                return true;
            }
        }
        false
    }

    /// Detects the hub group in a star topology.
    /// The hub is a non-wildcard group that has `peers: ["*"]` in the link policy.
    /// Returns `None` if no hub is found.
    pub fn hub_group(&self) -> Option<&str> {
        for policy in &self.links {
            if policy.name != "*" && policy.peers.iter().any(|p| p == "*") {
                return Some(&policy.name);
            }
        }
        None
    }

    /// Returns true if the topology is a valid star (hub-and-spoke) configuration:
    /// - Exactly one hub group that peers with everyone.
    /// - All other (non-wildcard) link policies only peer with the hub.
    pub fn is_star(&self) -> bool {
        let hub = match self.hub_group() {
            Some(h) => h,
            None => return false,
        };
        self.links.iter().all(|policy| {
            // The hub itself — valid
            if policy.name == hub {
                return true;
            }
            // All other groups (including "*") must only peer with the hub
            policy.peers.iter().all(|p| p == hub)
        })
    }

    /// Validates the topology configuration. Returns an error if a routing
    /// policy allows visibility between two groups that have no direct link
    /// and the topology is not star-shaped (no transit available).
    ///
    /// In a valid star topology, transit through the hub is implicitly allowed
    /// for any spoke-to-spoke visibility rule since all spokes connect to the hub.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.links.is_empty() || self.routing_policy.is_empty() {
            return Ok(());
        }

        let is_star = self.is_star();

        for policy in &self.routing_policy {
            for target in &policy.can_reach {
                // Skip wildcards — validated against concrete groups at runtime
                if policy.from == "*" || target == "*" {
                    continue;
                }
                // Same group is always fine
                if policy.from == *target {
                    continue;
                }
                // Direct link exists — no issue
                if self.can_link(&policy.from, target) {
                    continue;
                }
                // No direct link — transit only available in star topology
                if !is_star {
                    return Err(crate::error::Error::NoLinkForVisibility {
                        from: policy.from.clone(),
                        to: target.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

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
        assert!(t.can_see("a", "b"));
        assert!(t.can_see("x", "y"));
    }

    #[test]
    fn topology_can_link_star() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                LinkPolicy {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
            routing_policy: vec![],
        };
        assert!(t.can_link("platform", "customer-a"));
        assert!(t.can_link("customer-a", "platform"));
        assert!(!t.can_link("customer-a", "customer-b"));
    }

    #[test]
    fn topology_can_see_full_mesh_when_empty() {
        let t = TopologyConfig {
            links: vec![],
            routing_policy: vec![],
        };
        assert!(t.can_see("customer-a", "customer-b"));
        assert!(t.can_see("customer-b", "customer-a"));
    }

    #[test]
    fn topology_can_see_star() {
        let t = TopologyConfig {
            links: vec![],
            routing_policy: vec![
                RoutingPolicy {
                    from: "*".to_string(),
                    can_reach: vec!["platform".to_string()],
                },
                RoutingPolicy {
                    from: "platform".to_string(),
                    can_reach: vec!["*".to_string()],
                },
            ],
        };
        assert!(t.can_see("customer-a", "platform"));
        assert!(t.can_see("platform", "customer-a"));
        assert!(t.can_see("platform", "customer-b"));
        assert!(!t.can_see("customer-a", "customer-b"));
        assert!(!t.can_see("customer-b", "customer-a"));
    }

    #[test]
    fn topology_can_see_same_group_always() {
        let t = TopologyConfig {
            links: vec![],
            routing_policy: vec![RoutingPolicy {
                from: "platform".to_string(),
                can_reach: vec!["*".to_string()],
            }],
        };
        // Same group is always visible regardless of rules
        assert!(t.can_see("customer-a", "customer-a"));
    }

    #[test]
    fn topology_can_see_partial_mesh() {
        let t = TopologyConfig {
            links: vec![],
            routing_policy: vec![
                RoutingPolicy {
                    from: "customer-a".to_string(),
                    can_reach: vec!["platform".to_string(), "customer-b".to_string()],
                },
                RoutingPolicy {
                    from: "customer-b".to_string(),
                    can_reach: vec!["platform".to_string()],
                },
                RoutingPolicy {
                    from: "platform".to_string(),
                    can_reach: vec!["*".to_string()],
                },
            ],
        };
        assert!(t.can_see("customer-a", "platform"));
        assert!(t.can_see("customer-a", "customer-b"));
        assert!(t.can_see("customer-b", "platform"));
        assert!(!t.can_see("customer-b", "customer-a"));
        assert!(t.can_see("platform", "customer-a"));
        assert!(t.can_see("platform", "customer-b"));
    }

    #[test]
    fn topology_hub_group_detection() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                LinkPolicy {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
            routing_policy: vec![],
        };
        assert_eq!(t.hub_group(), Some("platform"));
    }

    #[test]
    fn topology_hub_group_none_in_mesh() {
        let t = TopologyConfig {
            links: vec![LinkPolicy {
                name: "*".to_string(),
                peers: vec!["*".to_string()],
            }],
            routing_policy: vec![],
        };
        assert_eq!(t.hub_group(), None);
    }

    #[test]
    fn topology_is_star() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                LinkPolicy {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
            routing_policy: vec![],
        };
        assert!(t.is_star());
    }

    #[test]
    fn topology_is_not_star_when_spoke_has_extra_peers() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                LinkPolicy {
                    name: "customer-a".to_string(),
                    peers: vec!["platform".to_string(), "customer-b".to_string()],
                },
            ],
            routing_policy: vec![],
        };
        assert!(!t.is_star());
    }

    #[test]
    fn topology_validate_star_with_transit_ok() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "platform".to_string(),
                    peers: vec!["*".to_string()],
                },
                LinkPolicy {
                    name: "*".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
            routing_policy: vec![RoutingPolicy {
                from: "customer-a".to_string(),
                can_reach: vec!["customer-b".to_string()],
            }],
        };
        assert!(t.validate().is_ok());
    }

    #[test]
    fn topology_validate_non_star_no_link_fails() {
        let t = TopologyConfig {
            links: vec![
                LinkPolicy {
                    name: "customer-a".to_string(),
                    peers: vec!["platform".to_string()],
                },
                LinkPolicy {
                    name: "customer-b".to_string(),
                    peers: vec!["platform".to_string()],
                },
            ],
            routing_policy: vec![RoutingPolicy {
                from: "customer-a".to_string(),
                can_reach: vec!["customer-b".to_string()],
            }],
        };
        let err = t.validate().unwrap_err();
        assert!(matches!(
            err,
            crate::error::Error::NoLinkForVisibility { .. }
        ));
    }
}

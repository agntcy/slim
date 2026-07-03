// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use duration_string::DurationString;
use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};
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
    /// between node domains.
    pub topology: TopologyConfig,
    /// Optional authentication for node domain membership on registration.
    /// When absent, all registrations are accepted.
    pub registration_auth: Option<RegistrationAuthConfig>,
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
            registration_auth: None,
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

/// Topology configuration: controls link creation and route visibility
/// between node domains.
///
/// The topology mode is determined by which field is present in YAML:
/// - Neither `links` nor `segments` → **API-managed mode** (DB owns topology)
/// - `links` → config-managed, single routing domain with custom link graph
/// - `segments` → config-managed, multiple independent routing domains
/// - Both → deserialization error
///
/// # Examples
///
/// **API-managed mode (default):** no `topology` key or empty section.
/// Topology is built via gRPC/CLI at runtime.
///
/// ```yaml
/// topology: {}
/// ```
///
/// **Full mesh (config-managed):** use wildcard link entry.
///
/// ```yaml
/// topology:
///   links:
///     - domain: "*"
///       neighbors: ["*"]
/// ```
///
/// **Single segment with star topology:**
///
/// ```yaml
/// topology:
///   links:
///     - domain: hub
///       neighbors: [spoke-a, spoke-b]
/// ```
///
/// **Multiple segments with dynamic `$domain` expansion:**
///
/// ```yaml
/// topology:
///   segments:
///     - name: segment-$domain
///       links:
///         - group: platform
///           neighbors: [$domain]
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TopologyConfig {
    /// No topology configured: API-managed mode. The DB owns topology state
    /// and full CRUD operations are available via gRPC/CLI.
    #[default]
    ApiManaged,
    /// Single routing domain with a custom link graph (config-managed).
    Links(Vec<AdjacencyEntry>),
    /// Multiple independent routing domains, each with its own link graph (config-managed).
    Segments(Vec<SegmentConfig>),
}

/// A segment defines an independent routing domain.
/// Each segment has its own link graph and SPT computation.
///
/// The `name` and link entries can use `$domain` as a template variable.
/// When present, the segment is expanded at runtime into one concrete
/// segment per registered domain (excluding groups already named explicitly).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SegmentConfig {
    /// Segment name. May contain `$domain` for template expansion.
    pub name: String,
    /// Link graph within this segment.
    pub links: Vec<AdjacencyEntry>,
}

/// An adjacency list entry: nodes in the specified `group` connect to nodes
/// in any of the domains listed in `neighbors`. Links are bidirectional.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdjacencyEntry {
    /// Source domain name (or `"*"` to match any domain, or `$domain` for template expansion).
    pub domain: String,
    /// Groups this domain connects to. `"*"` matches any, `$domain` for template.
    pub neighbors: Vec<String>,
}

impl<'de> Deserialize<'de> for TopologyConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TopologyVisitor;

        impl<'de> Visitor<'de> for TopologyVisitor {
            type Value = TopologyConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a topology config with either 'links' or 'segments' (not both)")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut links: Option<Vec<AdjacencyEntry>> = None;
                let mut segments: Option<Vec<SegmentConfig>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "links" => {
                            if links.is_some() {
                                return Err(de::Error::duplicate_field("links"));
                            }
                            links = Some(map.next_value()?);
                        }
                        "segments" => {
                            if segments.is_some() {
                                return Err(de::Error::duplicate_field("segments"));
                            }
                            segments = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                match (links, segments) {
                    (Some(_), Some(_)) => Err(de::Error::custom(
                        "'links' and 'segments' are mutually exclusive in topology config",
                    )),
                    (Some(l), None) => {
                        if l.is_empty() {
                            Ok(TopologyConfig::ApiManaged)
                        } else {
                            Ok(TopologyConfig::Links(l))
                        }
                    }
                    (None, Some(s)) => {
                        if s.is_empty() {
                            Ok(TopologyConfig::ApiManaged)
                        } else {
                            Ok(TopologyConfig::Segments(s))
                        }
                    }
                    (None, None) => Ok(TopologyConfig::ApiManaged),
                }
            }
        }

        deserializer.deserialize_map(TopologyVisitor)
    }
}

impl TopologyConfig {
    /// Build one graph per segment. For Links returns a single "default" entry.
    /// For ApiManaged, returns an empty vec (topology is loaded from DB, not config).
    /// Wildcard `"*"` is expanded to all groups in `known_domains`.
    pub fn build_graph(
        &self,
        known_domains: &[&str],
    ) -> Vec<(String, petgraph::graph::UnGraph<String, u32>)> {
        self.expand_segments(known_domains)
            .iter()
            .map(|seg| {
                (
                    seg.name.clone(),
                    Self::build_graph_from_links(&seg.links, known_domains),
                )
            })
            .collect()
    }

    /// Returns `true` if this is API-managed mode (no config-driven topology).
    pub fn is_api_managed(&self) -> bool {
        matches!(self, Self::ApiManaged)
    }

    /// Returns `true` if topology is config-managed (links or segments defined).
    pub fn is_config_managed(&self) -> bool {
        !self.is_api_managed()
    }

    fn build_graph_from_links(
        links: &[AdjacencyEntry],
        known_domains: &[&str],
    ) -> petgraph::graph::UnGraph<String, u32> {
        use petgraph::graph::UnGraph;
        use std::collections::HashMap;

        let mut graph = UnGraph::<String, u32>::new_undirected();
        let mut indices: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();

        for &domain in known_domains {
            let idx = graph.add_node(domain.to_string());
            indices.insert(domain, idx);
        }

        for entry in links {
            let sources: Vec<&str> = if entry.domain == "*" {
                known_domains.to_vec()
            } else {
                known_domains
                    .iter()
                    .filter(|&&g| g == entry.domain)
                    .copied()
                    .collect()
            };

            for &src in &sources {
                for peer_pattern in &entry.neighbors {
                    let targets: Vec<&str> = if peer_pattern == "*" {
                        known_domains.to_vec()
                    } else {
                        known_domains
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

        graph
    }

    /// Returns true if this config uses `$domain` template expansion.
    pub fn has_domain_template(&self) -> bool {
        match self {
            Self::ApiManaged | Self::Links(_) => false,
            Self::Segments(segments) => segments.iter().any(|seg| seg.has_domain_template()),
        }
    }

    /// Expand `$domain` templates into concrete segments for the given groups.
    /// Groups already explicitly named in a template segment's links are excluded
    /// from expansion. Non-template segments pass through unchanged.
    pub fn expand_segments(&self, known_domains: &[&str]) -> Vec<SegmentConfig> {
        match self {
            Self::ApiManaged => vec![],
            Self::Links(links) => vec![SegmentConfig {
                name: "default".to_string(),
                links: links.clone(),
            }],
            Self::Segments(segments) => {
                let mut result = Vec::new();
                for seg in segments {
                    if seg.has_domain_template() {
                        // Find groups explicitly named (not templates/wildcards)
                        let explicit: Vec<&str> = seg
                            .links
                            .iter()
                            .flat_map(|e| {
                                let mut names = vec![];
                                if e.domain != "*" && !e.domain.contains("$domain") {
                                    names.push(e.domain.as_str());
                                }
                                for n in &e.neighbors {
                                    if n != "*" && !n.contains("$domain") {
                                        names.push(n.as_str());
                                    }
                                }
                                names
                            })
                            .collect();

                        // Expand for each domain NOT explicitly named
                        for &domain in known_domains {
                            if explicit.contains(&domain) {
                                continue;
                            }
                            result.push(seg.expand_for_domain(domain));
                        }
                    } else {
                        result.push(seg.clone());
                    }
                }
                result
            }
        }
    }
}

impl SegmentConfig {
    /// Returns true if this segment uses `$domain` in its name or links.
    pub fn has_domain_template(&self) -> bool {
        if self.name.contains("$domain") {
            return true;
        }
        self.links
            .iter()
            .any(|e| e.domain.contains("$domain") || e.neighbors.iter().any(|n| n.contains("$domain")))
    }

    /// Expand this template segment for a specific group value.
    /// Replaces all `$domain` occurrences with the concrete domain name.
    pub fn expand_for_domain(&self, domain: &str) -> SegmentConfig {
        SegmentConfig {
            name: self.name.replace("$domain", domain),
            links: self
                .links
                .iter()
                .map(|e| AdjacencyEntry {
                    domain: e.domain.replace("$domain", domain),
                    neighbors: e
                        .neighbors
                        .iter()
                        .map(|n| n.replace("$domain", domain))
                        .collect(),
                })
                .collect(),
        }
    }
}

/// Configuration for authenticating node domain membership on registration.
///
/// ```yaml
/// registration_auth:
///   type: shared_secret
///   secrets:
///     cluster-a: "secret-for-cluster-a-abcdefghi-1234567890"
///     cluster-b: "secret-for-cluster-b-abcdefghi-1234567890"
/// ```
///
/// Or for SPIRE (trust domain = domain name):
/// ```yaml
/// registration_auth:
///   type: spire
///   socket_path: "/run/spire/agent-sockets/api.sock"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RegistrationAuthConfig {
    /// Per-domain shared secret authentication.
    /// Map of domain name → secret string.
    SharedSecret {
        /// Map of domain name → shared secret value.
        secrets: HashMap<String, String>,
    },
    /// SPIRE-based authentication. Trust domain = domain name by convention.
    #[cfg(not(target_family = "windows"))]
    Spire {
        /// Path to the SPIRE agent socket for JWT SVID validation.
        socket_path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns true if `pattern` matches `domain`. `"*"` matches any domain.
    fn matches_domain(pattern: &str, domain: &str) -> bool {
        pattern == "*" || pattern == domain
    }

    impl TopologyConfig {
        /// Test helper: check if group `a` is allowed to link to group `b`.
        fn can_link(&self, a: &str, b: &str) -> bool {
            match self {
                // In API mode, config allows no links. Allowed pairs come from DB.
                Self::ApiManaged => false,
                Self::Links(links) => Self::can_link_in(links, a, b),
                Self::Segments(segments) => segments
                    .iter()
                    .any(|seg| Self::can_link_in(&seg.links, a, b)),
            }
        }

        fn can_link_in(links: &[AdjacencyEntry], a: &str, b: &str) -> bool {
            links.iter().any(|entry| {
                (matches_domain(&entry.domain, a)
                    && entry.neighbors.iter().any(|p| matches_domain(p, b)))
                    || (matches_domain(&entry.domain, b)
                        && entry.neighbors.iter().any(|p| matches_domain(p, a)))
            })
        }
    }

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
    fn topology_default_is_api_managed() {
        let t = TopologyConfig::default();
        assert!(t.is_api_managed());
        // ApiManaged allows no links from config — topology comes from DB.
        assert!(!t.can_link("a", "b"));
        assert!(!t.can_link("x", "y"));
    }

    #[test]
    fn topology_can_link_star() {
        let t = TopologyConfig::Links(vec![
            AdjacencyEntry {
                domain: "platform".to_string(),
                neighbors: vec!["*".to_string()],
            },
            AdjacencyEntry {
                domain: "*".to_string(),
                neighbors: vec!["platform".to_string()],
            },
        ]);
        assert!(t.can_link("platform", "customer-a"));
        assert!(t.can_link("customer-a", "platform"));
        assert!(!t.can_link("customer-a", "customer-b"));
    }

    #[test]
    fn topology_can_link_explicit_pair() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "node-a".to_string(),
            neighbors: vec!["node-b".to_string()],
        }]);
        // Bidirectional
        assert!(t.can_link("node-a", "node-b"));
        assert!(t.can_link("node-b", "node-a"));
        // No link to others
        assert!(!t.can_link("node-a", "node-c"));
        assert!(!t.can_link("node-b", "node-c"));
    }

    #[test]
    fn build_graph_full_mesh() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "*".to_string(),
            neighbors: vec!["*".to_string()],
        }]);
        let domains = vec!["a", "b", "c", "d"];
        let segments = t.build_graph(&domains);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].0, "default");
        let graph = &segments[0].1;
        assert_eq!(graph.node_count(), 4);
        // Full mesh with 4 nodes = 6 edges
        assert_eq!(graph.edge_count(), 6);
    }

    #[test]
    fn build_graph_star() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "hub".to_string(),
            neighbors: vec!["*".to_string()],
        }]);
        let domains = vec!["hub", "a", "b", "c"];
        let segments = t.build_graph(&domains);

        let graph = &segments[0].1;
        assert_eq!(graph.node_count(), 4);
        // Star: hub connects to a, b, c = 3 edges
        assert_eq!(graph.edge_count(), 3);
    }

    #[test]
    fn build_graph_chain() {
        let t = TopologyConfig::Links(vec![
            AdjacencyEntry {
                domain: "a".to_string(),
                neighbors: vec!["b".to_string()],
            },
            AdjacencyEntry {
                domain: "b".to_string(),
                neighbors: vec!["c".to_string()],
            },
            AdjacencyEntry {
                domain: "c".to_string(),
                neighbors: vec!["d".to_string()],
            },
        ]);
        let domains = vec!["a", "b", "c", "d"];
        let segments = t.build_graph(&domains);

        let graph = &segments[0].1;
        assert_eq!(graph.node_count(), 4);
        // Chain: a-b, b-c, c-d = 3 edges
        assert_eq!(graph.edge_count(), 3);
    }

    #[test]
    fn build_graph_no_self_links() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "*".to_string(),
            neighbors: vec!["*".to_string()],
        }]);
        let domains = vec!["a", "b"];
        let segments = t.build_graph(&domains);

        let graph = &segments[0].1;
        // 2 nodes, 1 edge (no self-links)
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn build_graph_no_duplicate_edges() {
        // Both entries create a↔b, but should only be 1 edge
        let t = TopologyConfig::Links(vec![
            AdjacencyEntry {
                domain: "a".to_string(),
                neighbors: vec!["b".to_string()],
            },
            AdjacencyEntry {
                domain: "b".to_string(),
                neighbors: vec!["a".to_string()],
            },
        ]);
        let domains = vec!["a", "b"];
        let segments = t.build_graph(&domains);

        assert_eq!(segments[0].1.edge_count(), 1);
    }

    #[test]
    fn build_graph_unknown_domain_ignored() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "a".to_string(),
            neighbors: vec!["unknown".to_string()],
        }]);
        let domains = vec!["a", "b"];
        let segments = t.build_graph(&domains);

        let graph = &segments[0].1;
        // "unknown" not in known_domains, so no edge created
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
    }

    // --- Deserialization tests ---

    #[test]
    fn deserialize_empty_topology_is_api_managed() {
        let t: TopologyConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(t, TopologyConfig::ApiManaged);
    }

    #[test]
    fn deserialize_links_topology() {
        let yaml = r#"
links:
  - domain: hub
    neighbors: ["*"]
"#;
        let t: TopologyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            t,
            TopologyConfig::Links(vec![AdjacencyEntry {
                domain: "hub".to_string(),
                neighbors: vec!["*".to_string()],
            }])
        );
    }

    #[test]
    fn deserialize_segments_topology() {
        let yaml = r#"
segments:
  - name: seg-$domain
    links:
      - domain: hub
        neighbors: [$domain]
"#;
        let t: TopologyConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(t, TopologyConfig::Segments(_)));
    }

    #[test]
    fn deserialize_both_links_and_segments_errors() {
        let yaml = r#"
links:
  - domain: hub
    neighbors: ["*"]
segments:
  - name: seg
    links:
      - domain: a
        neighbors: [b]
"#;
        let result: Result<TopologyConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    // --- $domain expansion tests ---

    #[test]
    fn segment_has_domain_template() {
        let seg = SegmentConfig {
            name: "seg-$domain".to_string(),
            links: vec![AdjacencyEntry {
                domain: "hub".to_string(),
                neighbors: vec!["$domain".to_string()],
            }],
        };
        assert!(seg.has_domain_template());

        let seg_no_template = SegmentConfig {
            name: "static-seg".to_string(),
            links: vec![AdjacencyEntry {
                domain: "a".to_string(),
                neighbors: vec!["b".to_string()],
            }],
        };
        assert!(!seg_no_template.has_domain_template());
    }

    #[test]
    fn segment_expand_for_domain() {
        let seg = SegmentConfig {
            name: "seg-$domain".to_string(),
            links: vec![AdjacencyEntry {
                domain: "hub".to_string(),
                neighbors: vec!["$domain".to_string()],
            }],
        };
        let expanded = seg.expand_for_domain("customer-a");
        assert_eq!(expanded.name, "seg-customer-a");
        assert_eq!(expanded.links[0].domain, "hub");
        assert_eq!(expanded.links[0].neighbors, vec!["customer-a"]);
    }

    #[test]
    fn expand_segments_star_isolation() {
        let t = TopologyConfig::Segments(vec![SegmentConfig {
            name: "seg-$domain".to_string(),
            links: vec![AdjacencyEntry {
                domain: "hub".to_string(),
                neighbors: vec!["$domain".to_string()],
            }],
        }]);

        let domains = vec!["hub", "customer-a", "customer-b"];
        let expanded = t.expand_segments(&domains);

        // hub is explicitly named in links, so only customer-a and customer-b expand
        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].name, "seg-customer-a");
        assert_eq!(expanded[1].name, "seg-customer-b");
    }

    #[test]
    fn expand_segments_no_template_passes_through() {
        let t = TopologyConfig::Segments(vec![SegmentConfig {
            name: "static".to_string(),
            links: vec![AdjacencyEntry {
                domain: "a".to_string(),
                neighbors: vec!["b".to_string()],
            }],
        }]);

        let domains = vec!["a", "b", "c"];
        let expanded = t.expand_segments(&domains);

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].name, "static");
    }

    #[test]
    fn expand_segments_mixed_template_and_static() {
        let t = TopologyConfig::Segments(vec![
            SegmentConfig {
                name: "seg-$domain".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["$domain".to_string()],
                }],
            },
            SegmentConfig {
                name: "shared".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["monitoring".to_string()],
                }],
            },
        ]);

        let domains = vec!["hub", "customer-a", "monitoring"];
        let expanded = t.expand_segments(&domains);

        // Template expands for customer-a and monitoring (only hub is explicit in template)
        // Plus the static segment
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].name, "seg-customer-a");
        assert_eq!(expanded[1].name, "seg-monitoring");
        assert_eq!(expanded[2].name, "shared");
    }

    // --- Segments build_graph tests ---

    #[test]
    fn build_graph_segments_returns_per_segment() {
        let t = TopologyConfig::Segments(vec![
            SegmentConfig {
                name: "seg-a".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["a".to_string()],
                }],
            },
            SegmentConfig {
                name: "seg-b".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["b".to_string()],
                }],
            },
        ]);

        let domains = vec!["hub", "a", "b"];
        let segment_graphs = t.build_graph(&domains);

        assert_eq!(segment_graphs.len(), 2);
        assert_eq!(segment_graphs[0].0, "seg-a");
        assert_eq!(segment_graphs[0].1.edge_count(), 1); // hub↔a
        assert_eq!(segment_graphs[1].0, "seg-b");
        assert_eq!(segment_graphs[1].1.edge_count(), 1); // hub↔b
    }

    #[test]
    fn can_link_segments_union() {
        let t = TopologyConfig::Segments(vec![
            SegmentConfig {
                name: "seg-a".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["a".to_string()],
                }],
            },
            SegmentConfig {
                name: "seg-b".to_string(),
                links: vec![AdjacencyEntry {
                    domain: "hub".to_string(),
                    neighbors: vec!["b".to_string()],
                }],
            },
        ]);

        assert!(t.can_link("hub", "a"));
        assert!(t.can_link("hub", "b"));
        // a and b not in any common segment link
        assert!(!t.can_link("a", "b"));
    }
}

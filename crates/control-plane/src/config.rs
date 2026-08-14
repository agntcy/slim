// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use duration_string::DurationString;
use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};
use std::time::Duration;

use slim_config::client::KeepaliveConfig;
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_tracing::TracingConfiguration;

/// CP-enforced connection parameters applied to all nodes on registration.
/// Any field set here overrides the node's local configuration.
/// Omitted fields leave the node's local settings untouched.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NodeConnectionParams {
    /// Fixed-interval backoff to enforce (milliseconds).
    pub backoff: Option<u32>,
    /// Connect timeout to enforce (milliseconds).
    pub timeout: Option<u32>,
    /// Keepalive settings to enforce.
    pub keepalive: Option<KeepaliveConfig>,
}

/// One or more gRPC listeners for a single API surface.
///
/// Accepts either a single server mapping or a sequence of them, so an existing
/// single-listener config keeps working unchanged:
///
/// ```yaml
/// northbound:                      # one listener
///   endpoint: "0.0.0.0:50051"
///   tls:
///     insecure: true
/// ```
///
/// ```yaml
/// northbound:                      # two listeners, independent TLS
///   - endpoint: "127.0.0.1:50051"
///     tls:
///       insecure: true
///   - endpoint: "0.0.0.0:50451"
///     tls:
///       insecure: false
///       source:
///         type: file
///         cert: /etc/slim/tls.crt
///         key: /etc/slim/tls.key
/// ```
///
/// Every listener serves the same service, so a deployment can expose one API on
/// several addresses at once — e.g. plaintext on loopback for local tooling and
/// mTLS on a routable address, or one listener per network interface.
#[derive(Debug, Clone)]
pub struct ServerConfigs(Vec<ServerConfig>);

impl ServerConfigs {
    /// Wrap a single server config.
    pub fn single(cfg: ServerConfig) -> Self {
        Self(vec![cfg])
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ServerConfig> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The configured endpoints, for logging.
    pub fn endpoints(&self) -> Vec<&str> {
        self.0.iter().map(|c| c.endpoint.as_str()).collect()
    }
}

impl<'a> IntoIterator for &'a ServerConfigs {
    type Item = &'a ServerConfig;
    type IntoIter = std::slice::Iter<'a, ServerConfig>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<ServerConfig> for ServerConfigs {
    fn from(cfg: ServerConfig) -> Self {
        Self::single(cfg)
    }
}

impl From<Vec<ServerConfig>> for ServerConfigs {
    fn from(cfgs: Vec<ServerConfig>) -> Self {
        Self(cfgs)
    }
}

/// Accepts a single server mapping or a sequence of them.
impl<'de> Deserialize<'de> for ServerConfigs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ServerConfigsVisitor;

        impl<'de> Visitor<'de> for ServerConfigsVisitor {
            type Value = ServerConfigs;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a server config mapping, or a sequence of them")
            }

            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let cfg = ServerConfig::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(ServerConfigs(vec![cfg]))
            }

            fn visit_seq<S>(self, seq: S) -> Result<Self::Value, S::Error>
            where
                S: de::SeqAccess<'de>,
            {
                let cfgs =
                    Vec::<ServerConfig>::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                if cfgs.is_empty() {
                    return Err(de::Error::custom(
                        "at least one server must be configured; remove the key to use the default \
                         endpoint instead of an empty list",
                    ));
                }
                Ok(ServerConfigs(cfgs))
            }
        }

        deserializer.deserialize_any(ServerConfigsVisitor)
    }
}

/// Top-level control-plane configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Northbound gRPC API (management / ControlPlaneService).
    /// One or more listeners; see [`ServerConfigs`].
    pub northbound: ServerConfigs,
    /// Southbound gRPC API (node registration / ControllerService).
    /// One or more listeners; see [`ServerConfigs`].
    pub southbound: ServerConfigs,
    /// Settings for the route and link reconcilers.
    pub reconciler: ReconcilerConfig,
    /// Database backend configuration.
    pub database: DatabaseConfig,
    /// Tracing / logging configuration.
    pub tracing: TracingConfiguration,
    /// Topology and auth configuration: controls link creation, route visibility
    /// between node domains, and optional node registration authentication.
    pub topology: TopologySettings,
    /// Optional connection parameters the CP enforces on all connecting nodes.
    #[serde(default, rename = "enforce_node_connection")]
    pub node_connection_params: NodeConnectionParams,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            northbound: ServerConfigs::single(ServerConfig {
                endpoint: "0.0.0.0:50051".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            }),
            southbound: ServerConfigs::single(ServerConfig {
                endpoint: "0.0.0.0:50052".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            }),
            reconciler: ReconcilerConfig::default(),
            database: DatabaseConfig::default(),
            tracing: TracingConfiguration::default(),
            topology: TopologySettings::default(),
            node_connection_params: NodeConnectionParams::default(),
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

/// Topology and authentication configuration.
///
/// Controls link creation, route visibility between node domains, and optional
/// node registration authentication.
///
/// The topology mode is determined by which field is present in YAML:
/// - Neither `links`, `segments`, nor `segments-template` → **API-managed mode** (DB owns topology)
/// - `links` → config-managed, single routing domain with custom link graph
/// - `segments` → config-managed, multiple independent routing domains
/// - `segments-template` → config-managed, rendered against registered domains
/// - More than one topology field → deserialization error
///
/// The optional `registration_auth` field configures registration authentication.
/// In API mode, shared secret domains are managed via gRPC (persisted in DB).
/// In config mode, secrets come from the file and CRUD APIs are rejected.
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
/// **API-managed with SPIRE auth:**
///
/// ```yaml
/// topology:
///   registration_auth:
///     type: spire
///     socket_path: "/run/spire/agent-sockets/api.sock"
/// ```
///
/// **Config-managed with shared secret auth:**
///
/// ```yaml
/// topology:
///   links:
///     - domain: "*"
///       neighbors: ["*"]
///   registration_auth:
///     type: shared_secret
///     secrets:
///       cluster-a: "secret-for-cluster-a"
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
///         - domain: platform
///           neighbors: [$domain]
/// ```
///
/// **Jinja-style segment template:**
///
/// ```yaml
/// topology:
///   segments-template: |
///     {% for group in groups %}
///     - name: {{ ("segment-" ~ group) | tojson }}
///       links:
///         - domain: platform
///           neighbors: [{{ group | tojson }}]
///     {% endfor %}
/// ```
/// Combined topology and registration auth settings.
#[derive(Debug, Clone, Default)]
pub struct TopologySettings {
    /// The topology link configuration (config vs API-managed).
    pub config: TopologyConfig,
    /// Optional registration auth configuration.
    pub auth: Option<RegistrationAuthConfig>,
}

impl TopologySettings {
    /// Returns `true` if topology is API-managed (no links/segments in config).
    pub fn is_api_managed(&self) -> bool {
        self.config.is_api_managed()
    }

    /// Returns `true` if topology is config-managed (links or segments defined).
    pub fn is_config_managed(&self) -> bool {
        self.config.is_config_managed()
    }
}

/// Topology link graph mode.
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
    /// A MiniJinja template rendered into segment definitions whenever the set
    /// of registered domains changes. The template receives `groups`, a sorted
    /// list of all registered domain names.
    SegmentsTemplate(String),
}

/// Failure while expanding a dynamic segment template.
#[derive(Debug, thiserror::Error)]
pub enum TopologyTemplateError {
    /// MiniJinja could not parse or render the configured template.
    #[error("failed to render segments-template: {0}")]
    Render(#[from] minijinja::Error),
    /// The rendered template was not a YAML list of segment definitions.
    #[error("segments-template rendered invalid YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// A segment defines an independent routing domain.
/// Each segment has its own link graph and SPT computation.
///
/// The `name` and link entries can use `$domain` as a template variable.
/// When present, the segment is expanded at runtime into one concrete
/// segment per registered domain (excluding domains already named explicitly).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SegmentConfig {
    /// Segment name. May contain `$domain` for template expansion.
    pub name: String,
    /// Link graph within this segment.
    pub links: Vec<AdjacencyEntry>,
}

/// An adjacency list entry: nodes in the specified `domain` connect to nodes
/// in any of the domains listed in `neighbors`. Links are bidirectional.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdjacencyEntry {
    /// Source domain name (or `"*"` to match any domain, or `$domain` for template expansion).
    #[serde(alias = "name")]
    pub domain: String,
    /// Groups this domain connects to. `"*"` matches any, `$domain` for template.
    pub neighbors: Vec<String>,
}

fn topology_config_from_parts<E: de::Error>(
    links: Option<Vec<AdjacencyEntry>>,
    segments: Option<Vec<SegmentConfig>>,
    segments_template: Option<String>,
) -> Result<TopologyConfig, E> {
    let configured_fields = usize::from(links.is_some())
        + usize::from(segments.is_some())
        + usize::from(segments_template.is_some());
    if configured_fields > 1 {
        return Err(E::custom(
            "'links', 'segments', and 'segments-template' are mutually exclusive in topology config",
        ));
    }

    if let Some(links) = links {
        return if links.is_empty() {
            Ok(TopologyConfig::ApiManaged)
        } else {
            Ok(TopologyConfig::Links(links))
        };
    }
    if let Some(segments) = segments {
        return if segments.is_empty() {
            Ok(TopologyConfig::ApiManaged)
        } else {
            Ok(TopologyConfig::Segments(segments))
        };
    }
    if let Some(template) = segments_template {
        if template.trim().is_empty() {
            return Err(E::custom("'segments-template' must not be empty"));
        }
        minijinja::Environment::new()
            .template_from_named_str("segments-template", &template)
            .map_err(E::custom)?;
        return Ok(TopologyConfig::SegmentsTemplate(template));
    }

    Ok(TopologyConfig::ApiManaged)
}

/// Custom deserializer for `TopologyConfig`: parses `links`, `segments`, or
/// `segments-template` keys.
/// Unknown keys are silently ignored.
impl<'de> Deserialize<'de> for TopologyConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TopologyConfigVisitor;

        impl<'de> Visitor<'de> for TopologyConfigVisitor {
            type Value = TopologyConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map with one of 'links', 'segments', or 'segments-template'")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut links: Option<Vec<AdjacencyEntry>> = None;
                let mut segments: Option<Vec<SegmentConfig>> = None;
                let mut segments_template: Option<String> = None;

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
                        "segments-template" => {
                            if segments_template.is_some() {
                                return Err(de::Error::duplicate_field("segments-template"));
                            }
                            segments_template = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                topology_config_from_parts(links, segments, segments_template)
            }
        }

        deserializer.deserialize_map(TopologyConfigVisitor)
    }
}

/// Custom deserializer for `TopologySettings`: combines `TopologyConfig`
/// (from `links`/`segments`/`segments-template` keys) with optional
/// `registration_auth` key.
impl<'de> Deserialize<'de> for TopologySettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TopologySettingsVisitor;

        impl<'de> Visitor<'de> for TopologySettingsVisitor {
            type Value = TopologySettings;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(
                    "a topology settings map with optional topology and 'registration_auth' keys",
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut links: Option<Vec<AdjacencyEntry>> = None;
                let mut segments: Option<Vec<SegmentConfig>> = None;
                let mut segments_template: Option<String> = None;
                let mut auth: Option<RegistrationAuthConfig> = None;

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
                        "segments-template" => {
                            if segments_template.is_some() {
                                return Err(de::Error::duplicate_field("segments-template"));
                            }
                            segments_template = Some(map.next_value()?);
                        }
                        "registration_auth" => {
                            if auth.is_some() {
                                return Err(de::Error::duplicate_field("registration_auth"));
                            }
                            auth = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                let config = topology_config_from_parts(links, segments, segments_template)?;

                Ok(TopologySettings { config, auth })
            }
        }

        deserializer.deserialize_map(TopologySettingsVisitor)
    }
}

impl TopologyConfig {
    /// Build one graph per segment. `Links` returns a single `default` entry,
    /// `ApiManaged` returns an empty vec, and `SegmentsTemplate` renders against
    /// the current domain set. Wildcard `"*"` expands to all known domains.
    pub fn build_graph(
        &self,
        known_domains: &[&str],
    ) -> Result<Vec<(String, petgraph::graph::UnGraph<String, u32>)>, TopologyTemplateError> {
        Ok(self
            .expand_segments(known_domains)?
            .into_iter()
            .map(|seg| {
                (
                    seg.name,
                    Self::build_graph_from_links(&seg.links, known_domains),
                )
            })
            .collect())
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

    /// Returns true if this config requires dynamic segment expansion.
    pub fn has_domain_template(&self) -> bool {
        match self {
            Self::ApiManaged | Self::Links(_) => false,
            Self::Segments(segments) => segments.iter().any(|seg| seg.has_domain_template()),
            Self::SegmentsTemplate(_) => true,
        }
    }

    /// Resolve segment configuration for the given domains.
    ///
    /// Legacy `$domain` segments are expanded once per non-explicit domain.
    /// MiniJinja `segments-template` values are rendered with the sorted domains
    /// exposed as `groups`. Non-template segments pass through unchanged.
    pub fn expand_segments(
        &self,
        known_domains: &[&str],
    ) -> Result<Vec<SegmentConfig>, TopologyTemplateError> {
        match self {
            Self::ApiManaged => Ok(vec![]),
            Self::Links(links) => Ok(vec![SegmentConfig {
                name: "default".to_string(),
                links: links.clone(),
            }]),
            Self::Segments(segments) => {
                let mut result = Vec::new();
                for seg in segments {
                    if seg.has_domain_template() {
                        // Find domains explicitly named (not templates/wildcards)
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
                Ok(result)
            }
            Self::SegmentsTemplate(template) => {
                let mut groups = known_domains.to_vec();
                groups.sort_unstable();

                let mut env = minijinja::Environment::new();
                env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
                let rendered = env
                    .template_from_named_str("segments-template", template)?
                    .render(minijinja::context! { groups => &groups })?;
                if rendered.trim().is_empty() {
                    Ok(vec![])
                } else {
                    Ok(serde_yaml::from_str(&rendered)?)
                }
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
        self.links.iter().any(|e| {
            e.domain.contains("$domain") || e.neighbors.iter().any(|n| n.contains("$domain"))
        })
    }

    /// Expand this template segment for a specific domain value.
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
/// Nested under the `topology.registration_auth` key:
/// ```yaml
/// topology:
///   registration_auth:
///     type: shared_secret
///     secrets:
///       cluster-a: "secret-for-cluster-a-abcdefghi-1234567890"
///       cluster-b: "secret-for-cluster-b-abcdefghi-1234567890"
/// ```
///
/// Or for SPIRE (trust domain = domain name):
/// ```yaml
/// topology:
///   registration_auth:
///     type: spire
///     socket_path: "/run/spire/agent-sockets/api.sock"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RegistrationAuthConfig {
    /// Per-domain shared secret authentication.
    /// In config mode, secrets are read from this map.
    /// In API mode, this map may be empty — secrets are managed via gRPC.
    SharedSecret {
        /// Map of domain name → shared secret value.
        #[serde(default)]
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
        /// Test helper: check if domain `a` is allowed to link to domain `b`.
        fn can_link(&self, a: &str, b: &str) -> bool {
            match self {
                // In API mode, config allows no links. Allowed pairs come from DB.
                Self::ApiManaged => false,
                Self::Links(links) => Self::can_link_in(links, a, b),
                Self::Segments(segments) => segments
                    .iter()
                    .any(|seg| Self::can_link_in(&seg.links, a, b)),
                Self::SegmentsTemplate(_) => self.expand_segments(&[a, b]).is_ok_and(|segments| {
                    segments
                        .iter()
                        .any(|seg| Self::can_link_in(&seg.links, a, b))
                }),
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
        assert_eq!(c.northbound.endpoints(), vec!["0.0.0.0:50051"]);
        assert_eq!(c.southbound.endpoints(), vec!["0.0.0.0:50052"]);
        assert_eq!(c.topology.config, TopologyConfig::default());
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
        let segments = t.build_graph(&domains).unwrap();

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
        let segments = t.build_graph(&domains).unwrap();

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
        let segments = t.build_graph(&domains).unwrap();

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
        let segments = t.build_graph(&domains).unwrap();

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
        let segments = t.build_graph(&domains).unwrap();

        assert_eq!(segments[0].1.edge_count(), 1);
    }

    #[test]
    fn build_graph_unknown_group_ignored() {
        let t = TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "a".to_string(),
            neighbors: vec!["unknown".to_string()],
        }]);
        let domains = vec!["a", "b"];
        let segments = t.build_graph(&domains).unwrap();

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
    fn deserialize_segments_template_topology() {
        let yaml = r#"
segments-template: |
  {% for group in groups %}
  - name: segment-{{ group }}
    links:
      - name: cloud
        neighbors: [{{ group }}]
  {% endfor %}
"#;
        let topology: TopologyConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(topology, TopologyConfig::SegmentsTemplate(_)));

        let settings: TopologySettings = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(
            settings.config,
            TopologyConfig::SegmentsTemplate(_)
        ));
    }

    #[test]
    fn deserialize_invalid_segments_template_errors() {
        let yaml = r#"
segments-template: |
  {% for group in groups %}
"#;
        let error = serde_yaml::from_str::<TopologyConfig>(yaml).unwrap_err();
        assert!(error.to_string().contains("unexpected end of input"));
    }

    #[test]
    fn deserialize_segments_template_with_segments_errors() {
        let yaml = r#"
segments: []
segments-template: "[]"
"#;
        let error = serde_yaml::from_str::<TopologyConfig>(yaml).unwrap_err();
        assert!(error.to_string().contains("mutually exclusive"));
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
        let expanded = t.expand_segments(&domains).unwrap();

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
        let expanded = t.expand_segments(&domains).unwrap();

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
        let expanded = t.expand_segments(&domains).unwrap();

        // Template expands for customer-a and monitoring (only hub is explicit in template)
        // Plus the static segment
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].name, "seg-customer-a");
        assert_eq!(expanded[1].name, "seg-monitoring");
        assert_eq!(expanded[2].name, "shared");
    }

    // --- segments-template expansion tests ---

    #[test]
    fn segments_template_expands_once_per_group_in_sorted_order() {
        let topology = TopologyConfig::SegmentsTemplate(
            r#"
{% for group in groups %}
- name: segment-{{ group }}
  links:
    - name: cloud
      neighbors: [{{ group }}]
{% endfor %}
"#
            .to_string(),
        );

        let expanded = topology
            .expand_segments(&["customer-b", "customer-a"])
            .unwrap();
        assert_eq!(
            expanded
                .iter()
                .map(|segment| segment.name.as_str())
                .collect::<Vec<_>>(),
            vec!["segment-customer-a", "segment-customer-b"]
        );
        assert_eq!(expanded[0].links[0].domain, "cloud");
        assert_eq!(expanded[0].links[0].neighbors, ["customer-a"]);
    }

    #[test]
    fn segments_template_supports_prefix_filtering() {
        let topology = TopologyConfig::SegmentsTemplate(
            r#"
- name: segment-customer-a
  links:
{% for group in groups %}
{% if group is startingwith("customer-a-") %}
    - name: {{ group }}
      neighbors: [customer-a]
{% endif %}
{% endfor %}
"#
            .to_string(),
        );

        let expanded = topology
            .expand_segments(&[
                "customer-b-worker",
                "customer-a-worker-2",
                "customer-a",
                "customer-a-worker-1",
            ])
            .unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].links.len(), 2);
        assert_eq!(expanded[0].links[0].domain, "customer-a-worker-1");
        assert_eq!(expanded[0].links[1].domain, "customer-a-worker-2");
    }

    #[test]
    fn segments_template_empty_render_is_an_empty_topology() {
        let topology = TopologyConfig::SegmentsTemplate(
            "{% for group in groups %}- name: {{ group }}\n  links: []\n{% endfor %}".to_string(),
        );

        assert!(topology.expand_segments(&[]).unwrap().is_empty());
    }

    #[test]
    fn segments_template_tojson_safely_quotes_group_names() {
        let topology = TopologyConfig::SegmentsTemplate(
            r#"
{% for group in groups %}
- name: {{ ("segment-" ~ group) | tojson }}
  links:
    - domain: cloud
      neighbors: [{{ group | tojson }}]
{% endfor %}
"#
            .to_string(),
        );

        let expanded = topology
            .expand_segments(&["customer: [unexpected]"])
            .unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].name, "segment-customer: [unexpected]");
        assert_eq!(expanded[0].links[0].neighbors, ["customer: [unexpected]"]);
    }

    #[test]
    fn segments_template_reports_rendered_yaml_errors() {
        let topology = TopologyConfig::SegmentsTemplate("not-a-segment-list: true".to_string());
        let error = topology.expand_segments(&["customer-a"]).unwrap_err();
        assert!(error.to_string().contains("rendered invalid YAML"));
    }

    #[test]
    fn segments_template_uses_strict_undefined_variables() {
        let topology = TopologyConfig::SegmentsTemplate(
            "- name: segment-{{ missing_group }}\n  links: []".to_string(),
        );
        let error = topology.expand_segments(&["customer-a"]).unwrap_err();
        assert!(error.to_string().contains("undefined value"));
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
        let segment_graphs = t.build_graph(&domains).unwrap();

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

    // --- ServerConfigs deserialization ---

    /// The pre-existing single-mapping shape must keep working untouched.
    #[test]
    fn deserialize_single_server_mapping() {
        let yaml = r#"
endpoint: "127.0.0.1:50051"
tls:
  insecure: true
"#;
        let s: ServerConfigs = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.endpoints(), vec!["127.0.0.1:50051"]);
    }

    #[test]
    fn deserialize_server_sequence() {
        let yaml = r#"
- endpoint: "127.0.0.1:50051"
  tls:
    insecure: true
- endpoint: "0.0.0.0:50451"
  tls:
    insecure: true
"#;
        let s: ServerConfigs = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.endpoints(), vec!["127.0.0.1:50051", "0.0.0.0:50451"]);
    }

    /// Per-listener settings must be independent, not copied from the first.
    #[test]
    fn deserialize_server_sequence_keeps_per_listener_tls() {
        let yaml = r#"
- endpoint: "127.0.0.1:50051"
  tls:
    insecure: true
- endpoint: "0.0.0.0:50451"
  tls:
    insecure: false
"#;
        let s: ServerConfigs = serde_yaml::from_str(yaml).unwrap();
        let listeners: Vec<&ServerConfig> = s.iter().collect();
        assert!(listeners[0].tls_setting.insecure);
        assert!(!listeners[1].tls_setting.insecure);
    }

    /// An empty list would silently leave the API unreachable.
    #[test]
    fn deserialize_empty_server_sequence_errors() {
        let err = serde_yaml::from_str::<ServerConfigs>("[]").unwrap_err();
        assert!(
            err.to_string().contains("at least one server"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deserialize_server_scalar_errors() {
        assert!(serde_yaml::from_str::<ServerConfigs>("\"127.0.0.1:50051\"").is_err());
    }

    #[test]
    fn full_config_accepts_single_and_multiple_together() {
        let yaml = r#"
northbound:
  endpoint: "127.0.0.1:50051"
  tls:
    insecure: true
southbound:
  - endpoint: "127.0.0.1:50052"
    tls:
      insecure: true
  - endpoint: "127.0.0.1:50053"
    tls:
      insecure: true
"#;
        let c: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(c.northbound.endpoints(), vec!["127.0.0.1:50051"]);
        assert_eq!(
            c.southbound.endpoints(),
            vec!["127.0.0.1:50052", "127.0.0.1:50053"]
        );
    }

    /// Omitting a key falls back to the single default listener.
    #[test]
    fn full_config_omitted_bounds_use_defaults() {
        let c: Config = serde_yaml::from_str("tracing:\n  log_level: debug\n").unwrap();
        assert_eq!(c.northbound.endpoints(), vec!["0.0.0.0:50051"]);
        assert_eq!(c.southbound.endpoints(), vec!["0.0.0.0:50052"]);
    }

    #[test]
    fn server_configs_iter_matches_into_iter() {
        let s = ServerConfigs::from(vec![
            ServerConfig::with_endpoint("a:1"),
            ServerConfig::with_endpoint("b:2"),
        ]);
        assert!(!s.is_empty());
        let via_iter: Vec<&str> = s.iter().map(|c| c.endpoint.as_str()).collect();
        let via_into: Vec<&str> = (&s).into_iter().map(|c| c.endpoint.as_str()).collect();
        assert_eq!(via_iter, via_into);
        assert_eq!(via_iter, vec!["a:1", "b:2"]);
    }

    /// The exact YAML the Helm chart emits for a single-listener bound, so a
    /// drift on either side is caught here rather than at deploy time.
    #[test]
    fn parses_chart_rendered_single_listener() {
        let yaml = r#"
database:
  path: /db/controlplane.db
  type: sqlite
reconciler:
  max_requeues: 15
  workers: 4
topology: {}
tracing:
  log_level: debug
northbound:
  endpoint: 0.0.0.0:50051
  tls:
    insecure: true
southbound:
  endpoint: 0.0.0.0:50052
  tls:
    insecure: true
"#;
        let c: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(c.northbound.endpoints(), vec!["0.0.0.0:50051"]);
        assert_eq!(c.southbound.endpoints(), vec!["0.0.0.0:50052"]);
    }

    /// The chart's multi-listener rendering, including an unquoted `host:port`
    /// scalar and per-listener TLS.
    #[test]
    fn parses_chart_rendered_multiple_listeners() {
        let yaml = r#"
tracing:
  log_level: debug
northbound:
  - endpoint: 0.0.0.0:50051
    tls:
      insecure: true
  - endpoint: 0.0.0.0:50451
    tls:
      insecure: false
southbound:
  - endpoint: 0.0.0.0:50052
    tls:
      insecure: true
  - endpoint: 0.0.0.0:50053
    tls:
      insecure: true
"#;
        let c: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            c.northbound.endpoints(),
            vec!["0.0.0.0:50051", "0.0.0.0:50451"]
        );
        let nb: Vec<&ServerConfig> = c.northbound.iter().collect();
        assert!(nb[0].tls_setting.insecure);
        assert!(!nb[1].tls_setting.insecure);
        assert_eq!(
            c.southbound.endpoints(),
            vec!["0.0.0.0:50052", "0.0.0.0:50053"]
        );
    }

    /// An empty document must not silently yield defaults in a way that hides a
    /// broken render — it deserializes to the documented defaults, and callers
    /// that care about real content should assert on it.
    #[test]
    fn empty_document_yields_defaults() {
        let c: Config = serde_yaml::from_str("{}").unwrap();
        assert_eq!(c.northbound.endpoints(), vec!["0.0.0.0:50051"]);
    }

    /// The TLS shapes used in this module's docs and in the chart must be the
    /// ones the deserializer actually understands. `TlsServerConfig` silently
    /// ignores unknown keys, so a wrong field name here yields TLS enabled with
    /// no certificate source rather than a config error — worth pinning.
    #[test]
    fn documented_tls_forms_populate_a_source() {
        use slim_config::tls::common::TlsSource;

        let file_form = r#"
northbound:
  - endpoint: "0.0.0.0:50451"
    tls:
      insecure: false
      source:
        type: file
        cert: /etc/slim/tls.crt
        key: /etc/slim/tls.key
"#;
        let c: Config = serde_yaml::from_str(file_form).unwrap();
        let nb: Vec<&ServerConfig> = c.northbound.iter().collect();
        assert!(!nb[0].tls_setting.insecure);
        assert!(
            matches!(nb[0].tls_setting.config.source, TlsSource::File { .. }),
            "expected a file source, got {:?}",
            nb[0].tls_setting.config.source
        );

        #[cfg(not(target_family = "windows"))]
        {
            let spire_form = r#"
northbound:
  - endpoint: "0.0.0.0:50451"
    tls:
      insecure: false
      source:
        type: spire
        socket_path: "unix:///run/spire/agent-sockets/api.sock"
"#;
            let c: Config = serde_yaml::from_str(spire_form).unwrap();
            let nb: Vec<&ServerConfig> = c.northbound.iter().collect();
            assert!(
                matches!(nb[0].tls_setting.config.source, TlsSource::Spire { .. }),
                "expected a spire source, got {:?}",
                nb[0].tls_setting.config.source
            );
        }
    }

    /// Guards the failure mode that made the bad docs dangerous: an unknown key
    /// under `tls` is dropped, leaving TLS on with no source.
    #[test]
    fn unknown_tls_key_is_silently_ignored() {
        use slim_config::tls::common::TlsSource;

        let yaml = r#"
northbound:
  - endpoint: "0.0.0.0:50451"
    tls:
      insecure: false
      useSpiffe: true
"#;
        let c: Config = serde_yaml::from_str(yaml).unwrap();
        let nb: Vec<&ServerConfig> = c.northbound.iter().collect();
        assert!(!nb[0].tls_setting.insecure);
        assert!(
            matches!(nb[0].tls_setting.config.source, TlsSource::None),
            "an unknown key must not somehow configure a source"
        );
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration types for peer discovery.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use slim_config::client::ClientConfig;

/// Topology for peer-to-peer connections within a replica set.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeerTopology {
    /// Every replica connects to every other replica (N*(N-1)/2 connections).
    /// Subscriptions are forwarded 1 hop.
    #[default]
    FullMesh,
    /// One replica (the hub, determined by smallest lexicographic node_id)
    /// connects to all others (spokes). The hub relays subscriptions and
    /// data messages between spokes.
    HubAndSpoke,
}

/// A single static peer entry pairing a node identity with connection config.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StaticPeerEntry {
    /// Unique node identifier of the remote peer.
    pub node_id: String,

    /// Client connection configuration (endpoint, TLS, auth, etc.).
    #[serde(flatten)]
    pub config: ClientConfig,
}

/// Top-level peer configuration.
///
/// When present in the service configuration, enables peer-to-peer route
/// synchronization between SLIM replicas.
///
/// Static peers are listed directly in this config section. Each entry
/// requires a `node_id` (the remote peer's unique identity) alongside the
/// usual connection settings (`endpoint`, TLS, auth, etc.).
///
/// # Example (static peers)
/// ```yaml
/// peers:
///   deployment_name: "my-deployment"
///   static_peers:
///     - node_id: "slim-1"
///       endpoint: "slim-1:8080"
///     - node_id: "slim-2"
///       endpoint: "slim-2:8080"
///       tls_setting:
///         insecure: true
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    /// Shared group identity for mutual peer authentication during link negotiation.
    /// Peers must have the same `deployment_name` to accept each other.
    pub deployment_name: String,

    /// Topology for peer connections. Defaults to `FullMesh`.
    #[serde(default)]
    pub topology: PeerTopology,

    /// Static list of peer connections. Each entry requires a `node_id` plus
    /// the connection configuration fields (flattened from `ClientConfig`).
    /// The `connection_type` field is forced to `Peer` regardless of what is set.
    /// Must contain at least one entry.
    #[serde(deserialize_with = "deserialize_non_empty_peers")]
    pub static_peers: Vec<StaticPeerEntry>,

    /// Optional dynamic discovery backend (e.g., Kubernetes).
    /// When absent and `static_peers` is non-empty, only static discovery is used.
    #[serde(default)]
    pub discovery: Option<PeerDiscoveryConfig>,
}

/// Deserialize `static_peers` and reject empty lists.
fn deserialize_non_empty_peers<'de, D>(deserializer: D) -> Result<Vec<StaticPeerEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    let peers = Vec::<StaticPeerEntry>::deserialize(deserializer)?;
    if peers.is_empty() {
        return Err(D::Error::custom("static_peers must not be empty"));
    }
    Ok(peers)
}

/// Dynamic discovery backend configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum PeerDiscoveryConfig {
    /// Kubernetes-based peer discovery (watches pods by label selector).
    #[serde(rename = "kubernetes")]
    Kubernetes {
        /// Kubernetes namespace to watch.
        namespace: String,
        /// Label selector to filter peer pods (e.g., "app=slim").
        label_selector: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_peer_config_static_peers() {
        let yaml = r#"
            deployment_name: "my-deployment"
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
              - node_id: "slim-2"
                endpoint: "http://slim-2:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.deployment_name, "my-deployment");
        assert_eq!(config.static_peers.len(), 2);
        assert_eq!(config.static_peers[0].node_id, "slim-1");
        assert_eq!(config.static_peers[0].config.endpoint, "http://slim-1:8080");
        assert_eq!(config.static_peers[1].node_id, "slim-2");
        assert_eq!(config.static_peers[1].config.endpoint, "http://slim-2:8080");
        assert!(config.discovery.is_none());
    }

    #[test]
    fn test_deserialize_peer_config_no_peers_fails() {
        let yaml = r#"
            deployment_name: "my-deployment"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "static_peers is required");
    }

    #[test]
    fn test_deserialize_peer_config_empty_peers_fails() {
        let yaml = r#"
            deployment_name: "my-deployment"
            static_peers: []
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "static_peers must not be empty");
    }

    #[test]
    fn test_deserialize_kubernetes_config() {
        let yaml = r#"
            deployment_name: "slim-deployment"
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
            discovery:
              type: kubernetes
              namespace: "default"
              label_selector: "app=slim"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.deployment_name, "slim-deployment");

        match &config.discovery {
            Some(PeerDiscoveryConfig::Kubernetes {
                namespace,
                label_selector,
            }) => {
                assert_eq!(namespace, "default");
                assert_eq!(label_selector, "app=slim");
            }
            None => panic!("expected kubernetes config"),
        }
    }

    #[test]
    fn test_reject_unknown_fields() {
        let yaml = r#"
            deployment_name: "group"
            unknown_field: "oops"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_topology_defaults_to_full_mesh() {
        let yaml = r#"
            deployment_name: "my-deployment"
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::FullMesh);
    }

    #[test]
    fn test_deserialize_hub_and_spoke_topology() {
        let yaml = r#"
            deployment_name: "my-deployment"
            topology: hub_and_spoke
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
              - node_id: "slim-2"
                endpoint: "http://slim-2:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::HubAndSpoke);
        assert_eq!(config.static_peers.len(), 2);
    }

    #[test]
    fn test_deserialize_full_mesh_topology_explicit() {
        let yaml = r#"
            deployment_name: "my-deployment"
            topology: full_mesh
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::FullMesh);
    }
}

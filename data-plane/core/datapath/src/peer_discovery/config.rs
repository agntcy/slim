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
/// # Example (static peers)
/// ```yaml
/// peers:
///   peer_group: "my-deployment"
///   discovery:
///     type: static_peers
///     peers:
///       - node_id: "slim-1"
///         endpoint: "slim-1:8080"
///       - node_id: "slim-2"
///         endpoint: "slim-2:8080"
/// ```
///
/// # Example (kubernetes)
/// ```yaml
/// peers:
///   peer_group: "my-deployment"
///   discovery:
///     type: kubernetes
///     namespace: "default"
///     label_selector: "app=slim"
///     port: 8080
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    /// Shared group identity for mutual peer authentication during link negotiation.
    /// Peers must have the same `peer_group` to accept each other.
    pub peer_group: String,

    /// Topology for peer connections. Defaults to `FullMesh`.
    #[serde(default)]
    pub topology: PeerTopology,

    /// Discovery backend for finding peer replicas.
    pub discovery: PeerDiscoveryConfig,
}

/// Peer discovery backend configuration.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum PeerDiscoveryConfig {
    /// Static list of peer connections defined in configuration.
    #[serde(rename = "static_peers")]
    StaticPeers {
        /// List of peer entries. Each requires a `node_id` plus connection config.
        /// The `connection_type` is forced to `Peer` regardless of what is set.
        #[serde(deserialize_with = "deserialize_non_empty_peers")]
        peers: Vec<StaticPeerEntry>,
    },

    /// Kubernetes-based peer discovery (watches pods by label selector).
    #[serde(rename = "kubernetes")]
    Kubernetes {
        /// Kubernetes namespace to watch.
        namespace: String,
        /// Label selector to filter peer pods (e.g., "app=slim").
        label_selector: String,
        /// Port number on which peer pods listen for dataplane connections.
        port: u16,
    },
}

/// Deserialize peers and reject empty lists.
fn deserialize_non_empty_peers<'de, D>(deserializer: D) -> Result<Vec<StaticPeerEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    let peers = Vec::<StaticPeerEntry>::deserialize(deserializer)?;
    if peers.is_empty() {
        return Err(D::Error::custom("peers must not be empty"));
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_static_peers() {
        let yaml = r#"
            peer_group: "my-deployment"
            discovery:
              type: static_peers
              peers:
                - node_id: "slim-1"
                  endpoint: "http://slim-1:8080"
                - node_id: "slim-2"
                  endpoint: "http://slim-2:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.peer_group, "my-deployment");
        match &config.discovery {
            PeerDiscoveryConfig::StaticPeers { peers } => {
                assert_eq!(peers.len(), 2);
                assert_eq!(peers[0].node_id, "slim-1");
                assert_eq!(peers[0].config.endpoint, "http://slim-1:8080");
                assert_eq!(peers[1].node_id, "slim-2");
                assert_eq!(peers[1].config.endpoint, "http://slim-2:8080");
            }
            _ => panic!("expected static_peers"),
        }
    }

    #[test]
    fn test_deserialize_empty_peers_fails() {
        let yaml = r#"
            peer_group: "my-deployment"
            discovery:
              type: static_peers
              peers: []
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "peers must not be empty");
    }

    #[test]
    fn test_deserialize_peer_config_no_peers_fails() {
        let yaml = r#"
            peer_group: "my-deployment"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "static_peers is required");
    }

    #[test]
    fn test_deserialize_peer_config_empty_peers_fails() {
        let yaml = r#"
            peer_group: "my-deployment"
            static_peers: []
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "static_peers must not be empty");
    }

    #[test]
    fn test_deserialize_kubernetes_config() {
        let yaml = r#"
            peer_group: "slim-deployment"
            static_peers:
              - node_id: "slim-1"
                endpoint: "http://slim-1:8080"
            discovery:
              type: kubernetes
              namespace: "default"
              label_selector: "app=slim"
              port: 8080
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.peer_group, "slim-deployment");

        match &config.discovery {
            PeerDiscoveryConfig::Kubernetes {
                namespace,
                label_selector,
                port,
            } => {
                assert_eq!(namespace, "default");
                assert_eq!(label_selector, "app=slim");
                assert_eq!(*port, 8080);
            }
            _ => panic!("expected kubernetes config"),
        }
    }

    #[test]
    fn test_missing_discovery_fails() {
        let yaml = r#"
            peer_group: "my-deployment"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "discovery is required");
    }

    #[test]
    fn test_reject_unknown_fields() {
        let yaml = r#"
            peer_group: "group"
            unknown_field: "oops"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_topology_defaults_to_full_mesh() {
        let yaml = r#"
            peer_group: "my-deployment"
            discovery:
              type: static_peers
              peers:
                - node_id: "slim-1"
                  endpoint: "http://slim-1:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::FullMesh);
    }

    #[test]
    fn test_deserialize_hub_and_spoke_topology() {
        let yaml = r#"
            peer_group: "my-deployment"
            topology: hub_and_spoke
            discovery:
              type: static_peers
              peers:
                - node_id: "slim-1"
                  endpoint: "http://slim-1:8080"
                - node_id: "slim-2"
                  endpoint: "http://slim-2:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::HubAndSpoke);
    }

    #[test]
    fn test_deserialize_full_mesh_topology_explicit() {
        let yaml = r#"
            peer_group: "my-deployment"
            topology: full_mesh
            discovery:
              type: static_peers
              peers:
                - node_id: "slim-1"
                  endpoint: "http://slim-1:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.topology, PeerTopology::FullMesh);
    }
}

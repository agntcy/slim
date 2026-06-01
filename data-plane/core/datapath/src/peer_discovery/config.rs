// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration types for peer discovery.

use serde::Deserialize;
use slim_config::client::ClientConfig;

/// Top-level peer configuration.
///
/// When present in the service configuration, enables peer-to-peer route
/// synchronization between SLIM replicas.
///
/// Static peers are listed directly in this config section (each with a full
/// `ClientConfig`). For dynamic discovery (e.g., Kubernetes), set the
/// `discovery` field.
///
/// The peer's unique identity is derived from the service's `node_id`
/// (which defaults to a UUID when not configured).
///
/// # Example (static peers)
/// ```yaml
/// peers:
///   peer_group: "my-deployment"
///   static_peers:
///     - endpoint: "slim-1:8080"
///     - endpoint: "slim-2:8080"
///       tls_setting:
///         insecure: true
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    /// Shared group identity for mutual peer authentication during link negotiation.
    /// Peers must have the same `peer_group` to accept each other.
    pub peer_group: String,

    /// Static list of peer connections. Each entry is a full `ClientConfig`,
    /// allowing per-peer TLS, auth, keepalive, etc.
    /// The `connection_type` field is forced to `Peer` regardless of what is set.
    #[serde(default)]
    pub static_peers: Vec<ClientConfig>,

    /// Optional dynamic discovery backend (e.g., Kubernetes).
    /// When absent and `static_peers` is non-empty, only static discovery is used.
    #[serde(default)]
    pub discovery: Option<PeerDiscoveryConfig>,
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
            peer_group: "my-deployment"
            static_peers:
              - endpoint: "http://slim-1:8080"
              - endpoint: "http://slim-2:8080"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.peer_group, "my-deployment");
        assert_eq!(config.static_peers.len(), 2);
        assert_eq!(config.static_peers[0].endpoint, "http://slim-1:8080");
        assert_eq!(config.static_peers[1].endpoint, "http://slim-2:8080");
        assert!(config.discovery.is_none());
    }

    #[test]
    fn test_deserialize_peer_config_no_peers() {
        let yaml = r#"
            peer_group: "my-deployment"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.static_peers.is_empty());
        assert!(config.discovery.is_none());
    }

    #[test]
    fn test_deserialize_kubernetes_config() {
        let yaml = r#"
            peer_group: "slim-deployment"
            discovery:
              type: kubernetes
              namespace: "default"
              label_selector: "app=slim"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.peer_group, "slim-deployment");

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
            peer_group: "group"
            unknown_field: "oops"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }
}

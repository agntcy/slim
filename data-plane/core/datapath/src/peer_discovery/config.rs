// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration types for peer discovery.

use serde::Deserialize;

/// Top-level peer configuration.
///
/// When present in the service configuration, enables peer-to-peer route
/// synchronization between SLIM replicas.
///
/// For static peer discovery, peers are defined as `dataplane.clients` with
/// `connection_type: peer`. No additional configuration is needed here.
///
/// For dynamic discovery (e.g., Kubernetes), set the `discovery` field.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    /// Unique identifier for this replica within the peer group.
    /// Used for self-filtering in dynamic discovery and leader election.
    pub self_id: String,

    /// Shared group identity for mutual peer authentication during link negotiation.
    /// Peers must have the same `peer_group` to accept each other.
    pub peer_group: String,

    /// Optional dynamic discovery backend (e.g., Kubernetes).
    /// When absent, peers are discovered statically from `dataplane.clients`
    /// that have `connection_type: peer`.
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
    fn test_deserialize_peer_config_no_discovery() {
        let yaml = r#"
            self_id: "slim-0"
            peer_group: "my-deployment"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.self_id, "slim-0");
        assert_eq!(config.peer_group, "my-deployment");
        assert!(config.discovery.is_none());
    }

    #[test]
    fn test_deserialize_kubernetes_config() {
        let yaml = r#"
            self_id: "slim-pod-abc"
            peer_group: "slim-deployment"
            discovery:
              type: kubernetes
              namespace: "default"
              label_selector: "app=slim"
        "#;

        let config: PeerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.self_id, "slim-pod-abc");
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
            self_id: "slim-0"
            peer_group: "group"
            unknown_field: "oops"
        "#;

        let result: Result<PeerConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }
}

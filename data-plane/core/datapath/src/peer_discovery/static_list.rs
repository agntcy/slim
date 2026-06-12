// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Static peer discovery implementation.
//!
//! Discovers peers from a pre-built list of [`PeerInfo`] entries (typically
//! derived from `PeerConfig.static_peers`).
//! Emits `Joined` events for all entries on startup. Never emits `Left`
//! events since static peers are assumed to be always available.

use std::collections::VecDeque;

use slim_config::client::ConnType;

use super::config::StaticPeerEntry;
use super::{PeerDiscovery, PeerDiscoveryError, PeerEvent, PeerInfo};

/// Static peer discovery: all peers are known at configuration time.
pub struct StaticPeerDiscovery {
    /// Pre-filtered peer entries ready for emission.
    peers: Vec<PeerInfo>,
    /// Events pending delivery.
    pending: VecDeque<PeerEvent>,
    /// Whether `start()` has been called.
    started: bool,
}

impl StaticPeerDiscovery {
    /// Create a new static discovery instance from a list of peer entries.
    ///
    /// The caller is responsible for filtering (e.g., excluding self).
    pub fn new(peers: Vec<PeerInfo>) -> Self {
        Self {
            peers,
            pending: VecDeque::new(),
            started: false,
        }
    }

    /// Create from a list of `StaticPeerEntry` entries (from `PeerConfig.static_peers`).
    ///
    /// Each entry's `node_id` is used as the peer ID.
    /// Entries matching `self_node_id` are excluded (skip self).
    /// The `connection_type` is forced to `Peer` regardless of what's configured.
    /// The `link_id` is derived deterministically from the source and destination node IDs
    /// so that reconnecting peers always present the same link identity.
    pub fn from_static_peers(entries: &[StaticPeerEntry], self_node_id: &str) -> Self {
        let peers = entries
            .iter()
            .filter(|entry| entry.node_id != self_node_id)
            .map(|entry| {
                let mut config = entry.config.clone();
                config.connection_type = ConnType::Peer;
                config.link_id = super::peer_link_id(self_node_id, &entry.node_id);
                PeerInfo {
                    id: entry.node_id.clone(),
                    config,
                }
            })
            .collect();
        Self::new(peers)
    }
}

impl PeerDiscovery for StaticPeerDiscovery {
    async fn start(&mut self) -> Result<(), PeerDiscoveryError> {
        if self.started {
            return Ok(());
        }
        self.started = true;

        for peer in &self.peers {
            self.pending.push_back(PeerEvent::Joined(peer.clone()));
        }

        Ok(())
    }

    async fn recv(&mut self) -> Result<PeerEvent, PeerDiscoveryError> {
        match self.pending.pop_front() {
            Some(event) => Ok(event),
            // Static discovery never produces more events after initial list.
            // Block forever (the peer sync manager will drop the discovery on shutdown).
            None => std::future::pending().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::client::ClientConfig;

    fn test_peers() -> Vec<PeerInfo> {
        vec![
            PeerInfo {
                id: "slim-1".to_string(),
                config: ClientConfig {
                    endpoint: "slim-1:8080".to_string(),
                    connection_type: ConnType::Peer,
                    ..Default::default()
                },
            },
            PeerInfo {
                id: "slim-2".to_string(),
                config: ClientConfig {
                    endpoint: "slim-2:8080".to_string(),
                    connection_type: ConnType::Peer,
                    ..Default::default()
                },
            },
        ]
    }

    fn assert_joined_with_id(event: &PeerEvent, expected_id: &str) {
        match event {
            PeerEvent::Joined(info) => assert_eq!(info.id, expected_id),
            PeerEvent::Left(_) => panic!("expected Joined, got Left"),
        }
    }

    #[tokio::test]
    async fn test_emits_all_peers() {
        let mut discovery = StaticPeerDiscovery::new(test_peers());
        discovery.start().await.unwrap();

        let event1 = discovery.recv().await.unwrap();
        let event2 = discovery.recv().await.unwrap();

        assert_joined_with_id(&event1, "slim-1");
        assert_joined_with_id(&event2, "slim-2");
    }

    #[tokio::test]
    async fn test_start_is_idempotent() {
        let mut discovery = StaticPeerDiscovery::new(test_peers());
        discovery.start().await.unwrap();
        discovery.start().await.unwrap(); // second call is no-op

        // Should still only have 2 events, not 4
        let _event1 = discovery.recv().await.unwrap();
        let _event2 = discovery.recv().await.unwrap();

        // recv would block forever here (no more events)
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), discovery.recv()).await;
        assert!(result.is_err()); // timed out = no more events
    }

    #[tokio::test]
    async fn test_empty_peer_list() {
        let mut discovery = StaticPeerDiscovery::new(vec![]);
        discovery.start().await.unwrap();

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), discovery.recv()).await;
        assert!(result.is_err()); // no events
    }

    #[tokio::test]
    async fn test_from_static_peers() {
        let entries = vec![
            StaticPeerEntry {
                node_id: "node-0".to_string(),
                config: ClientConfig {
                    endpoint: "http://slim-0:8080".to_string(),
                    ..Default::default()
                },
            },
            StaticPeerEntry {
                node_id: "node-1".to_string(),
                config: ClientConfig {
                    endpoint: "http://slim-1:8080".to_string(),
                    ..Default::default()
                },
            },
            StaticPeerEntry {
                node_id: "node-2".to_string(),
                config: ClientConfig {
                    endpoint: "http://slim-2:8080".to_string(),
                    ..Default::default()
                },
            },
        ];

        // Self (node-1) is excluded
        let mut discovery = StaticPeerDiscovery::from_static_peers(&entries, "node-1");
        discovery.start().await.unwrap();

        let event1 = discovery.recv().await.unwrap();
        let event2 = discovery.recv().await.unwrap();

        assert_joined_with_id(&event1, "node-0");
        assert_joined_with_id(&event2, "node-2");

        // Verify connection_type is forced to Peer
        if let PeerEvent::Joined(info) = &event1 {
            assert_eq!(info.config.connection_type, ConnType::Peer);
        }

        // Verify deterministic link_id: "self_node_id:peer_node_id"
        if let PeerEvent::Joined(info) = &event1 {
            assert_eq!(info.config.link_id, "node-1:node-0");
        }
        if let PeerEvent::Joined(info) = &event2 {
            assert_eq!(info.config.link_id, "node-1:node-2");
        }

        // No more events
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), discovery.recv()).await;
        assert!(result.is_err());
    }
}

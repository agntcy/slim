// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Static peer discovery implementation.
//!
//! Discovers peers from a pre-built list of [`PeerInfo`] entries (typically
//! derived from `dataplane.clients` with `connection_type: peer`).
//! Emits `Joined` events for all entries on startup. Never emits `Left`
//! events since static peers are assumed to be always available.

use std::collections::VecDeque;

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

    fn test_peers() -> Vec<PeerInfo> {
        vec![
            PeerInfo {
                id: "slim-1".to_string(),
                endpoint: "slim-1:8080".to_string(),
            },
            PeerInfo {
                id: "slim-2".to_string(),
                endpoint: "slim-2:8080".to_string(),
            },
        ]
    }

    #[tokio::test]
    async fn test_emits_all_peers() {
        let mut discovery = StaticPeerDiscovery::new(test_peers());
        discovery.start().await.unwrap();

        let event1 = discovery.recv().await.unwrap();
        let event2 = discovery.recv().await.unwrap();

        assert_eq!(
            event1,
            PeerEvent::Joined(PeerInfo {
                id: "slim-1".to_string(),
                endpoint: "slim-1:8080".to_string(),
            })
        );
        assert_eq!(
            event2,
            PeerEvent::Joined(PeerInfo {
                id: "slim-2".to_string(),
                endpoint: "slim-2:8080".to_string(),
            })
        );
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
}

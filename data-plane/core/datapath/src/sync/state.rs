// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

/// Tracks the state of connected peers and their connection IDs.
#[derive(Debug, Default)]
pub struct PeerState {
    /// Maps peer_id → connection metadata.
    peers: HashMap<String, PeerEntry>,
    /// Maps conn_id → peer_id for reverse lookup (e.g., on connection drop).
    conn_to_peer: HashMap<u64, String>,
}

#[derive(Debug, Clone)]
pub struct PeerEntry {
    pub conn_id: u64,
    pub endpoint: String,
    /// Whether we initiated the connection (true) or accepted it (false).
    pub is_outgoing: bool,
}

impl PeerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a connected peer. Returns false if already known.
    pub fn insert(&mut self, peer_id: String, entry: PeerEntry) -> bool {
        if self.peers.contains_key(&peer_id) {
            return false;
        }
        self.conn_to_peer.insert(entry.conn_id, peer_id.clone());
        self.peers.insert(peer_id, entry);
        true
    }

    /// Remove a peer by ID. Returns the entry if it existed.
    pub fn remove(&mut self, peer_id: &str) -> Option<PeerEntry> {
        if let Some(entry) = self.peers.remove(peer_id) {
            self.conn_to_peer.remove(&entry.conn_id);
            Some(entry)
        } else {
            None
        }
    }

    /// Remove a peer by connection ID (e.g., on unexpected disconnect).
    pub fn remove_by_conn(&mut self, conn_id: u64) -> Option<(String, PeerEntry)> {
        if let Some(peer_id) = self.conn_to_peer.remove(&conn_id)
            && let Some(entry) = self.peers.remove(&peer_id)
        {
            return Some((peer_id, entry));
        }
        None
    }

    /// Check if a peer is already connected (by peer ID).
    pub fn contains(&self, peer_id: &str) -> bool {
        self.peers.contains_key(peer_id)
    }

    /// Get the connection ID for a peer.
    pub fn conn_id(&self, peer_id: &str) -> Option<u64> {
        self.peers.get(peer_id).map(|e| e.conn_id)
    }

    /// Get all peer connection IDs.
    pub fn all_conn_ids(&self) -> Vec<u64> {
        self.peers.values().map(|e| e.conn_id).collect()
    }

    /// Look up a peer_id by connection ID.
    pub fn peer_id_for_conn(&self, conn_id: u64) -> Option<&str> {
        self.conn_to_peer.get(&conn_id).map(|s| s.as_str())
    }

    /// Look up a peer entry by peer ID.
    pub fn get(&self, peer_id: &str) -> Option<&PeerEntry> {
        self.peers.get(peer_id)
    }

    /// Number of connected peers.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether there are no connected peers.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Returns true if any known peer has an ID lexicographically smaller than `id`.
    pub fn has_peer_smaller_than(&self, id: &str) -> bool {
        self.peers.keys().any(|peer_id| peer_id.as_str() < id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_lookup() {
        let mut state = PeerState::new();
        let entry = PeerEntry {
            conn_id: 42,
            endpoint: "peer-1:8080".to_string(),
            is_outgoing: true,
        };
        assert!(state.insert("peer-1".to_string(), entry));
        assert!(state.contains("peer-1"));
        assert_eq!(state.conn_id("peer-1"), Some(42));
        assert_eq!(state.peer_id_for_conn(42), Some("peer-1"));
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn test_duplicate_insert_rejected() {
        let mut state = PeerState::new();
        let entry = PeerEntry {
            conn_id: 42,
            endpoint: "peer-1:8080".to_string(),
            is_outgoing: true,
        };
        assert!(state.insert("peer-1".to_string(), entry.clone()));
        assert!(!state.insert("peer-1".to_string(), entry));
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn test_remove_by_id() {
        let mut state = PeerState::new();
        let entry = PeerEntry {
            conn_id: 42,
            endpoint: "peer-1:8080".to_string(),
            is_outgoing: true,
        };
        state.insert("peer-1".to_string(), entry);
        let removed = state.remove("peer-1").unwrap();
        assert_eq!(removed.conn_id, 42);
        assert!(!state.contains("peer-1"));
        assert_eq!(state.peer_id_for_conn(42), None);
    }

    #[test]
    fn test_remove_by_conn() {
        let mut state = PeerState::new();
        let entry = PeerEntry {
            conn_id: 42,
            endpoint: "peer-1:8080".to_string(),
            is_outgoing: true,
        };
        state.insert("peer-1".to_string(), entry);
        let (peer_id, removed) = state.remove_by_conn(42).unwrap();
        assert_eq!(peer_id, "peer-1");
        assert_eq!(removed.conn_id, 42);
        assert!(state.is_empty());
    }
}

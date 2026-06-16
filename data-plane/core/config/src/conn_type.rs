// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic connection type.
//!
//! [`ConnType`] is intentionally kept free of any transport (tonic / hyper /
//! tokio) dependencies so it can be shared with the datapath on every target,
//! including `wasm32-unknown-unknown`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The type of connection a client establishes.
///
/// Determines how the datapath treats messages on this connection
/// (routing rules, subscription forwarding, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConnType {
    /// Connection with a local application (agent).
    /// Not exposed in configuration — only used internally.
    #[serde(skip)]
    Local,
    /// Connection with a remote SLIM instance in other deployment.
    /// Always handled by the control plane.
    #[default]
    Remote,
    /// Connection with a peer replica in the same deployment.
    Peer,
    /// Connection to the first SLIM node in the network
    /// Edge connections are handled by the data plane
    Edge,
}

impl ConnType {
    /// All variants, for iteration.
    pub const ALL: [ConnType; 4] = [
        ConnType::Local,
        ConnType::Remote,
        ConnType::Peer,
        ConnType::Edge,
    ];

    /// Number of ConnType variants (derived automatically).
    pub const COUNT: usize = Self::ALL.len();

    /// Index for array-based storage. Stable mapping.
    pub const fn index(self) -> usize {
        match self {
            ConnType::Local => 0,
            ConnType::Remote => 1,
            ConnType::Peer => 2,
            ConnType::Edge => 3,
        }
    }

    /// Converts from the legacy `is_local` boolean for backward compatibility.
    pub fn from_is_local(is_local: bool) -> Self {
        if is_local {
            ConnType::Local
        } else {
            ConnType::Remote
        }
    }

    /// Returns true if this is a local connection (app/agent).
    pub fn is_local(self) -> bool {
        matches!(self, ConnType::Local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_remote() {
        assert_eq!(ConnType::default(), ConnType::Remote);
    }

    #[test]
    fn all_and_count_are_consistent() {
        assert_eq!(ConnType::COUNT, 4);
        assert_eq!(ConnType::ALL.len(), ConnType::COUNT);
        assert_eq!(
            ConnType::ALL,
            [
                ConnType::Local,
                ConnType::Remote,
                ConnType::Peer,
                ConnType::Edge
            ]
        );
    }

    #[test]
    fn index_is_stable_and_unique() {
        assert_eq!(ConnType::Local.index(), 0);
        assert_eq!(ConnType::Remote.index(), 1);
        assert_eq!(ConnType::Peer.index(), 2);
        assert_eq!(ConnType::Edge.index(), 3);

        // Every variant maps to its position in ALL.
        for (i, ct) in ConnType::ALL.iter().enumerate() {
            assert_eq!(ct.index(), i);
        }
    }

    #[test]
    fn from_is_local_maps_to_local_or_remote() {
        assert_eq!(ConnType::from_is_local(true), ConnType::Local);
        assert_eq!(ConnType::from_is_local(false), ConnType::Remote);
    }

    #[test]
    fn is_local_only_true_for_local() {
        assert!(ConnType::Local.is_local());
        assert!(!ConnType::Remote.is_local());
        assert!(!ConnType::Peer.is_local());
        assert!(!ConnType::Edge.is_local());
    }

    #[test]
    fn serde_roundtrip_lowercase() {
        // Local is `#[serde(skip)]`, so only Remote, Peer, and Edge are wire-visible.
        assert_eq!(
            serde_json::to_string(&ConnType::Remote).unwrap(),
            "\"remote\""
        );
        assert_eq!(serde_json::to_string(&ConnType::Peer).unwrap(), "\"peer\"");
        assert_eq!(serde_json::to_string(&ConnType::Edge).unwrap(), "\"edge\"");

        assert_eq!(
            serde_json::from_str::<ConnType>("\"remote\"").unwrap(),
            ConnType::Remote
        );
        assert_eq!(
            serde_json::from_str::<ConnType>("\"peer\"").unwrap(),
            ConnType::Peer
        );
        assert_eq!(
            serde_json::from_str::<ConnType>("\"edge\"").unwrap(),
            ConnType::Edge
        );
    }

    #[test]
    fn copy_and_eq_semantics() {
        let a = ConnType::Edge;
        let b = a; // Copy
        assert_eq!(a, b);
        assert_ne!(ConnType::Local, ConnType::Edge);
    }
}

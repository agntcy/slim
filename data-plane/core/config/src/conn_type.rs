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
    /// Connection with a remote SLIM instance (other deployment, via controller).
    #[default]
    Remote,
    /// Connection with a peer replica in the same deployment.
    Peer,
}

impl ConnType {
    /// All variants, for iteration.
    pub const ALL: [ConnType; 3] = [ConnType::Local, ConnType::Remote, ConnType::Peer];

    /// Number of ConnType variants (derived automatically).
    pub const COUNT: usize = Self::ALL.len();

    /// Index for array-based storage. Stable mapping.
    pub const fn index(self) -> usize {
        match self {
            ConnType::Local => 0,
            ConnType::Remote => 1,
            ConnType::Peer => 2,
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

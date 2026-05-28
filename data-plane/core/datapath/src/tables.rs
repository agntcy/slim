// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

pub mod connection_table;
pub mod remote_subscription_table;
pub mod subscription_table;

pub mod pool;

use crate::api::{EncodedName, ProtoName};

/// Categorization of a connection for subscription table routing.
///
/// Determines which internal ConnList (local, remote, or peer) a subscription
/// is stored in, and which lists are queried during publish-time matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnCategory {
    /// Connection with a local application (agent)
    Local,
    /// Connection with a remote SLIM instance (other deployment, via controller)
    #[default]
    Remote,
    /// Connection with a peer replica in the same deployment
    Peer,
}

impl ConnCategory {
    /// Converts from the legacy `is_local` boolean for backward compatibility.
    pub fn from_is_local(is_local: bool) -> Self {
        if is_local {
            ConnCategory::Local
        } else {
            ConnCategory::Remote
        }
    }

    /// Returns true if this is a local connection (app/agent).
    pub fn is_local(self) -> bool {
        matches!(self, ConnCategory::Local)
    }
}

/// Determines which connection categories to include when matching publish messages.
///
/// Used by the source-type-aware routing to implement the 1-hop rule:
/// - Messages from Local sources: full routing (local + peer + remote)
/// - Messages from Peer sources: local + remote only (excludes peers to prevent loops)
/// - Messages from Remote sources: full routing (local + peer + remote)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchFilter {
    pub include_local: bool,
    pub include_remote: bool,
    pub include_peer: bool,
}

impl MatchFilter {
    /// Full routing: include all categories (used for Local and Remote sources)
    pub const ALL: Self = Self {
        include_local: true,
        include_remote: true,
        include_peer: true,
    };

    /// Exclude peers: used for Peer sources (1-hop rule)
    pub const EXCLUDE_PEER: Self = Self {
        include_local: true,
        include_remote: true,
        include_peer: false,
    };
}

pub trait SubscriptionTable {
    type Error;

    fn for_each<F>(&self, f: F)
    where
        F: FnMut(&ProtoName, u64, &[u64], &[u64], &[u64]);

    fn add_subscription(
        &self,
        name: ProtoName,
        conn: u64,
        category: ConnCategory,
        subscription_id: u64,
    ) -> Result<(), Self::Error>;

    fn remove_subscription(
        &self,
        name: &ProtoName,
        conn: u64,
        category: ConnCategory,
        subscription_id: u64,
    ) -> Result<(), Self::Error>;

    /// Remove all subscriptions for `conn` and return a map of each name to its set of
    /// subscription IDs, so that callers can restore the exact state later if needed.
    fn remove_connection(
        &self,
        conn: u64,
        category: ConnCategory,
    ) -> Result<HashMap<ProtoName, HashSet<u64>>, Self::Error>;

    fn match_one(
        &self,
        encoded: &EncodedName,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<u64, Self::Error>;

    fn match_all(
        &self,
        encoded: &EncodedName,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<Vec<u64>, Self::Error>;
}

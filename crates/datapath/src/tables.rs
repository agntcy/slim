// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

pub mod connection_table;
pub mod subscription_table;

pub mod pool;

use crate::api::ProtoName;

pub use slim_config::conn_type::ConnType;

/// Determines which connection categories to include when matching publish messages.
///
/// Used by the source-type-aware routing to implement the 1-hop rule:
/// - Messages from Local sources: full routing (local + peer + remote)
/// - Messages from Peer sources: local + remote only (excludes peers to prevent loops)
/// - Messages from Remote sources: full routing (local + peer + remote)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchFilter {
    /// Indexed by `ConnType::index()`. True means include that category.
    pub include: [bool; ConnType::COUNT],
}

impl MatchFilter {
    /// Full routing: include all categories (used for Local and Remote sources)
    pub const ALL: Self = Self {
        include: [true; ConnType::COUNT],
    };

    /// Exclude peers: used for Peer sources (1-hop rule)
    pub const EXCLUDE_PEER: Self = Self {
        include: [true, true, false, true], // Local, Remote, Peer, Edge
    };

    /// Whether the given connection type is included in this filter.
    pub const fn includes(self, ct: ConnType) -> bool {
        self.include[ct.index()]
    }
}

pub trait SubscriptionTable {
    type Error;

    fn for_each<F>(&self, f: F)
    where
        F: FnMut(&ProtoName, u128, &[u64], &[u64], &[u64], &[u64]);

    /// Add a subscription. Returns `true` if this is the first connection for the
    /// given `(name, category)` pair (0→1 transition).
    fn add_subscription(
        &self,
        name: ProtoName,
        conn: u64,
        category: ConnType,
        subscription_id: u64,
    ) -> Result<bool, Self::Error>;

    /// Remove a subscription. Returns `true` if this was the last connection for the
    /// given `(name, category)` pair (1→0 transition).
    fn remove_subscription(
        &self,
        name: &ProtoName,
        conn: u64,
        category: ConnType,
        subscription_id: u64,
    ) -> Result<bool, Self::Error>;

    /// Remove all subscriptions for `conn` and return a map of each name to its set of
    /// subscription IDs, so that callers can restore the exact state later if needed.
    fn remove_connection(
        &self,
        conn: u64,
        category: ConnType,
    ) -> Result<HashMap<ProtoName, HashSet<u64>>, Self::Error>;

    fn match_one(
        &self,
        components: [u64; 3],
        id: u128,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<u64, Self::Error>;

    fn match_all(
        &self,
        components: [u64; 3],
        id: u128,
        incoming_conn: u64,
        filter: MatchFilter,
    ) -> Result<Vec<u64>, Self::Error>;
}

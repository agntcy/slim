// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Peer Sync Manager — manages peer-to-peer subscription synchronization
//! between replicas in the same deployment.
//!
//! # Architecture
//!
//! The PeerSyncManager:
//! - Listens to peer discovery events (Joined/Left)
//! - Establishes connections to peers (single bidirectional link per pair)
//! - Performs full subscription sync on peer join
//! - Forwards incremental subscription changes (aggregate transitions only)
//! - Handles peer disconnection cleanup
//!
//! # Connection Deduplication
//!
//! Only the peer with the lexicographically smaller ID initiates the connection.
//! The other side accepts the incoming connection. This guarantees exactly one
//! link per peer pair with no races.

mod manager;
mod state;
mod sync;

pub use manager::{PeerSyncConfig, PeerSyncManager};
pub use state::PeerState;

use crate::api::ProtoName;

/// Subscription events emitted by the message processor when a local
/// subscription aggregate transitions (0→1 or 1→0 local subscribers).
///
/// Only aggregate transitions are emitted — intermediate changes (e.g., second
/// local subscriber for the same name) do not produce events.
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    /// A name gained its first local subscriber.
    Added {
        name: ProtoName,
        subscription_id: u64,
    },
    /// A name lost its last local subscriber.
    Removed {
        name: ProtoName,
        subscription_id: u64,
    },
}

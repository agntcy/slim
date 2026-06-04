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
//! - Maintains the SubscriptionForwarder's peer connection list
//! - Handles peer disconnection cleanup
//!
//! # Subscription Forwarding
//!
//! Incremental subscription forwarding (to peers, controller, hub relay) is handled
//! by the `SubscriptionForwarder` component, which is called directly from the
//! `MessageProcessor::process_subscription` path. The PeerSyncManager only maintains
//! the peer connection list and performs full sync on join.
//!
//! # Connection Deduplication
//!
//! Only the peer with the lexicographically smaller ID initiates the connection.
//! The other side accepts the incoming connection. This guarantees exactly one
//! link per peer pair with no races.

pub(crate) mod forwarder;
mod manager;
mod state;
pub(crate) mod sync;

pub use forwarder::{ForwardTargets, PeerTarget, SubscriptionForwarder};
pub use manager::{PeerSyncConfig, PeerSyncManager};
pub use state::PeerState;

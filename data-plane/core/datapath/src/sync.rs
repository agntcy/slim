// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Subscription synchronization for peer and remote connections.
//!
//! - [`peer`]: Full-sync mechanism for peer connections. On every connect/reconnect,
//!   peers exchange their complete routing table.
//! - [`remote`]: Restore mechanism for remote/controller connections. On reconnect,
//!   only the previously-forwarded subscription set is replayed.
//! - [`forwarder`]: Subscription forwarding logic (loop prevention, ACK tracking).
//! - [`manager`]: Peer lifecycle management (discovery, connect/disconnect).

pub(crate) mod forwarder;
mod manager;
pub mod peer;
pub(crate) mod recovery;
pub mod remote;
mod state;

pub use forwarder::{ForwardTargets, PeerTarget, SubscriptionForwarder};
pub use manager::{PeerSyncConfig, PeerSyncManager};
pub use state::PeerState;

use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::proto::dataplane::v1::Message;
use crate::api::{ProtoMessage, ProtoName};
use crate::messages::utils::MessageError;

/// Monotonically increasing counter for generating unique sync subscription IDs.
/// These IDs are distinct from application-level subscription IDs.
static SYNC_SUB_ID: AtomicU64 = AtomicU64::new(1_000_000);

/// Generate a unique subscription ID for sync messages.
pub fn next_subscription_id() -> u64 {
    SYNC_SUB_ID.fetch_add(1, Ordering::Relaxed)
}

/// Build a Subscribe message for a given name.
pub fn build_subscribe_msg(
    name: &ProtoName,
    subscription_id: u64,
    ttl: u32,
) -> Result<Message, MessageError> {
    ProtoMessage::builder()
        .source(name.clone())
        .destination(name.clone())
        .subscription_id(subscription_id)
        .ttl(ttl)
        .build_subscribe()
}

/// Build an Unsubscribe message for a given name.
pub fn build_unsubscribe_msg(
    name: &ProtoName,
    subscription_id: u64,
    ttl: u32,
) -> Result<Message, MessageError> {
    ProtoMessage::builder()
        .source(name.clone())
        .destination(name.clone())
        .subscription_id(subscription_id)
        .ttl(ttl)
        .build_unsubscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_id_monotonic() {
        let id1 = next_subscription_id();
        let id2 = next_subscription_id();
        assert!(id2 > id1);
    }

    #[test]
    fn test_build_subscribe_msg() {
        let name = ProtoName::from_strings(["org", "ns", "class"]);
        let msg = build_subscribe_msg(&name, 42, 6).unwrap();
        assert!(msg.is_subscribe());
        assert_eq!(msg.get_ttl(), 6);
    }

    #[test]
    fn test_build_unsubscribe_msg() {
        let name = ProtoName::from_strings(["org", "ns", "class"]);
        let msg = build_unsubscribe_msg(&name, 42, 6).unwrap();
        assert!(msg.is_unsubscribe());
        assert_eq!(msg.get_ttl(), 6);
    }
}

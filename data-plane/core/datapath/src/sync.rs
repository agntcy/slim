// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Subscription synchronization for peer and remote connections.
//!
//! - [`peer`]: Full-sync mechanism for peer connections. On every connect/reconnect,
//!   peers exchange their complete routing table.
//! - [`remote`]: Restore mechanism for remote/controller connections. On reconnect,
//!   only the previously-forwarded subscription set is replayed.
//! - [`forwarder`]: Peer sync component (discovery, forwarding, ACK tracking).

pub(crate) mod forwarder;
pub mod peer;
pub(crate) mod recovery;
pub mod remote;
mod state;

pub use forwarder::{ForwardTargets, PeerSync, PeerSyncConfig, PeerTarget};
pub use state::PeerState;

use crate::api::proto::dataplane::v1::Message;
use crate::api::{ProtoMessage, ProtoName};
use crate::messages::utils::MessageError;

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

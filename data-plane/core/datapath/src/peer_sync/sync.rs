// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, warn};

use crate::api::proto::dataplane::v1::Message;
use crate::api::{NameId, ProtoMessage, ProtoName};
use crate::errors::DataPathError;
use crate::message_processing::MessageProcessor;
use crate::tables::SubscriptionTable;

/// Monotonically increasing counter for generating unique peer-sync subscription IDs.
/// These IDs are distinct from application-level subscription IDs.
static PEER_SYNC_SUB_ID: AtomicU64 = AtomicU64::new(1_000_000);

/// Generate a unique subscription ID for peer-sync messages.
pub fn next_subscription_id() -> u64 {
    PEER_SYNC_SUB_ID.fetch_add(1, Ordering::Relaxed)
}

/// Build a Subscribe message for a given name.
pub fn build_subscribe_msg(name: &ProtoName, subscription_id: u64) -> Result<Message, String> {
    ProtoMessage::builder()
        .source(name.clone())
        .destination(name.clone())
        .subscription_id(subscription_id)
        .build_subscribe()
        .map_err(|e| format!("failed to build subscribe message: {:?}", e))
}

/// Build an Unsubscribe message for a given name.
pub fn build_unsubscribe_msg(name: &ProtoName, subscription_id: u64) -> Result<Message, String> {
    ProtoMessage::builder()
        .source(name.clone())
        .destination(name.clone())
        .subscription_id(subscription_id)
        .build_unsubscribe()
        .map_err(|e| format!("failed to build unsubscribe message: {:?}", e))
}

/// Collect all names that have at least one local subscriber.
/// Returns a list of (ProtoName with ID, subscription_id) for full sync.
pub fn collect_local_subscriptions(mp: &MessageProcessor) -> Vec<(ProtoName, u64)> {
    let mut entries = Vec::new();
    mp.subscription_table().for_each(|name, id, local_conns, _remote, _peer| {
        if !local_conns.is_empty() {
            let full_name = if id != NameId::NULL_COMPONENT {
                name.clone().with_id(id)
            } else {
                name.clone()
            };
            let sub_id = next_subscription_id();
            entries.push((full_name, sub_id));
        }
    });
    entries
}

/// Send full subscription sync to a peer connection.
/// Collects all local subscriptions and sends them as Subscribe messages.
/// Returns the number of subscriptions sent.
pub async fn send_full_sync(
    mp: &MessageProcessor,
    peer_conn_id: u64,
) -> Result<usize, DataPathError> {
    // Collect outside the closure (for_each is sync, send_msg is async).
    let subscriptions = collect_local_subscriptions(mp);
    let count = subscriptions.len();

    debug!(
        %peer_conn_id,
        subscription_count = count,
        "sending full subscription sync to peer"
    );

    for (name, sub_id) in subscriptions {
        let msg = match build_subscribe_msg(&name, sub_id) {
            Ok(m) => m,
            Err(e) => {
                warn!(%peer_conn_id, error = %e, "failed to build sync subscribe message");
                continue;
            }
        };
        if let Err(e) = mp.send_msg(msg, peer_conn_id).await {
            warn!(
                %peer_conn_id,
                error = %e,
                "failed to send sync subscribe to peer, aborting full sync"
            );
            return Err(e);
        }
    }

    debug!(%peer_conn_id, count, "full subscription sync complete");
    Ok(count)
}

/// Send an incremental subscription update to all connected peers.
pub async fn broadcast_subscribe(
    mp: &MessageProcessor,
    name: &ProtoName,
    subscription_id: u64,
    peer_conn_ids: &[u64],
) {
    let msg = match build_subscribe_msg(name, subscription_id) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "failed to build incremental subscribe message");
            return;
        }
    };

    for &conn_id in peer_conn_ids {
        if let Err(e) = mp.send_msg(msg.clone(), conn_id).await {
            warn!(%conn_id, error = %e, "failed to send subscribe to peer");
        }
    }
}

/// Send an incremental unsubscription update to all connected peers.
pub async fn broadcast_unsubscribe(
    mp: &MessageProcessor,
    name: &ProtoName,
    subscription_id: u64,
    peer_conn_ids: &[u64],
) {
    let msg = match build_unsubscribe_msg(name, subscription_id) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "failed to build incremental unsubscribe message");
            return;
        }
    };

    for &conn_id in peer_conn_ids {
        if let Err(e) = mp.send_msg(msg.clone(), conn_id).await {
            warn!(%conn_id, error = %e, "failed to send unsubscribe to peer");
        }
    }
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
        let msg = build_subscribe_msg(&name, 42).unwrap();
        assert!(msg.is_subscribe());
    }

    #[test]
    fn test_build_unsubscribe_msg() {
        let name = ProtoName::from_strings(["org", "ns", "class"]);
        let msg = build_unsubscribe_msg(&name, 42).unwrap();
        assert!(msg.is_unsubscribe());
    }
}

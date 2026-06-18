// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Peer connection sync: full sync of subscriptions on connect/reconnect.
//!
//! Peer connections exchange their complete routing table on every connection
//! establishment. No per-subscription tracking is needed — the full sync
//! is the single mechanism for both initial connect and reconnect.

use std::collections::HashMap;

use tracing::{debug, warn};

use crate::api::ProtoName;
use crate::errors::DataPathError;
use crate::message_processing::MessageProcessor;
use crate::tables::MatchFilter;

use super::build_subscribe_msg;

/// Collect subscriptions matching the given filter, excluding `exclude_conn`.
///
/// Returns one `(ProtoName, sub_id)` per unique name, using the original subscription ID.
/// The filter controls which subscription categories to include:
/// - `MatchFilter::ALL`: local + remote + peer (generic/hub topologies)
/// - `MatchFilter::EXCLUDE_PEER`: local + remote only (full-mesh topology)
pub fn collect_subscriptions(
    mp: &MessageProcessor,
    exclude_conn: u64,
    filter: MatchFilter,
) -> Vec<(ProtoName, u64)> {
    let mut entries: HashMap<ProtoName, u64> = HashMap::new();
    mp.subscription_table()
        .for_each_subscription(|name, sub_id, conn_id, category| {
            if !filter.includes(category) {
                return;
            }
            if conn_id == exclude_conn {
                return;
            }
            // Keep one sub_id per unique name (first seen wins).
            entries.entry(name).or_insert(sub_id);
        });
    entries.into_iter().collect()
}

/// Send local + remote subscriptions to a peer connection (full-mesh topology).
/// Peers already know each other's routes, so peer-learned subs are excluded.
/// Returns the number of subscriptions sent.
pub async fn send_local_remote_sync(
    mp: &MessageProcessor,
    peer_conn_id: u64,
    ttl: u32,
) -> Result<usize, DataPathError> {
    let subscriptions = collect_subscriptions(mp, peer_conn_id, MatchFilter::EXCLUDE_PEER);
    send_subscriptions(mp, peer_conn_id, &subscriptions, ttl).await
}

/// Send full subscription sync (local + remote + other peers, excluding target).
/// Used for generic/standalone topologies where a node relays routes.
pub async fn send_full_sync(
    mp: &MessageProcessor,
    peer_conn_id: u64,
    ttl: u32,
) -> Result<usize, DataPathError> {
    let subscriptions = collect_subscriptions(mp, peer_conn_id, MatchFilter::ALL);
    send_subscriptions(mp, peer_conn_id, &subscriptions, ttl).await
}

/// Send a list of subscriptions to a peer connection.
pub async fn send_subscriptions(
    mp: &MessageProcessor,
    peer_conn_id: u64,
    subscriptions: &[(ProtoName, u64)],
    ttl: u32,
) -> Result<usize, DataPathError> {
    let count = subscriptions.len();

    debug!(
        %peer_conn_id,
        subscription_count = count,
        "sending full subscription sync to peer"
    );

    for (name, sub_id) in subscriptions {
        let msg = match build_subscribe_msg(name, *sub_id, ttl) {
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

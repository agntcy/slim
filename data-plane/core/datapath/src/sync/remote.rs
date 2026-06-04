// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Remote/controller connection sync.
//!
//! [`RemoteSync`] tracks subscriptions forwarded to each remote connection,
//! preserves routing state during brief disconnects (recovery), and handles
//! restore on reconnect. Unlike peer connections (which do a full sync),
//! remote connections only replay the specific set of subscriptions that were
//! previously forwarded to them.

use std::collections::{HashMap, HashSet};

use display_error_chain::ErrorChainExt;
use parking_lot::RwLock;
use tracing::error;

use crate::api::ProtoName;
use crate::message_processing::MessageProcessor;
use crate::sync::recovery::RecoveryTable;

// ─── SubscriptionInfo ────────────────────────────────────────────────────────

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct SubscriptionInfo {
    source: ProtoName,
    name: ProtoName,
    source_identity: String,
    subscription_id: u64,
}

impl SubscriptionInfo {
    #[cfg(test)]
    pub(crate) fn new(
        source: ProtoName,
        name: ProtoName,
        source_identity: String,
        _conn: u64,
        subscription_id: u64,
    ) -> Self {
        Self {
            source,
            name,
            source_identity,
            subscription_id,
        }
    }

    pub fn source(&self) -> &ProtoName {
        &self.source
    }

    pub fn source_identity(&self) -> &String {
        &self.source_identity
    }

    pub fn name(&self) -> &ProtoName {
        &self.name
    }

    pub fn subscription_id(&self) -> u64 {
        self.subscription_id
    }
}

// ─── RemoteSync ──────────────────────────────────────────────────────────────

/// Manages subscription tracking, recovery, and restore for remote (non-peer) connections.
///
/// Event-driven interface used by the message processor:
/// - [`on_forwarded_subscription`](Self::on_forwarded_subscription): a sub was sent to a remote conn
/// - [`on_connection_drop`](Self::on_connection_drop): a remote connection was lost
/// - [`get_subscriptions_for_reconnect`](Self::get_subscriptions_for_reconnect): snapshot before client reconnect
/// - [`restore`](Self::restore): replay subscriptions after reconnect
///
/// Also owns the [`RecoveryTable`] for server-side route preservation during brief disconnects.
#[derive(Debug)]
pub struct RemoteSync {
    /// Subscriptions forwarded to each remote connection, keyed by conn_id.
    table: RwLock<HashMap<u64, HashSet<SubscriptionInfo>>>,

    /// Server-side route preservation during brief disconnects (keyed by link_id).
    pub(crate) recovery: RecoveryTable,
}

impl RemoteSync {
    pub fn new(recovery_ttl: Option<std::time::Duration>) -> Self {
        let recovery = match recovery_ttl {
            Some(ttl) => RecoveryTable::new(ttl),
            None => RecoveryTable::default(),
        };
        Self {
            table: RwLock::new(HashMap::new()),
            recovery,
        }
    }

    /// Record that a subscription was forwarded (or unforwarded) on a remote connection.
    pub fn on_forwarded_subscription(
        &self,
        source: ProtoName,
        name: ProtoName,
        source_identity: String,
        conn: u64,
        add: bool,
        subscription_id: u64,
    ) {
        let info = SubscriptionInfo {
            source,
            name,
            source_identity,
            subscription_id,
        };
        let mut map = self.table.write();
        if add {
            map.entry(conn).or_default().insert(info);
        } else {
            match map.get_mut(&conn) {
                None => {
                    error!(%conn, "on_forwarded_subscription(remove): connection not found");
                }
                Some(set) => {
                    set.remove(&info);
                    if set.is_empty() {
                        map.remove(&conn);
                    }
                }
            }
        }
    }

    /// A remote connection dropped — remove and return its tracked subscriptions.
    /// The caller stores them in the recovery table (keyed by link_id).
    pub fn on_connection_drop(&self, conn: u64) -> HashSet<SubscriptionInfo> {
        let mut map = self.table.write();
        map.remove(&conn).unwrap_or_default()
    }

    /// Snapshot the subscriptions forwarded on a connection (for client-side reconnect).
    /// Unlike `on_connection_drop`, this does NOT remove them from the table.
    pub fn get_subscriptions_for_reconnect(&self, conn: u64) -> HashSet<SubscriptionInfo> {
        let map = self.table.read();
        map.get(&conn).cloned().unwrap_or_default()
    }

    /// Re-send previously-forwarded subscriptions to a remote connection after reconnect.
    ///
    /// When `restore_tracking` is `true` (server-side recovery), also re-registers each
    /// subscription in the tracking table. This is necessary because `on_connection_drop`
    /// already wiped that state.
    ///
    /// When `restore_tracking` is `false` (client-side reconnect), the tracking table was
    /// never cleaned (reconnect reuses the same slot), so no re-registration is needed.
    pub async fn restore(
        &self,
        mp: &MessageProcessor,
        remote_subs: &HashSet<SubscriptionInfo>,
        conn_index: u64,
        restore_tracking: bool,
    ) {
        for r in remote_subs {
            let sub_msg = crate::api::proto::dataplane::v1::Message::builder()
                .source(r.source().clone())
                .destination(r.name().clone())
                .identity(r.source_identity())
                .build_subscribe()
                .unwrap();
            if let Err(e) = mp.send_msg(sub_msg, conn_index).await {
                error!(
                    error = %e.chain(), %conn_index,
                    "error restoring subscription on remote node",
                );
            } else if restore_tracking {
                self.on_forwarded_subscription(
                    r.source().clone(),
                    r.name().clone(),
                    r.source_identity().clone(),
                    conn_index,
                    true,
                    r.subscription_id(),
                );
            }
        }
    }
}

impl Default for RemoteSync {
    fn default() -> Self {
        Self::new(None)
    }
}

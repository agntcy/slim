// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Unified subscription forwarding component.
//!
//! Handles all outbound subscription/unsubscription forwarding:
//! - To peers (full mesh broadcast or hub relay excluding source)
//! - To the forward connection (controller)
//!
//! Owns the in-flight ACK tracking state (register/resolve/timeout) and
//! spawns tasks for ACK lifecycle management so message processing is never blocked.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::api::ProtoName;
use crate::api::proto::dataplane::v1::Message;
use crate::errors::DataPathError;
use crate::message_processing::MessageProcessor;
use crate::peer_discovery::config::PeerTopology;

use super::sync;

/// Timeout for a single ACK wait cycle before retrying.
pub(crate) const ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum number of retry attempts for a remote subscription ACK.
pub(crate) const ACK_MAX_RETRIES: u32 = 3;

/// Specifies which peers to target for subscription forwarding.
#[derive(Debug, Clone)]
pub enum PeerTarget {
    /// Forward to ALL connected peers (normal subscription transition).
    All,
    /// Forward to all peers EXCEPT the specified connection (hub relay from spoke).
    ExcludeConn(u64),
}

/// Specifies where to forward a subscription.
#[derive(Debug, Clone)]
pub struct ForwardTargets {
    /// Forward to peer connections. None = skip peer forwarding.
    pub peers: Option<PeerTarget>,
    /// Forward to a specific connection (controller), preserving the original message.
    pub forward_conn: Option<u64>,
}

impl ForwardTargets {
    /// Returns true if there are any targets to forward to.
    pub fn has_any(&self) -> bool {
        self.peers.is_some() || self.forward_conn.is_some()
    }

    /// No forwarding targets.
    pub fn none() -> Self {
        Self {
            peers: None,
            forward_conn: None,
        }
    }
}

/// Unified subscription forwarding and ACK lifecycle management.
///
/// Shared via `Arc` between `PeerSyncManager` (updates peer list) and
/// `MessageProcessor` (calls forwarding methods).
#[derive(Debug, Clone)]
pub struct SubscriptionForwarder {
    inner: Arc<SubscriptionForwarderInner>,
}

#[derive(Debug)]
struct SubscriptionForwarderInner {
    /// Current peer connection IDs (updated by PeerSyncManager on join/leave).
    peer_conns: RwLock<Vec<u64>>,
    /// Tracks which sub_id was forwarded to each peer for each name.
    /// name → { peer_conn_id → sub_id }
    forwarded_subs: RwLock<HashMap<ProtoName, HashMap<u64, u64>>>,
    /// In-flight pending ACK state: sub_id → oneshot sender.
    pending_acks: RwLock<HashMap<u64, oneshot::Sender<Result<(), DataPathError>>>>,
    /// Reference to MessageProcessor for sending messages.
    message_processor: MessageProcessor,
    /// Whether this node is the hub in hub-and-spoke topology.
    is_hub: bool,
    /// Topology type.
    topology: PeerTopology,
}

impl SubscriptionForwarder {
    /// Create a new SubscriptionForwarder.
    pub fn new(message_processor: MessageProcessor, is_hub: bool, topology: PeerTopology) -> Self {
        Self {
            inner: Arc::new(SubscriptionForwarderInner {
                peer_conns: RwLock::new(Vec::new()),
                forwarded_subs: RwLock::new(HashMap::new()),
                pending_acks: RwLock::new(HashMap::new()),
                message_processor,
                is_hub,
                topology,
            }),
        }
    }

    /// Create a standalone forwarder (no peers configured).
    /// Still handles controller forwarding and ACK tracking.
    pub fn standalone(message_processor: MessageProcessor) -> Self {
        Self::new(message_processor, false, PeerTopology::FullMesh)
    }

    /// Update the peer connection list (called by PeerSyncManager on join/leave).
    pub fn set_peer_conns(&self, conns: Vec<u64>) {
        *self.inner.peer_conns.write() = conns;
    }

    /// Add a peer connection ID.
    pub fn add_peer_conn(&self, conn_id: u64) {
        let mut conns = self.inner.peer_conns.write();
        if !conns.contains(&conn_id) {
            conns.push(conn_id);
        }
    }

    /// Remove a peer connection ID.
    pub fn remove_peer_conn(&self, conn_id: u64) {
        self.inner.peer_conns.write().retain(|&c| c != conn_id);
    }

    /// Get a snapshot of the current peer connection list.
    pub fn peer_conns(&self) -> Vec<u64> {
        self.inner.peer_conns.read().clone()
    }

    /// Whether this node is the hub.
    pub fn is_hub(&self) -> bool {
        self.inner.is_hub
    }

    /// Topology type.
    pub fn topology(&self) -> &PeerTopology {
        &self.inner.topology
    }

    /// Register a subscription that was sent during full sync.
    /// This ensures unsubscribe later can look up the correct per-peer sub_id.
    pub fn register_full_sync_sub(&self, name: &ProtoName, conn_id: u64, sub_id: u64) {
        self.inner
            .forwarded_subs
            .write()
            .entry(name.clone())
            .or_default()
            .insert(conn_id, sub_id);
    }

    // ── ACK tracking ─────────────────────────────────────────────────────────

    /// Register a new in-flight ACK; returns the result receiver.
    pub(crate) fn register_ack(&self, ack_id: u64) -> oneshot::Receiver<Result<(), DataPathError>> {
        let (tx, rx) = oneshot::channel();
        self.inner.pending_acks.write().insert(ack_id, tx);
        rx
    }

    /// Deliver a result to a waiting ACK receiver (no-op if unknown id).
    /// Called from the message processing path when a SubscriptionAck arrives.
    pub(crate) fn resolve_ack(&self, ack_id: u64, result: Result<(), DataPathError>) {
        if let Some(tx) = self.inner.pending_acks.write().remove(&ack_id) {
            debug!(%ack_id, "subscription: remote ack received");
            let _ = tx.send(result);
        }
    }

    /// Remove a pending ACK entry (e.g. all retries exhausted, or cleanup).
    pub(crate) fn remove_ack(&self, ack_id: u64) {
        self.inner.pending_acks.write().remove(&ack_id);
    }

    /// Spawn a task that forwards the subscription to targets, waits for ACKs,
    /// then sends the upstream ACK to the client. Does NOT block the caller.
    ///
    /// The spawned task:
    /// 1. Sends to target peers (unique sub_id per peer) + forward_conn (original msg)
    /// 2. Waits for ACKs with timeout + retry
    /// 3. ACKs the upstream client based on aggregate result
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_forward_and_ack(
        &self,
        msg: Message,
        name: ProtoName,
        sub_id: u64,
        add: bool,
        targets: ForwardTargets,
        in_connection: u64,
        upstream_subscription_id: Option<u64>,
    ) {
        let forwarder = self.clone();
        tokio::spawn(async move {
            forwarder
                .forward_and_ack(
                    msg,
                    name,
                    sub_id,
                    add,
                    targets,
                    in_connection,
                    upstream_subscription_id,
                )
                .await;
        });
    }

    /// The actual forwarding + ACK lifecycle (runs in spawned task).
    #[allow(clippy::too_many_arguments)]
    async fn forward_and_ack(
        &self,
        msg: Message,
        name: ProtoName,
        sub_id: u64,
        add: bool,
        targets: ForwardTargets,
        in_connection: u64,
        upstream_subscription_id: Option<u64>,
    ) {
        let mp = &self.inner.message_processor;

        // Run peer forwarding and forward-conn forwarding concurrently.
        let (peer_result, forward_result) = tokio::join!(
            self.forward_to_peers(&name, add, &targets),
            self.forward_to_conn(&msg, sub_id, add, &targets),
        );

        // Aggregate results:
        // - Forward (controller) failure is critical → propagate to upstream
        // - Peer failure is non-fatal → log and treat as OK
        let final_result = match (&peer_result, &forward_result) {
            (_, Err(e)) => {
                // Controller ACK failure is fatal
                Err(DataPathError::RemoteSubscriptionAckError(e.to_string()))
            }
            (Err(e), _) => {
                // Peer failure is non-fatal, log warning
                warn!(error = %e, %name, "peer subscription forwarding failed (non-fatal)");
                Ok(())
            }
            _ => Ok(()),
        };

        // Send upstream ACK to client
        if let Some(id) = upstream_subscription_id {
            debug!(
                %in_connection,
                subscription_id = id,
                ok = final_result.is_ok(),
                "sending subscription ack after forwarding"
            );
            mp.send_subscription_ack(in_connection, id, &final_result)
                .await;
        }
    }

    /// Forward subscription/unsubscription to peer connections.
    ///
    /// For subscribe: generates a unique sub_id per peer, sends to each,
    /// waits for ACKs from all peers with timeout.
    ///
    /// For unsubscribe: looks up previously stored per-peer sub_ids and
    /// sends unsubscribe best-effort (no ACK wait).
    async fn forward_to_peers(
        &self,
        name: &ProtoName,
        add: bool,
        targets: &ForwardTargets,
    ) -> Result<(), DataPathError> {
        let peer_target = match &targets.peers {
            Some(t) => t,
            None => return Ok(()),
        };

        // Snapshot peer conn IDs (don't hold lock across await)
        let peer_conns: Vec<u64> = {
            let conns = self.inner.peer_conns.read();
            match peer_target {
                PeerTarget::All => conns.clone(),
                PeerTarget::ExcludeConn(exclude) => {
                    conns.iter().copied().filter(|c| c != exclude).collect()
                }
            }
        };

        if peer_conns.is_empty() {
            debug!(%name, "no peer connections, skipping peer forwarding");
            return Ok(());
        }

        let mp = &self.inner.message_processor;

        if add {
            debug!(%name, ?peer_conns, "forwarding subscription to peers");

            // Generate unique sub_id per peer, build per-peer messages, send + register ACKs.
            let mut ack_futures = Vec::with_capacity(peer_conns.len());

            for &conn_id in &peer_conns {
                let per_peer_sub_id = sync::next_subscription_id();

                let peer_msg = match sync::build_subscribe_msg(name, per_peer_sub_id) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(error = %e, %conn_id, "failed to build peer subscribe message");
                        continue;
                    }
                };

                let rx = self.register_ack(per_peer_sub_id);
                if let Err(e) = mp.send_msg(peer_msg, conn_id).await {
                    warn!(%conn_id, error = %e, "failed to send subscribe to peer");
                    self.remove_ack(per_peer_sub_id);
                    continue;
                }

                // Track the per-peer sub_id for later unsubscribe
                self.inner
                    .forwarded_subs
                    .write()
                    .entry(name.clone())
                    .or_default()
                    .insert(conn_id, per_peer_sub_id);

                ack_futures.push((conn_id, rx, per_peer_sub_id));
            }

            // Wait for all ACKs with timeout
            for (conn_id, rx, per_peer_sub_id) in ack_futures {
                match tokio::time::timeout(ACK_TIMEOUT, rx).await {
                    Ok(Ok(result)) => {
                        if let Err(e) = &result {
                            warn!(%conn_id, error = %e, "peer subscription ACK failed");
                        }
                    }
                    Ok(Err(_)) => {
                        // Sender dropped (shutdown)
                        debug!(%conn_id, "peer ACK sender dropped");
                    }
                    Err(_) => {
                        warn!(%conn_id, "peer subscription ACK timeout");
                        self.remove_ack(per_peer_sub_id);
                    }
                }
            }
        } else {
            // Unsubscribe: look up per-peer sub_ids and send to each
            let peer_entries: Vec<(u64, u64)> = {
                let mut fwd = self.inner.forwarded_subs.write();
                if let Some(entries) = fwd.remove(name) {
                    entries.into_iter().collect()
                } else {
                    Vec::new()
                }
            };

            if peer_entries.is_empty() {
                debug!(%name, "no forwarded sub_ids found, skipping peer unsubscribe");
                return Ok(());
            }

            debug!(%name, entries = ?peer_entries, "forwarding unsubscription to peers");

            for (conn_id, sub_id) in peer_entries {
                // Only send to peers that are still connected
                if !peer_conns.contains(&conn_id) {
                    continue;
                }
                let msg = match sync::build_unsubscribe_msg(name, sub_id) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(%conn_id, error = %e, "failed to build peer unsubscribe message");
                        continue;
                    }
                };
                if let Err(e) = mp.send_msg(msg, conn_id).await {
                    warn!(%conn_id, error = %e, "failed to send unsubscribe to peer");
                }
            }
        }

        Ok(())
    }

    /// Forward subscription/unsubscription to the forward connection (controller).
    ///
    /// Sends the original message, tracks in remote subscription table,
    /// and waits for ACK with retry.
    async fn forward_to_conn(
        &self,
        msg: &Message,
        sub_id: u64,
        add: bool,
        targets: &ForwardTargets,
    ) -> Result<(), DataPathError> {
        let out_conn = match targets.forward_conn {
            Some(c) => c,
            None => return Ok(()),
        };

        let mp = &self.inner.message_processor;

        debug!(%out_conn, %add, "forwarding subscription to forward connection");

        // Register ACK and send
        let rx = self.register_ack(sub_id);
        if let Err(e) = mp.send_msg(msg.clone(), out_conn).await {
            self.remove_ack(sub_id);
            return Err(e);
        }

        // Track in remote subscription table (for reconnection replay) — both add and remove.
        let source = msg.get_source();
        let dst = msg.get_dst();
        let identity = msg.get_identity();
        mp.forwarder()
            .on_forwarded_subscription(source, dst, identity, out_conn, add, sub_id);

        // Wait for ACK with retry
        let result = self
            .wait_for_ack_with_retry(sub_id, msg.clone(), out_conn, rx)
            .await;

        self.remove_ack(sub_id);
        result
    }

    /// Wait for a remote ACK with retries.
    async fn wait_for_ack_with_retry(
        &self,
        _sub_id: u64,
        msg: Message,
        out_conn: u64,
        mut rx: oneshot::Receiver<Result<(), DataPathError>>,
    ) -> Result<(), DataPathError> {
        let mp = &self.inner.message_processor;
        for attempt in 0..=ACK_MAX_RETRIES {
            tokio::select! {
                result = &mut rx => {
                    return match result {
                        Ok(r) => r,
                        Err(_) => Err(DataPathError::RemoteSubscriptionAckTimeout(attempt)),
                    };
                }
                _ = tokio::time::sleep(ACK_TIMEOUT) => {
                    if attempt < ACK_MAX_RETRIES {
                        debug!(attempt = attempt + 1, "remote sub ack timeout, retrying");
                        mp.send_msg(msg.clone(), out_conn).await?;
                    }
                }
            }
        }

        Err(DataPathError::RemoteSubscriptionAckTimeout(ACK_MAX_RETRIES))
    }

    /// Best-effort unsubscribe to peers only (used by connection-drop path).
    /// Does not wait for ACKs. Looks up stored per-peer sub_ids.
    pub async fn notify_peers_unsubscribe(&self, name: &ProtoName) {
        // Take per-peer entries without holding lock across await
        let peer_entries: Vec<(u64, u64)> = {
            let mut fwd = self.inner.forwarded_subs.write();
            match fwd.remove(name) {
                Some(entries) => entries.into_iter().collect(),
                None => return,
            }
        };

        let peer_conns = self.inner.peer_conns.read().clone();
        if peer_conns.is_empty() {
            return;
        }

        debug!(%name, entries = ?peer_entries, "notifying peers of unsubscription (best-effort)");

        let mp = &self.inner.message_processor;
        for (conn_id, sub_id) in peer_entries {
            if !peer_conns.contains(&conn_id) {
                continue;
            }
            let msg = match sync::build_unsubscribe_msg(name, sub_id) {
                Ok(m) => m,
                Err(e) => {
                    warn!(%conn_id, error = %e, "failed to build unsubscribe for peer notify");
                    continue;
                }
            };
            if let Err(e) = mp.send_msg(msg, conn_id).await {
                warn!(%conn_id, error = %e, "failed to send unsubscribe to peer (best-effort)");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message_processing::MessageProcessor;

    fn make_forwarder() -> SubscriptionForwarder {
        let mp = MessageProcessor::new();
        mp.subscription_forwarder().unwrap()
    }

    #[tokio::test]
    async fn test_register_and_resolve_delivers_ok() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(1);
        fwd.resolve_ack(1, Ok(()));
        let result = rx.await.expect("sender dropped unexpectedly");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_and_resolve_delivers_err() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(2);
        fwd.resolve_ack(
            2,
            Err(DataPathError::RemoteSubscriptionAckError("boom".into())),
        );
        let result = rx.await.expect("sender dropped unexpectedly");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_id_is_noop() {
        let fwd = make_forwarder();
        let mut rx = fwd.register_ack(3);
        // Resolve a different (unknown) id — must not affect the registered one.
        fwd.resolve_ack(4, Ok(()));
        assert!(
            rx.try_recv().is_err(),
            "registered channel must not have received anything"
        );
    }

    #[test]
    fn test_remove_cleans_up() {
        let fwd = make_forwarder();
        fwd.register_ack(5);
        assert!(fwd.inner.pending_acks.read().contains_key(&5));
        fwd.remove_ack(5);
        assert!(!fwd.inner.pending_acks.read().contains_key(&5));
    }

    #[test]
    fn test_peer_conns_management() {
        let fwd = make_forwarder();
        assert!(fwd.peer_conns().is_empty());

        fwd.add_peer_conn(10);
        fwd.add_peer_conn(20);
        assert_eq!(fwd.peer_conns(), vec![10, 20]);

        // Duplicate add is idempotent
        fwd.add_peer_conn(10);
        assert_eq!(fwd.peer_conns(), vec![10, 20]);

        fwd.remove_peer_conn(10);
        assert_eq!(fwd.peer_conns(), vec![20]);
    }
}

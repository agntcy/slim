// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Peer synchronization component.
//!
//! Handles:
//! - Peer lifecycle (discovery, connect/disconnect, state tracking)
//! - Subscription forwarding to peers (full mesh broadcast or hub relay)
//! - Subscription forwarding to the controller (forward connection)
//! - In-flight ACK tracking and retry
//! - Loop prevention via seen subscription IDs

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use display_error_chain::ErrorChainExt;
use parking_lot::RwLock;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::api::ProtoName;
use crate::api::proto::dataplane::v1::Message;
use crate::errors::DataPathError;
use crate::message_processing::MessageProcessor;
use crate::messages::utils::DEFAULT_TTL;
use crate::peer_discovery::config::PeerTopology;
use crate::peer_discovery::{PeerDiscovery, PeerEvent, PeerInfo};
use crate::sync::state::{PeerEntry, PeerState};

use super::peer;

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

/// Configuration for peer synchronization.
#[derive(Debug, Clone)]
pub struct PeerSyncConfig {
    /// This replica's unique identifier.
    pub self_id: String,
    /// Shared group identifier for peer authentication.
    pub deployment_name: String,
    /// Topology for peer connections.
    pub topology: PeerTopology,
}

/// Peer synchronization and subscription forwarding.
///
/// This is the single component responsible for:
/// - Tracking peer connections and state
/// - Forwarding subscriptions to peers and the controller
/// - Managing the peer discovery lifecycle (via `start_discovery`)
/// - Handling incoming peer registration
/// - ACK tracking for forwarded subscriptions
///
/// Shared via `Arc` between the discovery task and `MessageProcessor`.
#[derive(Debug, Clone)]
pub struct PeerSync {
    inner: Arc<PeerSyncInner>,
}

/// State for a pending multi-peer ACK.
/// Tracks how many ACKs are expected and resolves once all arrive or on timeout.
struct PendingAck {
    /// Number of ACKs still expected.
    remaining: usize,
    /// Sender to signal completion to the waiting task.
    tx: Option<oneshot::Sender<Result<(), DataPathError>>>,
    /// Collects all errors received from peers.
    errors: Vec<DataPathError>,
}

impl std::fmt::Debug for PendingAck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAck")
            .field("remaining", &self.remaining)
            .field("tx", &self.tx.is_some())
            .field("errors", &self.errors.len())
            .finish()
    }
}

#[derive(Debug)]
struct PeerSyncInner {
    /// Current peer connection IDs (updated on join/leave).
    peer_conns: RwLock<HashSet<u64>>,
    /// Set of subscription IDs this node has already forwarded or processed.
    /// Used for loop prevention: if an incoming subscription has an ID in this set,
    /// it means the subscription has looped back and should be dropped.
    seen_sub_ids: RwLock<HashSet<u64>>,
    /// Tracks which sub_id was forwarded to peers for each name.
    /// Needed for the connection-drop path: when a connection drops and a name
    /// becomes unreachable, we only have the name — this map gives us the sub_id
    /// to send in the unsubscribe message.
    forwarded_sub_for_name: RwLock<HashMap<ProtoName, u64>>,
    /// In-flight pending ACK state: sub_id → pending ack tracker.
    pending_acks: RwLock<HashMap<u64, PendingAck>>,
    /// TTL to set on subscription messages forwarded to peers.
    subscription_ttl: u32,
    /// Filter to apply when syncing subscriptions with a new peer.
    /// FullMesh → EXCLUDE_PEER (peers get subs from each other directly).
    /// HubAndSpoke / standalone → ALL.
    sync_filter: crate::tables::MatchFilter,
    /// Shared peer state (if discovery is active).
    /// Used to register incoming peers directly without a channel.
    peer_state: Option<Arc<RwLock<PeerState>>>,
    /// All peer IDs ever discovered (used for hub election in HubAndSpoke).
    /// Unlike peer_state which only tracks connected peers, this tracks
    /// every peer ID we've heard about from discovery.
    discovered_peers: RwLock<HashSet<String>>,
}

impl PeerSync {
    /// Create a new PeerSync.
    pub fn new(topology: &PeerTopology) -> Self {
        let (subscription_ttl, sync_filter) = match topology {
            PeerTopology::FullMesh => (2, crate::tables::MatchFilter::EXCLUDE_PEER),
            PeerTopology::HubAndSpoke => (3, crate::tables::MatchFilter::ALL),
        };
        Self {
            inner: Arc::new(PeerSyncInner {
                peer_conns: RwLock::new(HashSet::new()),
                seen_sub_ids: RwLock::new(HashSet::new()),
                forwarded_sub_for_name: RwLock::new(HashMap::new()),
                pending_acks: RwLock::new(HashMap::new()),
                subscription_ttl,
                sync_filter,
                peer_state: None,
                discovered_peers: RwLock::new(HashSet::new()),
            }),
        }
    }

    /// Create a PeerSync with shared peer state (for discovery mode).
    pub fn with_peer_state(topology: &PeerTopology, peer_state: Arc<RwLock<PeerState>>) -> Self {
        let (subscription_ttl, sync_filter) = match topology {
            PeerTopology::FullMesh => (2, crate::tables::MatchFilter::EXCLUDE_PEER),
            PeerTopology::HubAndSpoke => (3, crate::tables::MatchFilter::ALL),
        };
        Self {
            inner: Arc::new(PeerSyncInner {
                peer_conns: RwLock::new(HashSet::new()),
                seen_sub_ids: RwLock::new(HashSet::new()),
                forwarded_sub_for_name: RwLock::new(HashMap::new()),
                pending_acks: RwLock::new(HashMap::new()),
                subscription_ttl,
                sync_filter,
                peer_state: Some(peer_state),
                discovered_peers: RwLock::new(HashSet::new()),
            }),
        }
    }

    /// Create a standalone PeerSync (no discovery, no peer state).
    /// Uses DEFAULT_TTL for generic multi-hop topologies.
    /// Peer connections are auto-registered during link negotiation.
    pub fn standalone() -> Self {
        Self {
            inner: Arc::new(PeerSyncInner {
                peer_conns: RwLock::new(HashSet::new()),
                seen_sub_ids: RwLock::new(HashSet::new()),
                forwarded_sub_for_name: RwLock::new(HashMap::new()),
                pending_acks: RwLock::new(HashMap::new()),
                subscription_ttl: DEFAULT_TTL,
                sync_filter: crate::tables::MatchFilter::ALL,
                peer_state: None,
                discovered_peers: RwLock::new(HashSet::new()),
            }),
        }
    }

    /// Update the peer connection set (called by PeerSyncManager on join/leave).
    pub fn set_peer_conns(&self, conns: HashSet<u64>) {
        *self.inner.peer_conns.write() = conns;
    }

    /// Add a peer connection ID.
    pub fn add_peer_conn(&self, conn_id: u64) {
        self.inner.peer_conns.write().insert(conn_id);
    }

    /// Remove a peer connection ID.
    pub fn remove_peer_conn(&self, conn_id: u64) {
        self.inner.peer_conns.write().remove(&conn_id);
    }

    /// Get a snapshot of the current peer connection set.
    pub fn peer_conns(&self) -> HashSet<u64> {
        self.inner.peer_conns.read().clone()
    }

    /// Resolve a connection ID to the remote peer's node name (for logging).
    /// Returns the node_id if available, otherwise the conn_id as a string.
    fn peer_label(&self, mp: &MessageProcessor, conn_id: u64) -> String {
        mp.forwarder()
            .get_connection(conn_id)
            .and_then(|c| c.peer_node_id().map(|s| s.to_string()))
            .unwrap_or_else(|| conn_id.to_string())
    }

    /// The initial TTL set on subscription messages forwarded to peers.
    pub fn subscription_ttl(&self) -> u32 {
        self.inner.subscription_ttl
    }

    /// Whether a PeerSyncManager is active (peer state is shared).
    pub fn has_peer_state(&self) -> bool {
        self.inner.peer_state.is_some()
    }

    /// Handle an incoming peer connection: register in state, add to peer conns,
    /// and perform subscription sync.
    ///
    /// This is the single entry point for incoming peer registration from message processing.
    pub fn on_incoming_peer(&self, mp: &MessageProcessor, node_id: String, conn_id: u64) {
        // Register in peer state (dedup, reconnect awareness).
        if let Some(ref state) = self.inner.peer_state {
            if state.read().contains(&node_id) {
                debug!(
                    %node_id,
                    %conn_id,
                    "incoming peer already registered, skipping"
                );
                return;
            }
            info!(
                %node_id,
                %conn_id,
                "registering incoming peer in state table"
            );
            state.write().insert(
                node_id,
                PeerEntry {
                    conn_id,
                    endpoint: String::new(),
                    is_outgoing: false,
                },
            );
        }

        self.add_peer_conn_and_sync(mp, conn_id);
    }

    /// Register a peer connection and perform full sync (send local subscriptions).
    /// Used by PeerSyncManager for outgoing connections (state already tracked by manager).
    pub fn add_peer_conn_and_sync(&self, mp: &MessageProcessor, conn_id: u64) {
        self.add_peer_conn(conn_id);
        let forwarder = self.clone();
        let mp = mp.clone();
        tokio::spawn(async move {
            let ttl = forwarder.inner.subscription_ttl;
            let filter = forwarder.inner.sync_filter;
            let subscriptions = peer::collect_subscriptions(&mp, conn_id, filter);
            match peer::send_subscriptions(&mp, conn_id, &subscriptions, ttl).await {
                Ok(count) => {
                    info!(
                        %conn_id,
                        count,
                        "completed full sync for peer"
                    );
                    for (name, sub_id) in &subscriptions {
                        forwarder.register_forwarded_sub(name, *sub_id);
                    }
                }
                Err(e) => {
                    warn!(%conn_id, error = %e, "full sync failed for peer");
                }
            }
        });
    }

    // ── Peer Discovery Lifecycle ─────────────────────────────────────────────

    /// Start peer discovery and run the event loop until cancellation.
    ///
    /// This spawns nothing — it runs as an async loop. The caller is expected
    /// to spawn this (e.g., via `tokio::spawn`).
    pub async fn run_discovery<D: PeerDiscovery + Send>(
        &self,
        mp: &MessageProcessor,
        config: PeerSyncConfig,
        mut discovery: D,
        cancel: CancellationToken,
    ) {
        info!(
            self_id = %config.self_id,
            deployment_name = %config.deployment_name,
            "peer sync starting"
        );

        if let Err(e) = discovery.start().await {
            error!(error = %e, "failed to start peer discovery");
            return;
        }

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("peer sync shutting down");
                    break;
                }
                event = discovery.recv() => {
                    match event {
                        Ok(PeerEvent::Joined(peer)) => {
                            self.handle_peer_joined(mp, &config, peer).await;
                        }
                        Ok(PeerEvent::Left(peer)) => {
                            self.handle_peer_left(mp, peer).await;
                        }
                        Err(e) => {
                            error!(error = %e, "peer discovery error, shutting down");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Handle a newly discovered peer.
    ///
    /// Connection behavior depends on topology:
    /// - **FullMesh**: only the node with the lexicographically smaller self_id
    ///   initiates the outbound connection (tie-breaking for deduplication).
    /// - **HubAndSpoke**: only the hub (smallest ID) initiates connections.
    ///   Spokes never dial out.
    async fn handle_peer_joined(
        &self,
        mp: &MessageProcessor,
        config: &PeerSyncConfig,
        peer: PeerInfo,
    ) {
        // Skip self
        if peer.id == config.self_id {
            debug!(peer_id = %peer.id, "skipping self in peer discovery");
            return;
        }

        // Track all discovered peers (for hub election in HubAndSpoke).
        self.inner.discovered_peers.write().insert(peer.id.clone());

        // Determine whether to dial based on topology.
        let should_dial = match config.topology {
            PeerTopology::FullMesh => {
                // In full mesh, only the node with the smaller ID initiates
                // each pairwise connection (avoids duplicate links).
                config.self_id < peer.id
            }
            PeerTopology::HubAndSpoke => {
                // In hub-and-spoke, only the hub (globally smallest ID) dials.
                // A node is the hub if it has a smaller ID than ALL discovered peers.
                !self
                    .inner
                    .discovered_peers
                    .read()
                    .iter()
                    .any(|p| p.as_str() < config.self_id.as_str())
            }
        };

        if !should_dial {
            debug!(
                peer_id = %peer.id,
                self_id = %config.self_id,
                topology = ?config.topology,
                "skipping outbound connection (waiting for incoming)"
            );
            return;
        }

        // Skip if already connected
        if let Some(ref state) = self.inner.peer_state
            && state.read().contains(&peer.id)
        {
            debug!(peer_id = %peer.id, "peer already connected, skipping");
            return;
        }

        info!(peer_id = %peer.id, endpoint = %peer.config.endpoint, "connecting to peer");

        match mp.connect(peer.config.clone(), None, None).await {
            Ok((_handle, conn_id)) => {
                info!(peer_id = %peer.id, %conn_id, "connected to peer");

                if let Some(ref state) = self.inner.peer_state {
                    state.write().insert(
                        peer.id.clone(),
                        PeerEntry {
                            conn_id,
                            endpoint: peer.config.endpoint.clone(),
                            is_outgoing: true,
                        },
                    );
                }

                self.add_peer_conn(conn_id);

                // Perform full sync: send subscriptions to the new peer.
                // In HubAndSpoke, only the hub dials (self_id < peer.id), so if we
                // reached here with HubAndSpoke topology, we are the hub.
                let ttl = self.inner.subscription_ttl;
                let is_hub = config.self_id < peer.id;
                let sync_result = if is_hub && config.topology == PeerTopology::HubAndSpoke {
                    peer::send_full_sync(mp, conn_id, ttl).await
                } else {
                    peer::send_local_remote_sync(mp, conn_id, ttl).await
                };
                if let Err(e) = sync_result {
                    warn!(
                        peer_id = %peer.id,
                        error = %e,
                        "full sync failed after connecting to peer"
                    );
                }
            }
            Err(e) => {
                error!(
                    peer_id = %peer.id,
                    endpoint = %peer.config.endpoint,
                    error = %e.chain(),
                    "failed to connect to peer"
                );
            }
        }
    }

    /// Handle a peer leaving the deployment.
    async fn handle_peer_left(&self, mp: &MessageProcessor, peer: PeerInfo) {
        let entry = self
            .inner
            .peer_state
            .as_ref()
            .and_then(|s| s.write().remove(&peer.id));

        if let Some(entry) = entry {
            info!(
                peer_id = %peer.id,
                conn_id = entry.conn_id,
                "peer left, disconnecting"
            );

            self.remove_peer_conn(entry.conn_id);
            if let Err(e) = mp.disconnect(entry.conn_id) {
                warn!(
                    peer_id = %peer.id,
                    error = %e,
                    "error disconnecting from peer"
                );
            }
        }
    }

    /// Notify that a connection was dropped. Cleans up peer state if it was a peer.
    pub fn on_connection_drop(&self, conn_id: u64) {
        if let Some(ref state) = self.inner.peer_state {
            // Look up peer info under a read lock.
            let peer_info = {
                let s = state.read();
                s.peer_id_for_conn(conn_id)
                    .and_then(|id| s.get(id).map(|entry| (id.to_string(), entry.is_outgoing)))
            };

            if let Some((peer_id, is_outgoing)) = peer_info {
                if is_outgoing {
                    // For outgoing peer connections, keep the peer_state entry.
                    // The discovery-driven handle_peer_left will clean it up
                    // and call mp.disconnect() to stop reconnection.
                    debug!(
                        %peer_id,
                        %conn_id,
                        "outgoing peer connection dropped, awaiting discovery Left event"
                    );
                } else {
                    // For incoming connections, clean up immediately.
                    state.write().remove_by_conn(conn_id);
                    info!(
                        %peer_id,
                        %conn_id,
                        "incoming peer connection dropped, cleaned up state"
                    );
                    self.remove_peer_conn(conn_id);
                }
            }
        }
    }

    /// Get the current peer state (for testing/inspection).
    pub fn peer_state(&self) -> Option<Arc<RwLock<PeerState>>> {
        self.inner.peer_state.clone()
    }

    // ── Subscription Tracking ────────────────────────────────────────────────

    /// Register a subscription that was forwarded (tracks sub_id for unsubscribe
    /// and adds to seen set for loop prevention).
    /// Mark a subscription as forwarded: adds to seen set (loop prevention)
    /// and records the name→sub_id mapping (for connection-drop unsubscribe).
    pub fn register_forwarded_sub(&self, name: &ProtoName, sub_id: u64) {
        self.inner.seen_sub_ids.write().insert(sub_id);
        self.inner
            .forwarded_sub_for_name
            .write()
            .insert(name.clone(), sub_id);
    }

    /// Remove a subscription from the seen set (on unsubscribe).
    pub fn remove_forwarded_sub(&self, name: &ProtoName, sub_id: u64) {
        self.inner.seen_sub_ids.write().remove(&sub_id);
        self.inner.forwarded_sub_for_name.write().remove(name);
    }

    /// Check if a subscription_id has already been seen/forwarded by this node.
    /// Used for loop prevention.
    pub fn has_seen_sub_id(&self, sub_id: u64) -> bool {
        self.inner.seen_sub_ids.read().contains(&sub_id)
    }

    // ── ACK tracking ─────────────────────────────────────────────────────────

    /// Register a multi-peer ACK; returns the result receiver.
    /// `expected_count` is the number of ACKs expected before resolving.
    pub(crate) fn register_ack(
        &self,
        ack_id: u64,
        expected_count: usize,
    ) -> oneshot::Receiver<Result<(), DataPathError>> {
        let (tx, rx) = oneshot::channel();
        self.inner.pending_acks.write().insert(
            ack_id,
            PendingAck {
                remaining: expected_count,
                tx: Some(tx),
                errors: Vec::new(),
            },
        );
        rx
    }

    /// Deliver a result for one ACK. When all expected ACKs have arrived,
    /// the waiting task is unblocked with the aggregate result.
    /// Called from the message processing path when a SubscriptionAck arrives.
    pub(crate) fn resolve_ack(&self, ack_id: u64, result: Result<(), DataPathError>) {
        let mut acks = self.inner.pending_acks.write();
        let Entry::Occupied(mut entry) = acks.entry(ack_id) else {
            return;
        };

        let pending = entry.get_mut();
        debug!(%ack_id, remaining = pending.remaining, "subscription: remote ack received");

        if let Err(e) = result {
            pending.errors.push(e);
        }

        pending.remaining = pending.remaining.saturating_sub(1);
        if pending.remaining == 0 {
            let pending = entry.remove();
            if let Some(tx) = pending.tx {
                let final_result = if pending.errors.is_empty() {
                    Ok(())
                } else {
                    let msg = pending
                        .errors
                        .iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    Err(DataPathError::RemoteSubscriptionAckError(msg))
                };
                let _ = tx.send(final_result);
            }
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
    /// 1. Sends to target peers (same sub_id) + forward_conn (original msg)
    /// 2. Waits for ACKs with timeout
    /// 3. ACKs the upstream client based on aggregate result
    ///
    /// The task is wrapped with a drain watch so it stops promptly on shutdown.
    ///
    /// `peer_ttl` is the TTL set on outgoing subscription messages to peers.
    /// For initial forwarding (local sub), use `subscription_ttl()`.
    /// For relay (peer sub), use the remaining TTL from the incoming message.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_forward_and_ack(
        &self,
        mp: MessageProcessor,
        msg: Message,
        name: ProtoName,
        sub_id: u64,
        add: bool,
        targets: ForwardTargets,
        in_connection: u64,
        upstream_subscription_id: Option<u64>,
        peer_ttl: u32,
        drain: drain::Watch,
    ) {
        let forwarder = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = forwarder.forward_and_ack(
                    &mp,
                    msg,
                    name,
                    sub_id,
                    add,
                    targets,
                    in_connection,
                    upstream_subscription_id,
                    peer_ttl,
                ) => {}
                _ = drain.signaled() => {
                    debug!(%in_connection, %sub_id, "subscription forwarder stopped by drain");
                }
            }
        });
    }

    /// The actual forwarding + ACK lifecycle (runs in spawned task).
    #[allow(clippy::too_many_arguments)]
    async fn forward_and_ack(
        &self,
        mp: &MessageProcessor,
        msg: Message,
        name: ProtoName,
        sub_id: u64,
        add: bool,
        targets: ForwardTargets,
        in_connection: u64,
        upstream_subscription_id: Option<u64>,
        peer_ttl: u32,
    ) {
        // Run peer forwarding and forward-conn forwarding concurrently.
        let (peer_result, forward_result) = tokio::join!(
            self.forward_to_peers(mp, &name, sub_id, add, &targets, peer_ttl),
            self.forward_to_conn(mp, &msg, sub_id, add, &targets),
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
    /// For subscribe: uses the subscription's stable sub_id, sends to all target peers,
    /// waits for ACKs from all peers with timeout.
    ///
    /// For unsubscribe: looks up the previously stored sub_id for the name and
    /// sends unsubscribe best-effort (no ACK wait).
    ///
    /// `sub_id` is the stable subscription identifier (same across all hops).
    /// `ttl` is set on the outgoing subscription messages (controls propagation depth).
    async fn forward_to_peers(
        &self,
        mp: &MessageProcessor,
        name: &ProtoName,
        sub_id: u64,
        add: bool,
        targets: &ForwardTargets,
        ttl: u32,
    ) -> Result<(), DataPathError> {
        let peer_target = match &targets.peers {
            Some(t) => t,
            None => return Ok(()),
        };

        // Snapshot peer conn IDs (don't hold lock across await)
        let peer_conns: Vec<u64> = {
            let conns = self.inner.peer_conns.read();
            match peer_target {
                PeerTarget::All => conns.iter().copied().collect(),
                PeerTarget::ExcludeConn(exclude) => {
                    conns.iter().copied().filter(|c| c != exclude).collect()
                }
            }
        };

        if peer_conns.is_empty() {
            debug!(%name, "no peer connections, skipping peer forwarding");
            return Ok(());
        }

        let action = if add { "subscribe" } else { "unsubscribe" };
        debug!(%name, %sub_id, %action, ?peer_conns, "forwarding to peers");

        // Build the appropriate message.
        let build_result = if add {
            self.register_forwarded_sub(name, sub_id);
            super::build_subscribe_msg(name, sub_id, ttl)
        } else {
            self.remove_forwarded_sub(name, sub_id);
            super::build_unsubscribe_msg(name, sub_id, ttl)
        };

        let peer_msg = match build_result {
            Ok(m) => m,
            Err(e) => {
                warn!(%action, error = %e, %name, "failed to build peer message");
                return Err(e.into());
            }
        };

        // Send to all target peers concurrently.
        let send_results = futures::future::join_all(peer_conns.iter().map(|&conn_id| {
            let msg = peer_msg.clone();
            async move { (conn_id, mp.send_msg(msg, conn_id).await) }
        }))
        .await;

        // Count successes; log failures.
        let mut sent_count = 0usize;
        for (conn_id, result) in &send_results {
            if let Err(e) = result {
                let peer = self.peer_label(mp, *conn_id);
                warn!(%conn_id, %peer, error = %e, "failed to send to peer");
            } else {
                sent_count += 1;
            }
        }

        if sent_count == 0 {
            return Ok(());
        }

        // Wait for ACKs with timeout.
        let rx = self.register_ack(sub_id, sent_count);
        match tokio::time::timeout(ACK_TIMEOUT, rx).await {
            Ok(Ok(result)) => {
                if let Err(e) = &result {
                    warn!(%name, %sub_id, error = %e, "peer ACK aggregated failure");
                }
            }
            Ok(Err(_)) => {
                debug!(%name, %sub_id, "peer ACK sender dropped");
            }
            Err(_) => {
                warn!(%name, %sub_id, "peer ACK timeout");
                self.remove_ack(sub_id);
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
        mp: &MessageProcessor,
        msg: &Message,
        sub_id: u64,
        add: bool,
        targets: &ForwardTargets,
    ) -> Result<(), DataPathError> {
        let out_conn = match targets.forward_conn {
            Some(c) => c,
            None => return Ok(()),
        };

        debug!(%out_conn, %add, "forwarding subscription to forward connection");

        // Register ACK and send
        let rx = self.register_ack(sub_id, 1);
        if let Err(e) = mp.send_msg(msg.clone(), out_conn).await {
            self.remove_ack(sub_id);
            return Err(e);
        }

        // Track in remote subscription table (for reconnection replay) — both add and remove.
        let source = msg.get_source();
        let dst = msg.get_dst();
        let identity = msg.get_identity();
        mp.remote_sync()
            .on_forwarded_subscription(source, dst, identity, out_conn, add, sub_id);

        // Wait for ACK with retry
        let result = self
            .wait_for_ack_with_retry(mp, sub_id, msg.clone(), out_conn, rx)
            .await;

        self.remove_ack(sub_id);
        result
    }

    /// Wait for a remote ACK with retries.
    async fn wait_for_ack_with_retry(
        &self,
        mp: &MessageProcessor,
        _sub_id: u64,
        msg: Message,
        out_conn: u64,
        mut rx: oneshot::Receiver<Result<(), DataPathError>>,
    ) -> Result<(), DataPathError> {
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
    /// Does not wait for ACKs. Looks up the forwarded sub_id for the name.
    pub async fn notify_peers_unsubscribe(&self, mp: &MessageProcessor, name: &ProtoName) {
        // Look up the sub_id that was actually forwarded for this name.
        let sub_id = match self.inner.forwarded_sub_for_name.read().get(name).copied() {
            Some(id) => id,
            None => return, // Never forwarded — nothing to unsubscribe
        };

        let targets = ForwardTargets {
            peers: Some(PeerTarget::All),
            forward_conn: None,
        };
        if let Err(e) = self
            .forward_to_peers(
                mp,
                name,
                sub_id,
                false,
                &targets,
                self.inner.subscription_ttl,
            )
            .await
        {
            warn!(%name, %sub_id, error = %e, "failed to notify peers of unsubscription");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use slim_config::client::ClientConfig;
    use tokio_util::sync::CancellationToken;

    use crate::message_processing::MessageProcessor;
    use crate::peer_discovery::{PeerDiscovery, PeerDiscoveryError, PeerEvent, PeerInfo};

    fn make_forwarder() -> PeerSync {
        let mp = MessageProcessor::new();
        mp.peer_sync()
    }

    fn make_forwarder_with_state() -> PeerSync {
        PeerSync::with_peer_state(
            &PeerTopology::FullMesh,
            Arc::new(RwLock::new(PeerState::new())),
        )
    }

    fn make_name() -> ProtoName {
        ProtoName::from_strings(["org", "example", "svc"])
    }

    fn make_peer_info(id: &str) -> PeerInfo {
        PeerInfo {
            id: id.to_string(),
            config: ClientConfig {
                endpoint: "http://127.0.0.1:9999".to_string(),
                ..Default::default()
            },
        }
    }

    // ── ACK tests ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_register_and_resolve_delivers_ok() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(1, 1);
        fwd.resolve_ack(1, Ok(()));
        let result = rx.await.expect("sender dropped unexpectedly");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_and_resolve_delivers_err() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(2, 1);
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
        let mut rx = fwd.register_ack(3, 1);
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
        fwd.register_ack(5, 1);
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
        assert_eq!(fwd.peer_conns(), HashSet::from([10, 20]));

        // Duplicate add is idempotent
        fwd.add_peer_conn(10);
        assert_eq!(fwd.peer_conns(), HashSet::from([10, 20]));

        fwd.remove_peer_conn(10);
        assert_eq!(fwd.peer_conns(), HashSet::from([20]));
    }

    // ── Multi-peer ACK aggregation ──────────────────────────────────────────

    #[tokio::test]
    async fn test_multi_ack_all_ok() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(10, 3);
        fwd.resolve_ack(10, Ok(()));
        fwd.resolve_ack(10, Ok(()));
        fwd.resolve_ack(10, Ok(()));
        let result = rx.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_multi_ack_partial_error() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(11, 2);
        fwd.resolve_ack(11, Ok(()));
        fwd.resolve_ack(
            11,
            Err(DataPathError::RemoteSubscriptionAckError(
                "peer-fail".into(),
            )),
        );
        let result = rx.await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multi_ack_all_errors() {
        let fwd = make_forwarder();
        let rx = fwd.register_ack(12, 2);
        fwd.resolve_ack(
            12,
            Err(DataPathError::RemoteSubscriptionAckError("e1".into())),
        );
        fwd.resolve_ack(
            12,
            Err(DataPathError::RemoteSubscriptionAckError("e2".into())),
        );
        let result = rx.await.unwrap();
        assert!(result.is_err());
    }

    // ── Forwarded sub tracking ──────────────────────────────────────────────

    #[test]
    fn test_forwarded_sub_tracking() {
        let fwd = make_forwarder();
        let name = make_name();

        assert!(!fwd.has_seen_sub_id(42));
        fwd.register_forwarded_sub(&name, 42);
        assert!(fwd.has_seen_sub_id(42));

        // name → sub_id lookup
        assert_eq!(
            fwd.inner.forwarded_sub_for_name.read().get(&name).copied(),
            Some(42)
        );

        fwd.remove_forwarded_sub(&name, 42);
        assert!(!fwd.has_seen_sub_id(42));
        assert!(fwd.inner.forwarded_sub_for_name.read().get(&name).is_none());
    }

    // ── forward_to_peers ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_forward_to_peers_no_target() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let targets = ForwardTargets {
            peers: None,
            forward_conn: None,
        };
        // No peer target → early return Ok
        let result = fwd.forward_to_peers(&mp, &name, 1, true, &targets, 2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_forward_to_peers_no_conns() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let targets = ForwardTargets {
            peers: Some(PeerTarget::All),
            forward_conn: None,
        };
        // Peer target set but no peer conns → early return Ok
        let result = fwd.forward_to_peers(&mp, &name, 1, true, &targets, 2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_forward_to_peers_send_failure_still_ok() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();

        // Add peer conns that don't actually exist in the forwarder table
        fwd.add_peer_conn(100);
        fwd.add_peer_conn(200);

        let targets = ForwardTargets {
            peers: Some(PeerTarget::All),
            forward_conn: None,
        };

        // send_msg will fail since conns 100/200 don't exist → sends fail,
        // sent_count == 0, returns Ok (no ACK wait needed)
        let result = fwd
            .forward_to_peers(&mp, &name, 50, true, &targets, 2)
            .await;
        assert!(result.is_ok());
        // Despite failure, the sub was registered in seen set
        assert!(fwd.has_seen_sub_id(50));
    }

    #[tokio::test]
    async fn test_forward_to_peers_exclude_conn() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();

        fwd.add_peer_conn(100);
        fwd.add_peer_conn(200);

        let targets = ForwardTargets {
            peers: Some(PeerTarget::ExcludeConn(100)),
            forward_conn: None,
        };

        // Only conn 200 will be targeted (and fail since it doesn't exist)
        let result = fwd
            .forward_to_peers(&mp, &name, 51, true, &targets, 2)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_forward_to_peers_unsubscribe() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();

        // Pre-register to test removal path
        fwd.register_forwarded_sub(&name, 60);
        assert!(fwd.has_seen_sub_id(60));

        fwd.add_peer_conn(100);
        let targets = ForwardTargets {
            peers: Some(PeerTarget::All),
            forward_conn: None,
        };

        let result = fwd
            .forward_to_peers(&mp, &name, 60, false, &targets, 2)
            .await;
        assert!(result.is_ok());
        // Unsubscribe removes from seen set
        assert!(!fwd.has_seen_sub_id(60));
    }

    // ── forward_to_conn ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_forward_to_conn_no_target() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 70, 2).unwrap();
        let targets = ForwardTargets {
            peers: None,
            forward_conn: None,
        };
        let result = fwd.forward_to_conn(&mp, &msg, 70, true, &targets).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_forward_to_conn_send_failure() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 71, 2).unwrap();
        let targets = ForwardTargets {
            peers: None,
            forward_conn: Some(999), // non-existent connection
        };
        let result = fwd.forward_to_conn(&mp, &msg, 71, true, &targets).await;
        assert!(result.is_err());
        // ACK should have been cleaned up
        assert!(!fwd.inner.pending_acks.read().contains_key(&71));
    }

    // ── forward_and_ack ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_forward_and_ack_no_targets() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 80, 2).unwrap();
        let targets = ForwardTargets::none();
        // No targets → both return Ok, no upstream ACK sent (None)
        fwd.forward_and_ack(&mp, msg, name, 80, true, targets, 1, None, 2)
            .await;
    }

    #[tokio::test]
    async fn test_forward_and_ack_peer_failure_nonfatal() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 81, 2).unwrap();

        // peer target with fake conns → send fails → peer error is non-fatal
        fwd.add_peer_conn(300);
        let targets = ForwardTargets {
            peers: Some(PeerTarget::All),
            forward_conn: None,
        };
        fwd.forward_and_ack(&mp, msg, name, 81, true, targets, 1, None, 2)
            .await;
        // Should not panic; peer failure is logged but not propagated
    }

    #[tokio::test]
    async fn test_forward_and_ack_forward_conn_failure() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 82, 2).unwrap();

        let targets = ForwardTargets {
            peers: None,
            forward_conn: Some(999), // non-existent
        };
        // forward_conn failure → upstream ACK should NOT be sent (no upstream_subscription_id)
        fwd.forward_and_ack(&mp, msg, name, 82, true, targets, 1, None, 2)
            .await;
    }

    // ── notify_peers_unsubscribe ────────────────────────────────────────────

    #[tokio::test]
    async fn test_notify_peers_unsubscribe_unknown_name() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        // Never forwarded → should return immediately
        fwd.notify_peers_unsubscribe(&mp, &name).await;
    }

    #[tokio::test]
    async fn test_notify_peers_unsubscribe_known_name() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();

        // Register the name first
        fwd.register_forwarded_sub(&name, 90);
        fwd.add_peer_conn(400); // fake conn → send will fail but we just check it doesn't panic

        fwd.notify_peers_unsubscribe(&mp, &name).await;
        // After unsubscribe, the seen set no longer contains this ID
        assert!(!fwd.has_seen_sub_id(90));
    }

    // ── on_connection_drop ──────────────────────────────────────────────────

    #[test]
    fn test_on_connection_drop_keeps_outgoing_peer_state() {
        let fwd = make_forwarder_with_state();
        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "peer-a".to_string(),
            PeerEntry {
                conn_id: 50,
                endpoint: "http://a".to_string(),
                is_outgoing: true,
            },
        );
        fwd.add_peer_conn(50);

        fwd.on_connection_drop(50);
        // Outgoing peer state is kept for discovery-driven cleanup
        assert!(state.read().contains("peer-a"));
    }

    #[test]
    fn test_on_connection_drop_removes_incoming_peer_state() {
        let fwd = make_forwarder_with_state();
        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "peer-a".to_string(),
            PeerEntry {
                conn_id: 50,
                endpoint: "http://a".to_string(),
                is_outgoing: false,
            },
        );
        fwd.add_peer_conn(50);

        fwd.on_connection_drop(50);
        // Incoming peer state is cleaned up immediately
        assert!(!fwd.peer_conns().contains(&50));
        assert!(!state.read().contains("peer-a"));
    }

    #[test]
    fn test_on_connection_drop_unknown_conn() {
        let fwd = make_forwarder_with_state();
        // Should not panic on unknown conn
        fwd.on_connection_drop(999);
    }

    // ── on_incoming_peer ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_on_incoming_peer_dedup() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();

        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "peer-a".to_string(),
            PeerEntry {
                conn_id: 10,
                endpoint: "http://a".to_string(),
                is_outgoing: false,
            },
        );

        // Second registration of same node_id should be a no-op
        fwd.on_incoming_peer(&mp, "peer-a".to_string(), 20);
        // conn_id should still be the original
        assert_eq!(state.read().conn_id("peer-a"), Some(10));
    }

    #[tokio::test]
    async fn test_on_incoming_peer_new() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();

        fwd.on_incoming_peer(&mp, "peer-b".to_string(), 30);
        let state = fwd.peer_state().unwrap();
        assert!(state.read().contains("peer-b"));
        assert!(fwd.peer_conns().contains(&30));
    }

    // ── ForwardTargets ──────────────────────────────────────────────────────

    #[test]
    fn test_forward_targets_has_any() {
        assert!(!ForwardTargets::none().has_any());
        assert!(
            ForwardTargets {
                peers: Some(PeerTarget::All),
                forward_conn: None,
            }
            .has_any()
        );
        assert!(
            ForwardTargets {
                peers: None,
                forward_conn: Some(1),
            }
            .has_any()
        );
    }

    // ── PeerSync constructors ───────────────────────────────────────────────

    #[test]
    fn test_standalone_constructor() {
        let fwd = PeerSync::standalone();
        assert_eq!(fwd.subscription_ttl(), DEFAULT_TTL);
        assert!(!fwd.has_peer_state());
    }

    #[test]
    fn test_fullmesh_constructor() {
        let fwd = PeerSync::new(&PeerTopology::FullMesh);
        assert_eq!(fwd.subscription_ttl(), 2);
        assert!(!fwd.has_peer_state());
    }

    #[test]
    fn test_hub_and_spoke_constructor() {
        let fwd = PeerSync::new(&PeerTopology::HubAndSpoke);
        assert_eq!(fwd.subscription_ttl(), 3);
    }

    #[test]
    fn test_with_peer_state_constructor() {
        let fwd = make_forwarder_with_state();
        assert!(fwd.has_peer_state());
    }

    // ── run_discovery ───────────────────────────────────────────────────────

    /// Mock discovery that yields a sequence of events then closes.
    struct MockDiscovery {
        events: Vec<Result<PeerEvent, PeerDiscoveryError>>,
        start_error: Option<PeerDiscoveryError>,
    }

    impl MockDiscovery {
        fn new(events: Vec<Result<PeerEvent, PeerDiscoveryError>>) -> Self {
            Self {
                events,
                start_error: None,
            }
        }

        fn with_start_error(err: PeerDiscoveryError) -> Self {
            Self {
                events: vec![],
                start_error: Some(err),
            }
        }
    }

    impl PeerDiscovery for MockDiscovery {
        async fn start(&mut self) -> Result<(), PeerDiscoveryError> {
            if let Some(e) = self.start_error.take() {
                Err(e)
            } else {
                Ok(())
            }
        }

        async fn recv(&mut self) -> Result<PeerEvent, PeerDiscoveryError> {
            if self.events.is_empty() {
                // Block forever (will be cancelled)
                std::future::pending().await
            } else {
                self.events.remove(0)
            }
        }
    }

    #[tokio::test]
    async fn test_run_discovery_start_error() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let cancel = CancellationToken::new();
        let config = PeerSyncConfig {
            self_id: "self".to_string(),
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        let discovery =
            MockDiscovery::with_start_error(PeerDiscoveryError::Backend("cannot start".into()));
        // Should return without panicking
        fwd.run_discovery(&mp, config, discovery, cancel).await;
    }

    #[tokio::test]
    async fn test_run_discovery_cancellation() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let cancel = CancellationToken::new();
        let config = PeerSyncConfig {
            self_id: "self".to_string(),
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        let discovery = MockDiscovery::new(vec![]);
        cancel.cancel();
        fwd.run_discovery(&mp, config, discovery, cancel).await;
    }

    #[tokio::test]
    async fn test_run_discovery_error_event() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let cancel = CancellationToken::new();
        let config = PeerSyncConfig {
            self_id: "self".to_string(),
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        let discovery = MockDiscovery::new(vec![Err(PeerDiscoveryError::Backend(
            "stream error".into(),
        ))]);
        // Should break on error event
        fwd.run_discovery(&mp, config, discovery, cancel).await;
    }

    // ── handle_peer_joined ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_peer_joined_skip_self() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let config = PeerSyncConfig {
            self_id: "self-node".to_string(),
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        // Peer with same id as self → skip
        let peer = make_peer_info("self-node");
        fwd.handle_peer_joined(&mp, &config, peer).await;
        assert!(fwd.peer_conns().is_empty());
    }

    #[tokio::test]
    async fn test_handle_peer_joined_skip_no_dial_fullmesh() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let config = PeerSyncConfig {
            self_id: "z-node".to_string(), // lexicographically larger
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        // In FullMesh, only smaller ID dials → "z-node" > "a-peer", so no dial
        let peer = make_peer_info("a-peer");
        fwd.handle_peer_joined(&mp, &config, peer).await;
        assert!(fwd.peer_conns().is_empty());
    }

    #[tokio::test]
    async fn test_handle_peer_joined_skip_no_dial_spoke() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let config = PeerSyncConfig {
            self_id: "spoke-node".to_string(),
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::HubAndSpoke,
        };

        let peer = make_peer_info("other");
        fwd.handle_peer_joined(&mp, &config, peer).await;
        assert!(fwd.peer_conns().is_empty());
    }

    #[tokio::test]
    async fn test_handle_peer_joined_already_connected() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();
        let config = PeerSyncConfig {
            self_id: "a-node".to_string(), // smaller → should_dial
            deployment_name: "deploy".to_string(),
            topology: PeerTopology::FullMesh,
        };

        // Pre-register the peer as already connected
        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "b-peer".to_string(),
            PeerEntry {
                conn_id: 100,
                endpoint: "http://b".to_string(),
                is_outgoing: true,
            },
        );

        let peer = make_peer_info("b-peer");
        fwd.handle_peer_joined(&mp, &config, peer).await;
        // Should skip — no new peer_conn added
        assert!(fwd.peer_conns().is_empty());
    }

    // ── handle_peer_left ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_peer_left_known_peer() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();

        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "peer-x".to_string(),
            PeerEntry {
                conn_id: 55,
                endpoint: "http://x".to_string(),
                is_outgoing: true,
            },
        );
        fwd.add_peer_conn(55);

        let peer = make_peer_info("peer-x");
        fwd.handle_peer_left(&mp, peer).await;
        assert!(!fwd.peer_conns().contains(&55));
        assert!(!state.read().contains("peer-x"));
    }

    #[tokio::test]
    async fn test_handle_peer_left_unknown_peer() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();

        let peer = make_peer_info("unknown-peer");
        // Should not panic
        fwd.handle_peer_left(&mp, peer).await;
    }

    #[tokio::test]
    async fn test_handle_peer_left_incoming_peer_no_disconnect() {
        let fwd = make_forwarder_with_state();
        let mp = MessageProcessor::new();

        let state = fwd.peer_state().unwrap();
        state.write().insert(
            "peer-y".to_string(),
            PeerEntry {
                conn_id: 66,
                endpoint: "http://y".to_string(),
                is_outgoing: false, // incoming → no disconnect call
            },
        );
        fwd.add_peer_conn(66);

        let peer = make_peer_info("peer-y");
        fwd.handle_peer_left(&mp, peer).await;
        assert!(!fwd.peer_conns().contains(&66));
    }

    // ── wait_for_ack_with_retry ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_wait_for_ack_immediate_ok() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 100, 2).unwrap();

        let (tx, rx) = oneshot::channel();
        tx.send(Ok(())).unwrap();

        let result = fwd.wait_for_ack_with_retry(&mp, 100, msg, 999, rx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_for_ack_immediate_err() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 101, 2).unwrap();

        let (tx, rx) = oneshot::channel();
        tx.send(Err(DataPathError::RemoteSubscriptionAckError(
            "nope".into(),
        )))
        .unwrap();

        let result = fwd.wait_for_ack_with_retry(&mp, 101, msg, 999, rx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_wait_for_ack_sender_dropped() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        let name = make_name();
        let msg = super::super::build_subscribe_msg(&name, 102, 2).unwrap();

        let (_tx, rx) = oneshot::channel::<Result<(), DataPathError>>();
        // Drop tx immediately
        drop(_tx);

        let result = fwd.wait_for_ack_with_retry(&mp, 102, msg, 999, rx).await;
        assert!(result.is_err());
    }

    // ── set_peer_conns ──────────────────────────────────────────────────────

    #[test]
    fn test_set_peer_conns() {
        let fwd = make_forwarder();
        fwd.set_peer_conns(HashSet::from([1, 2, 3]));
        assert_eq!(fwd.peer_conns(), HashSet::from([1, 2, 3]));
    }

    // ── peer_label ──────────────────────────────────────────────────────────

    #[test]
    fn test_peer_label_unknown_conn() {
        let fwd = make_forwarder();
        let mp = MessageProcessor::new();
        // Unknown conn → falls back to conn_id as string
        assert_eq!(fwd.peer_label(&mp, 12345), "12345");
    }

    // ── PendingAck Debug ────────────────────────────────────────────────────

    #[test]
    fn test_pending_ack_debug() {
        let (tx, _rx) = oneshot::channel();
        let ack = PendingAck {
            remaining: 2,
            tx: Some(tx),
            errors: vec![DataPathError::RemoteSubscriptionAckError("x".into())],
        };
        let dbg = format!("{:?}", ack);
        assert!(dbg.contains("remaining: 2"));
        assert!(dbg.contains("tx: true"));
        assert!(dbg.contains("errors: 1"));
    }
}

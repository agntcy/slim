// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::RwLock;
use slim_config::client::{ClientConfig, ConnType};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::message_processing::MessageProcessor;
use crate::peer_discovery::{PeerDiscovery, PeerEvent, PeerInfo};

use super::SubscriptionEvent;
use super::state::{PeerEntry, PeerState};
use super::sync;

/// Configuration for the PeerSyncManager.
#[derive(Debug, Clone)]
pub struct PeerSyncConfig {
    /// This replica's unique identifier.
    pub self_id: String,
    /// Shared group identifier for peer authentication.
    pub peer_group: String,
}

/// Manages peer-to-peer subscription synchronization between replicas.
///
/// # Responsibilities
/// - Connects to discovered peers (each node connects to all others)
/// - Performs full subscription sync on peer join
/// - Forwards incremental aggregate subscription transitions to all peers
/// - Cleans up peer state on disconnection
///
/// # Connection Model
/// Each node initiates an outbound connection to every peer. The server side
/// of each node accepts incoming connections as `ConnType::Peer`. Both
/// directions form independent streams; subscriptions flow via the outbound
/// connections managed by this manager.
pub struct PeerSyncManager<D: PeerDiscovery + Send> {
    config: PeerSyncConfig,
    message_processor: MessageProcessor,
    discovery: D,
    state: Arc<RwLock<PeerState>>,
    /// Receiver for subscription events (aggregate transitions).
    subscription_rx: broadcast::Receiver<SubscriptionEvent>,
    /// Receiver for incoming peer connections detected during negotiation.
    incoming_peer_rx:
        tokio::sync::mpsc::UnboundedReceiver<crate::message_processing::IncomingPeerEvent>,
}

impl<D: PeerDiscovery + Send> PeerSyncManager<D> {
    /// Create a new PeerSyncManager.
    ///
    /// `subscription_rx` should be obtained from `MessageProcessor::subscribe_events()`.
    /// `incoming_peer_rx` should be obtained from `MessageProcessor::set_incoming_peer_channel()`.
    pub fn new(
        config: PeerSyncConfig,
        message_processor: MessageProcessor,
        discovery: D,
        subscription_rx: broadcast::Receiver<SubscriptionEvent>,
        incoming_peer_rx: tokio::sync::mpsc::UnboundedReceiver<
            crate::message_processing::IncomingPeerEvent,
        >,
    ) -> Self {
        Self {
            config,
            message_processor,
            discovery,
            state: Arc::new(RwLock::new(PeerState::new())),
            subscription_rx,
            incoming_peer_rx,
        }
    }

    /// Run the peer sync manager until cancellation.
    ///
    /// This is the main event loop that:
    /// 1. Starts peer discovery
    /// 2. Handles discovery events (join/leave)
    /// 3. Forwards incremental subscription changes to peers
    pub async fn run(&mut self, cancel: CancellationToken) {
        info!(
            self_id = %self.config.self_id,
            peer_group = %self.config.peer_group,
            "peer sync manager starting"
        );

        if let Err(e) = self.discovery.start().await {
            error!(error = %e, "failed to start peer discovery");
            return;
        }

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("peer sync manager shutting down");
                    break;
                }
                event = self.discovery.recv() => {
                    match event {
                        Ok(PeerEvent::Joined(peer)) => {
                            self.handle_peer_joined(peer).await;
                        }
                        Ok(PeerEvent::Left(peer)) => {
                            self.handle_peer_left(peer).await;
                        }
                        Err(e) => {
                            error!(error = %e, "peer discovery error, shutting down");
                            break;
                        }
                    }
                }
                event = self.subscription_rx.recv() => {
                    match event {
                        Ok(sub_event) => {
                            self.handle_subscription_event(sub_event).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(
                                missed = n,
                                "subscription event channel lagged, triggering full resync"
                            );
                            self.full_resync_all_peers().await;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("subscription event channel closed, shutting down");
                            break;
                        }
                    }
                }
                Some(event) = self.incoming_peer_rx.recv() => {
                    self.handle_incoming_peer(event).await;
                }
            }
        }
    }

    /// Handle a newly discovered peer.
    ///
    /// Only the node with the lexicographically smaller self_id initiates the
    /// outbound connection. The other side waits for the incoming connection
    /// which is detected during server-side link negotiation and registered
    /// via `handle_incoming_peer`.
    async fn handle_peer_joined(&self, peer: PeerInfo) {
        // Skip self
        if peer.id == self.config.self_id {
            debug!(peer_id = %peer.id, "skipping self in peer discovery");
            return;
        }

        // Tie-breaking: only the smaller ID dials out.
        // The larger ID waits for the incoming connection detected via negotiation.
        if self.config.self_id > peer.id {
            debug!(
                peer_id = %peer.id,
                self_id = %self.config.self_id,
                "skipping outbound connection (larger ID waits for incoming)"
            );
            return;
        }

        // Skip if already connected
        if self.state.read().contains(&peer.id) {
            debug!(peer_id = %peer.id, "peer already connected, skipping");
            return;
        }

        info!(peer_id = %peer.id, endpoint = %peer.endpoint, "connecting to peer");

        let config = ClientConfig {
            endpoint: peer.endpoint.clone(),
            connection_type: ConnType::Peer,
            ..Default::default()
        };

        match self.message_processor.connect(config, None, None).await {
            Ok((_handle, conn_id)) => {
                info!(
                    peer_id = %peer.id,
                    %conn_id,
                    "connected to peer"
                );

                self.state.write().insert(
                    peer.id.clone(),
                    PeerEntry {
                        conn_id,
                        endpoint: peer.endpoint,
                        is_outgoing: true,
                    },
                );

                // Perform full sync: send all local subscriptions to the new peer
                if let Err(e) = sync::send_full_sync(&self.message_processor, conn_id).await {
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
                    endpoint = %peer.endpoint,
                    error = %e,
                    "failed to connect to peer"
                );
            }
        }
    }

    /// Handle a peer leaving the deployment.
    async fn handle_peer_left(&self, peer: PeerInfo) {
        if let Some(entry) = self.state.write().remove(&peer.id) {
            info!(
                peer_id = %peer.id,
                conn_id = entry.conn_id,
                "peer left, disconnecting"
            );

            // Disconnect triggers on_connection_drop which removes all
            // subscriptions associated with this peer connection.
            if entry.is_outgoing
                && let Err(e) = self.message_processor.disconnect(entry.conn_id)
            {
                warn!(
                    peer_id = %peer.id,
                    error = %e,
                    "error disconnecting from peer"
                );
            }
        }
    }

    /// Handle an incoming peer connection detected during server-side negotiation.
    ///
    /// When a remote peer dials us, the server-side link negotiation detects the
    /// peer connection_type and upgrades it. This handler registers that connection
    /// in peer state and performs a full sync.
    async fn handle_incoming_peer(&self, event: crate::message_processing::IncomingPeerEvent) {
        let peer_id = event.link_id.clone();
        let conn_id = event.conn_id;

        if self.state.read().contains(&peer_id) {
            debug!(
                %peer_id,
                %conn_id,
                "incoming peer already registered, skipping"
            );
            return;
        }

        info!(
            %peer_id,
            %conn_id,
            "registering incoming peer connection"
        );

        self.state.write().insert(
            peer_id.clone(),
            PeerEntry {
                conn_id,
                endpoint: String::new(), // server doesn't know the peer's listen endpoint
                is_outgoing: false,
            },
        );

        // Perform full sync: send all local subscriptions to the incoming peer
        if let Err(e) = sync::send_full_sync(&self.message_processor, conn_id).await {
            warn!(
                %peer_id,
                error = %e,
                "full sync failed for incoming peer"
            );
        }
    }

    /// Handle an aggregate subscription transition event.
    async fn handle_subscription_event(&self, event: SubscriptionEvent) {
        let peer_conns = self.state.read().all_conn_ids();
        if peer_conns.is_empty() {
            return;
        }

        match event {
            SubscriptionEvent::Added {
                name,
                subscription_id,
            } => {
                debug!(%name, "forwarding subscription to peers");
                sync::broadcast_subscribe(
                    &self.message_processor,
                    &name,
                    subscription_id,
                    &peer_conns,
                )
                .await;
            }
            SubscriptionEvent::Removed {
                name,
                subscription_id,
            } => {
                debug!(%name, "forwarding unsubscription to peers");
                sync::broadcast_unsubscribe(
                    &self.message_processor,
                    &name,
                    subscription_id,
                    &peer_conns,
                )
                .await;
            }
        }
    }

    /// Trigger a full resync to all connected peers (e.g., after event lag).
    async fn full_resync_all_peers(&self) {
        let peer_conns = self.state.read().all_conn_ids();
        for conn_id in peer_conns {
            if let Err(e) = sync::send_full_sync(&self.message_processor, conn_id).await {
                warn!(
                    %conn_id,
                    error = %e,
                    "full resync failed for peer"
                );
            }
        }
    }

    /// Register an incoming peer connection (server-side).
    ///
    /// Called when a peer connects to us and is identified during link negotiation.
    /// The connection is already established; this just registers it in peer state.
    pub fn register_incoming_peer(&self, peer_id: String, conn_id: u64, endpoint: String) {
        if self.state.read().contains(&peer_id) {
            debug!(
                %peer_id,
                %conn_id,
                "incoming peer already registered, skipping"
            );
            return;
        }

        info!(
            %peer_id,
            %conn_id,
            "registering incoming peer connection"
        );

        self.state.write().insert(
            peer_id,
            PeerEntry {
                conn_id,
                endpoint,
                is_outgoing: false,
            },
        );
    }

    /// Notify the manager that a connection was dropped.
    /// If it was a peer connection, clean up state.
    pub fn on_connection_drop(&self, conn_id: u64) {
        if let Some((peer_id, _entry)) = self.state.write().remove_by_conn(conn_id) {
            info!(
                %peer_id,
                %conn_id,
                "peer connection dropped, cleaned up state"
            );
        }
    }

    /// Get the current peer state (for testing/inspection).
    pub fn peer_state(&self) -> Arc<RwLock<PeerState>> {
        self.state.clone()
    }
}

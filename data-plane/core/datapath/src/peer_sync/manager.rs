// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use parking_lot::RwLock;
use slim_config::client::{ClientConfig, ConnType};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::message_processing::MessageProcessor;
use crate::peer_discovery::config::PeerTopology;
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
    /// Topology for peer connections.
    pub topology: PeerTopology,
    /// Whether this node is the hub (smallest ID). Only meaningful for HubAndSpoke.
    pub is_hub: bool,
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
    /// Receiver for peer relay events (subscription received on peer connections).
    peer_relay_rx:
        tokio::sync::mpsc::UnboundedReceiver<crate::peer_sync::PeerRelayEvent>,
}

impl<D: PeerDiscovery + Send> PeerSyncManager<D> {
    /// Create a new PeerSyncManager.
    ///
    /// `subscription_rx` should be obtained from `MessageProcessor::subscribe_events()`.
    /// `incoming_peer_rx` should be obtained from `MessageProcessor::set_incoming_peer_channel()`.
    /// `peer_relay_rx` should be obtained from `MessageProcessor::set_peer_relay_channel()`.
    pub fn new(
        config: PeerSyncConfig,
        message_processor: MessageProcessor,
        discovery: D,
        subscription_rx: broadcast::Receiver<SubscriptionEvent>,
        incoming_peer_rx: tokio::sync::mpsc::UnboundedReceiver<
            crate::message_processing::IncomingPeerEvent,
        >,
        peer_relay_rx: tokio::sync::mpsc::UnboundedReceiver<
            crate::peer_sync::PeerRelayEvent,
        >,
    ) -> Self {
        // If hub in hub-and-spoke, configure the message processor for publish relay.
        if config.is_hub && config.topology == PeerTopology::HubAndSpoke {
            message_processor.set_peer_hub(true);
        }

        Self {
            config,
            message_processor,
            discovery,
            state: Arc::new(RwLock::new(PeerState::new())),
            subscription_rx,
            incoming_peer_rx,
            peer_relay_rx,
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
                Some(relay) = self.peer_relay_rx.recv() => {
                    self.handle_peer_relay(relay).await;
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
    async fn handle_peer_joined(&self, peer: PeerInfo) {
        // Skip self
        if peer.id == self.config.self_id {
            debug!(peer_id = %peer.id, "skipping self in peer discovery");
            return;
        }

        // Determine whether to dial based on topology.
        let should_dial = match self.config.topology {
            PeerTopology::FullMesh => {
                // Tie-breaking: only the smaller ID dials out.
                self.config.self_id < peer.id
            }
            PeerTopology::HubAndSpoke => {
                // Only the hub (smallest ID) dials. Spokes wait for incoming.
                self.config.is_hub
            }
        };

        if !should_dial {
            debug!(
                peer_id = %peer.id,
                self_id = %self.config.self_id,
                topology = ?self.config.topology,
                "skipping outbound connection (waiting for incoming)"
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

                // Perform full sync: send subscriptions to the new peer.
                // Hub sends local + other peers' subscriptions; non-hub sends only local.
                let sync_result = if self.config.is_hub
                    && self.config.topology == PeerTopology::HubAndSpoke
                {
                    sync::send_full_sync_as_hub(&self.message_processor, conn_id).await
                } else {
                    sync::send_full_sync(&self.message_processor, conn_id).await
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

        // Perform full sync: send subscriptions to the incoming peer.
        let sync_result = if self.config.is_hub
            && self.config.topology == PeerTopology::HubAndSpoke
        {
            sync::send_full_sync_as_hub(&self.message_processor, conn_id).await
        } else {
            sync::send_full_sync(&self.message_processor, conn_id).await
        };
        if let Err(e) = sync_result {
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

    /// Handle a peer relay event (hub-and-spoke only).
    ///
    /// When the hub receives a subscription from one spoke, it relays it to all
    /// other spokes so they know to route messages through the hub.
    async fn handle_peer_relay(&self, relay: crate::peer_sync::PeerRelayEvent) {
        // Only the hub in hub-and-spoke relays peer subscriptions.
        if self.config.topology != PeerTopology::HubAndSpoke || !self.config.is_hub {
            return;
        }

        // Forward to all peers except the source.
        let peer_conns: Vec<u64> = self
            .state
            .read()
            .all_conn_ids()
            .into_iter()
            .filter(|&c| c != relay.source_conn)
            .collect();

        if peer_conns.is_empty() {
            return;
        }

        if relay.is_subscribe {
            debug!(
                name = %relay.name,
                source_conn = relay.source_conn,
                targets = peer_conns.len(),
                "hub relaying subscription to other spokes"
            );
            sync::broadcast_subscribe(
                &self.message_processor,
                &relay.name,
                relay.subscription_id,
                &peer_conns,
            )
            .await;
        } else {
            debug!(
                name = %relay.name,
                source_conn = relay.source_conn,
                targets = peer_conns.len(),
                "hub relaying unsubscription to other spokes"
            );
            sync::broadcast_unsubscribe(
                &self.message_processor,
                &relay.name,
                relay.subscription_id,
                &peer_conns,
            )
            .await;
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

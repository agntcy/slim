// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use display_error_chain::ErrorChainExt;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::message_processing::MessageProcessor;
use crate::peer_discovery::config::PeerTopology;
use crate::peer_discovery::{PeerDiscovery, PeerEvent, PeerInfo};

use super::SubscriptionForwarder;
use super::peer;
use super::state::{PeerEntry, PeerState};

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
/// - Updates the SubscriptionForwarder's peer connection list on join/leave
/// - Cleans up peer state on disconnection
///
/// # Connection Model
/// Each node initiates an outbound connection to every peer. The server side
/// of each node accepts incoming connections as `ConnType::Peer`. Both
/// directions form independent streams; subscriptions flow via the outbound
/// connections managed by this manager.
///
/// # Subscription Forwarding
/// Incremental subscription forwarding (peer broadcast, hub relay) is handled
/// by the `SubscriptionForwarder` component — this manager only maintains the
/// peer connection list and performs full sync on join.
pub struct PeerSyncManager<D: PeerDiscovery + Send> {
    config: PeerSyncConfig,
    message_processor: MessageProcessor,
    discovery: D,
    state: Arc<RwLock<PeerState>>,
    /// The subscription forwarder (shared with MessageProcessor).
    forwarder: SubscriptionForwarder,
    /// Receiver for incoming peer connections detected during negotiation.
    incoming_peer_rx: mpsc::Receiver<crate::message_processing::IncomingPeerEvent>,
}

impl<D: PeerDiscovery + Send> PeerSyncManager<D> {
    /// Create a new PeerSyncManager.
    ///
    /// Creates a `SubscriptionForwarder` and installs it on the `MessageProcessor`
    /// so that `process_subscription` can use it for peer forwarding.
    ///
    /// `incoming_peer_rx` should be obtained from `MessageProcessor::take_incoming_peer_rx()`.
    pub fn new(
        config: PeerSyncConfig,
        message_processor: MessageProcessor,
        discovery: D,
        incoming_peer_rx: mpsc::Receiver<crate::message_processing::IncomingPeerEvent>,
    ) -> Self {
        // Create and install the subscription forwarder.
        let forwarder = SubscriptionForwarder::new(message_processor.clone(), &config.topology);
        message_processor.set_subscription_forwarder(forwarder.clone());

        Self {
            config,
            message_processor,
            discovery,
            state: Arc::new(RwLock::new(PeerState::new())),
            forwarder,
            incoming_peer_rx,
        }
    }

    /// Run the peer sync manager until cancellation.
    ///
    /// This is the main event loop that:
    /// 1. Starts peer discovery
    /// 2. Handles discovery events (join/leave)
    /// 3. Registers incoming peer connections
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
                Some(event) = self.incoming_peer_rx.recv() => {
                    self.handle_incoming_peer(event).await;
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

        info!(peer_id = %peer.id, endpoint = %peer.config.endpoint, "connecting to peer");

        match self
            .message_processor
            .connect(peer.config.clone(), None, None)
            .await
        {
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
                        endpoint: peer.config.endpoint.clone(),
                        is_outgoing: true,
                    },
                );

                // Update the forwarder's peer connection list.
                self.forwarder.add_peer_conn(conn_id);

                // Perform full sync: send subscriptions to the new peer.
                // Hub sends all subscriptions; non-hub sends only local + remote.
                let ttl = self.forwarder.subscription_ttl();
                let sync_result =
                    if self.config.is_hub && self.config.topology == PeerTopology::HubAndSpoke {
                        peer::send_full_sync(&self.message_processor, conn_id, ttl).await
                    } else {
                        peer::send_local_remote_sync(&self.message_processor, conn_id, ttl).await
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
    async fn handle_peer_left(&self, peer: PeerInfo) {
        if let Some(entry) = self.state.write().remove(&peer.id) {
            info!(
                peer_id = %peer.id,
                conn_id = entry.conn_id,
                "peer left, disconnecting"
            );

            // Remove from forwarder's peer connection list.
            self.forwarder.remove_peer_conn(entry.conn_id);

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
        let peer_id = event.node_id;
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
            "registering incoming peer in state table"
        );

        // State tracking only — registration + sync already handled by the forwarder.
        self.state.write().insert(
            peer_id,
            PeerEntry {
                conn_id,
                endpoint: String::new(), // server doesn't know the peer's listen endpoint
                is_outgoing: false,
            },
        );
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

        // Update the forwarder's peer connection list.
        self.forwarder.add_peer_conn(conn_id);
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

            // Remove from forwarder's peer connection list.
            self.forwarder.remove_peer_conn(conn_id);
        }
    }

    /// Get the current peer state (for testing/inspection).
    pub fn peer_state(&self) -> Arc<RwLock<PeerState>> {
        self.state.clone()
    }

    /// Get a reference to the subscription forwarder.
    pub fn subscription_forwarder(&self) -> &SubscriptionForwarder {
        &self.forwarder
    }
}

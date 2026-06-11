// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use crate::api::DataPlaneServiceServer;
use display_error_chain::ErrorChainExt;
use parking_lot::RwLock;
use slim_config::client::ClientConfig;
use slim_config::client::TransportChannel;
use slim_config::component::configuration::Configuration;
use slim_config::server::ServerConfig;
use slim_config::server_handler::ServerHandler;
use slim_config::websocket::server as websocket_server;
use slim_config::websocket::server::AcceptedWebSocketConnection;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use tonic::{Request, Response, Status};
use tracing::{Instrument, debug, error, info, warn};

#[cfg(feature = "otel_tracing")]
use crate::otel_tracing;

use crate::api::ProtoMessage;
use crate::api::ProtoPublishType as PublishType;
use crate::api::ProtoSubscribeType as SubscribeType;
use crate::api::ProtoSubscriptionAckType as SubscriptionAckType;
use crate::api::ProtoUnsubscribeType as UnsubscribeType;
use crate::api::proto::dataplane::v1::Message;

use crate::api::proto::dataplane::v1::LinkConnectionType;
use crate::api::proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
use crate::api::proto::dataplane::v1::data_plane_service_server::DataPlaneService;
use crate::api::{
    LinkNegotiationPayload, ProtoLink, ProtoLinkMessageType as LinkType, ProtoLinkType, ProtoName,
};
use crate::connection::{Channel, Connection};
use crate::errors::{DataPathError, MessageContext};
use crate::forwarder::Forwarder;
use crate::link_ecdh::{self, X25519_PUBLIC_KEY_LEN};
use crate::messages::utils::SlimHeaderFlags;
use crate::sync::peer as sync_peer;
use crate::sync::remote::{RemoteSync, SubscriptionInfo};
use crate::tables::connection_table::ConnectionTable;
use crate::tables::subscription_table::SubscriptionTableImpl;
use crate::tables::{ConnType, MatchFilter};
use crate::websocket;
use semver;

fn local_version() -> &'static str {
    slim_version::version()
}

/// Result of updating subscription state (pure state change, no forwarding).
struct SubscriptionOutcome {
    /// Whether an aggregate transition occurred (0→1 or 1→0).
    transition: bool,
    /// Whether the source connection is a peer connection.
    is_peer_conn: bool,
    /// The forward-to connection (controller), if any.
    forward_conn: Option<u64>,
}

// Sync tests using environment variables
#[cfg(test)]
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug)]
struct MessageProcessorInternal {
    /// The forwarder to handle processing events
    forwarder: Forwarder<Connection>,

    /// Drain signal to gracefully close all pending tasks
    drain_signal: parking_lot::RwLock<Option<drain::Signal>>,

    ///Drain watch to receive drain signal
    drain_watch: parking_lot::RwLock<Option<drain::Watch>>,

    /// Tx channel towards control plane
    tx_control_plane: RwLock<Option<Sender<Result<Message, Status>>>>,

    /// Tracks subscriptions forwarded to remote connections and handles restore on reconnect.
    /// Also owns the recovery table for server-side route preservation.
    remote_sync: RemoteSync,

    /// Service ID for tracing
    service_id: String,

    /// Peer group this node belongs to. Used during link negotiation to verify
    /// that both sides of a peer connection belong to the same deployment.
    /// Empty when no peer config is set.
    deployment_name: String,

    /// Default strict header MAC policy for server-accepted inter-node connections (see [`ServerConfig::require_header_mac`]).
    server_require_header_mac: bool,

    /// Timeout to wait for link HMAC session to be installed.
    link_hmac_timeout: std::time::Duration,

    /// Polling interval (in milliseconds) to wait between HMAC existence checks.
    link_hmac_poll_interval: std::time::Duration,

    /// Whether peer-originated publishes should be relayed to other peers.
    /// True for hub-and-spoke (hub) or generic multi-hop topologies.
    /// False for full-mesh (peers deliver directly — 1-hop rule).
    relay_peer_publishes: bool,

    /// Peer sync component for subscription forwarding and peer lifecycle.
    /// Initialized as standalone; replaced with a peer-aware instance when peers are configured.
    peer_sync: parking_lot::RwLock<crate::sync::PeerSync>,
}

#[derive(Debug, Clone)]
pub struct MessageProcessor {
    internal: Arc<MessageProcessorInternal>,
}

impl Default for MessageProcessor {
    fn default() -> Self {
        Self::new_with_service_id(String::new())
    }
}

impl MessageProcessor {
    pub fn new_with_service_id(service_id: String) -> Self {
        Self::new_with_options(service_id, None)
    }

    pub fn new_with_options(service_id: String, recovery_ttl: Option<std::time::Duration>) -> Self {
        Self::new_internal(
            service_id,
            String::new(),
            recovery_ttl,
            false,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(5),
            false,
        )
    }

    /// Create a processor with the server strict header MAC policy from `server_config`.
    pub fn new_with_server_config(
        service_id: String,
        deployment_name: String,
        server_config: &ServerConfig,
        recovery_ttl: Option<std::time::Duration>,
        relay_peer_publishes: bool,
    ) -> Self {
        Self::new_internal(
            service_id,
            deployment_name,
            recovery_ttl,
            server_config.require_header_mac,
            std::time::Duration::from_secs(server_config.link_hmac_timeout_secs),
            std::time::Duration::from_millis(server_config.link_hmac_poll_interval_ms),
            relay_peer_publishes,
        )
    }

    fn new_internal(
        service_id: String,
        deployment_name: String,
        recovery_ttl: Option<std::time::Duration>,
        server_require_header_mac: bool,
        link_hmac_timeout: std::time::Duration,
        link_hmac_poll_interval: std::time::Duration,
        relay_peer_publishes: bool,
    ) -> Self {
        let (signal, watch) = drain::channel();
        let internal = MessageProcessorInternal {
            forwarder: Forwarder::new(),
            drain_signal: RwLock::new(Some(signal)),
            drain_watch: RwLock::new(Some(watch)),
            tx_control_plane: RwLock::new(None),
            remote_sync: RemoteSync::new(recovery_ttl),
            service_id,
            deployment_name,
            server_require_header_mac,
            link_hmac_timeout,
            link_hmac_poll_interval,
            relay_peer_publishes,
            peer_sync: parking_lot::RwLock::new(crate::sync::PeerSync::standalone()),
        };
        Self {
            internal: Arc::new(internal),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Run a data plane server using this message processor's drain watch.
    /// Dispatch on the configured transport happens inside slim-config via the
    /// [`ServerHandler`] trait below. Returns a cancellation token that can be
    /// used to stop the server task.
    pub async fn run_server(
        &self,
        config: &ServerConfig,
    ) -> Result<CancellationToken, DataPathError> {
        debug!(%config, "starting dataplane server");

        if config.require_header_mac != self.internal.server_require_header_mac {
            warn!(
                configured = config.require_header_mac,
                processor = self.internal.server_require_header_mac,
                "server require_header_mac differs from MessageProcessor; inbound connections use the processor value set at construction (prefer MessageProcessor::new_with_server_config)",
            );
        }

        let watch = self.get_drain_watch()?;
        config
            .run_server(watch, Arc::new(self.clone()))
            .await
            .map_err(Into::into)
    }

    async fn handle_websocket_accepted(&self, accepted: AcceptedWebSocketConnection) {
        let cancellation_token = CancellationToken::new();
        let streams =
            websocket::spawn_transport_tasks(accepted.websocket, cancellation_token.clone());

        let connection = Connection::new(ConnType::Remote, Channel::Client(streams.outbound))
            .with_remote_addr(accepted.remote_addr)
            .with_local_addr(accepted.local_addr)
            .with_require_header_mac(self.internal.server_require_header_mac)
            .with_cancellation_token(Some(cancellation_token.clone()));

        debug!(
            remote = ?connection.remote_addr(),
            local = ?connection.local_addr(),
            "new websocket connection received from remote",
        );
        info!(telemetry = true, counter.num_active_connections = 1);

        let conn_index = match self.forwarder().on_connection_established(connection, None) {
            Some(index) => index,
            None => {
                error!("failed to add websocket connection to table");
                cancellation_token.cancel();
                return;
            }
        };

        if let Err(err) = self.process_stream(
            streams.inbound,
            conn_index,
            None,
            cancellation_token,
            ConnType::Remote,
            false,
        ) {
            error!(error = %err.chain(), "error starting websocket processing stream");
        }
    }

    /// Signal all spawned tasks (process_stream, etc.) to begin shutting down.
    ///
    /// Unlike [`shutdown`], this is synchronous: it drops the drain signal (which
    /// notifies all drain watches) and the drain watch, but does NOT wait for the
    /// tasks to finish.  Safe to call from a synchronous `Drop` implementation.
    pub fn signal_drain(&self) {
        self.internal.drain_signal.write().take();
        self.internal.drain_watch.write().take();
    }

    pub async fn shutdown(&self) -> Result<(), DataPathError> {
        // Take the drain signal
        let signal = self
            .internal
            .drain_signal
            .write()
            .take()
            .ok_or(DataPathError::AlreadyClosedError)?;

        // Take drain watch
        self.internal.drain_watch.write().take();

        // Signal completion to all tasks
        signal.drain().await;

        Ok(())
    }

    fn set_tx_control_plane(&self, tx: Sender<Result<Message, Status>>) {
        let mut tx_guard = self.internal.tx_control_plane.write();
        *tx_guard = Some(tx);
    }

    fn get_tx_control_plane(&self) -> Option<Sender<Result<Message, Status>>> {
        let tx_guard = self.internal.tx_control_plane.read();
        tx_guard.clone()
    }

    pub(crate) fn forwarder(&self) -> &Forwarder<Connection> {
        &self.internal.forwarder
    }

    pub(crate) fn remote_sync(&self) -> &RemoteSync {
        &self.internal.remote_sync
    }

    /// Verify SLIM header MAC for inter-node traffic only (local app connections skip this).
    pub(crate) fn verify_remote_header_mac(
        &self,
        conn_index: u64,
        message: &Message,
        enforce_strict_verification: bool,
    ) -> Result<(), DataPathError> {
        let conn = self
            .forwarder()
            .get_connection(conn_index)
            .ok_or(DataPathError::ConnectionNotFound(conn_index))?;
        if !matches!(conn.connection_type(), ConnType::Remote) {
            return Ok(());
        }
        let header = message
            .try_get_slim_header()
            .ok_or(DataPathError::UnknownMsgType)?;

        let has_wire_mac = header.header_mac.as_ref().is_some_and(|m| !m.is_empty());

        // Publishes must carry a MAC once the inter-node session has derived a key.  Control
        // messages (subscribe / unsubscribe) may still traverse the same gRPC stream without a
        // tag on some federation paths; skipping verification only when the tag is absent keeps
        // tamper detection for application traffic.
        if (message.is_subscribe() || message.is_unsubscribe()) && !has_wire_mac {
            if enforce_strict_verification {
                return Err(DataPathError::NegotiationError(
                    "empty HMAC is not allowed in strict verification mode".to_string(),
                ));
            }
            return Ok(());
        }

        let Some(mac) = conn.header_hmac() else {
            if enforce_strict_verification {
                return Err(DataPathError::NegotiationError(
                    "strict header MAC required but link HMAC session is not installed".to_string(),
                ));
            }
            // Do not accept inter-node publishes that already carry an integrity tag until this
            // side has derived the link MAC; otherwise verification is silently skipped and peers
            // never see `HeaderIntegrity` failures (including tampered test traffic).
            if message.is_publish() && has_wire_mac {
                return Err(DataPathError::HeaderMacAwaitingLinkNegotiation(conn_index));
            }
            return Ok(());
        };
        let link_id = conn
            .link_id()
            .filter(|id| !id.is_empty())
            .ok_or(DataPathError::HeaderMacAwaitingLinkNegotiation(conn_index))?;
        mac.verify_slim_header(header, &link_id)
            .map_err(DataPathError::HeaderIntegrity)
    }

    pub(crate) fn get_drain_watch(&self) -> Result<drain::Watch, DataPathError> {
        self.internal
            .drain_watch
            .read()
            .clone()
            .ok_or(DataPathError::AlreadyClosedError)
    }

    /// Re-send `remote_subs` as subscribe messages to `conn_index`.
    /// Delegates to [`RemoteSync::restore`].
    async fn restore_remote_subscriptions(
        &self,
        remote_subs: &HashSet<SubscriptionInfo>,
        conn_index: u64,
        restore_tracking: bool,
    ) {
        self.remote_sync()
            .restore(self, remote_subs, conn_index, restore_tracking)
            .await;
    }

    async fn try_to_connect(
        &self,
        client_config: ClientConfig,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
        existing_conn_index: Option<u64>,
    ) -> Result<(JoinHandle<()>, u64), DataPathError> {
        client_config.validate()?;

        let mut watch = std::pin::pin!(self.get_drain_watch()?.signaled());
        let channel = tokio::select! {
            _ = &mut watch => {
                return Err(DataPathError::ShuttingDownError);
            }
            res = client_config.to_channel() => {
                res?
            }
        };

        let cancellation_token = CancellationToken::new();
        let link_id = client_config.link_id.clone();

        match channel {
            TransportChannel::Grpc(grpc_channel) => {
                let mut client = DataPlaneServiceClient::new(grpc_channel);
                let (tx, rx) = mpsc::channel(128);
                let stream = client
                    .open_channel(Request::new(ReceiverStream::new(rx)))
                    .await?;

                let (ecdh_sk, ecdh_pk) = link_ecdh::generate_x25519_ephemeral()
                    .map_err(|_| DataPathError::LinkKeyGeneration)?;

                let (handle, conn_index) = self.register_remote_connection(
                    stream.into_inner(),
                    Channel::Client(tx),
                    &client_config,
                    local,
                    remote,
                    existing_conn_index,
                    cancellation_token,
                    Some(link_id.clone()),
                    Some(ecdh_sk),
                )?;

                self.send_client_link_negotiation(
                    &link_id,
                    conn_index,
                    Some(ecdh_pk),
                    client_config.connection_type,
                )
                .await;
                self.await_link_hmac_ready(conn_index, client_config.require_header_mac)
                    .await?;

                Ok((handle, conn_index))
            }
            TransportChannel::Websocket(ws_channel) => {
                let websocket = ws_channel
                    .take_websocket()
                    .expect("websocket channel already consumed");
                let streams =
                    websocket::spawn_transport_tasks(websocket, cancellation_token.clone());

                let (ecdh_sk, ecdh_pk) = link_ecdh::generate_x25519_ephemeral()
                    .map_err(|_| DataPathError::LinkKeyGeneration)?;

                let (handle, conn_index) = self.register_remote_connection(
                    streams.inbound,
                    Channel::Client(streams.outbound),
                    &client_config,
                    local.or(ws_channel.local_addr()),
                    remote.or(ws_channel.remote_addr()),
                    existing_conn_index,
                    cancellation_token,
                    Some(link_id.clone()),
                    Some(ecdh_sk),
                )?;

                self.send_client_link_negotiation(
                    &link_id,
                    conn_index,
                    Some(ecdh_pk),
                    client_config.connection_type,
                )
                .await;
                self.await_link_hmac_ready(conn_index, client_config.require_header_mac)
                    .await?;

                Ok((handle, conn_index))
            }
        }
    }

    /// Send the outbound link negotiation request (best-effort for older peers).
    async fn send_client_link_negotiation(
        &self,
        link_id: &str,
        conn_index: u64,
        ecdh_public_key: Option<Vec<u8>>,
        connection_type: ConnType,
    ) {
        let negotiation_msg = ProtoMessage::builder().build_link_negotiation(
            link_id,
            local_version(),
            false,
            ecdh_public_key,
            connection_type.into(),
            &self.internal.service_id,
            &self.internal.deployment_name,
        );
        if let Err(e) = self.send_msg(negotiation_msg, conn_index).await {
            debug!(
                %conn_index,
                error = %e.chain(),
                "failed to send link negotiation (remote may be an older SLIM instance)",
            );
        }
    }

    /// Block until the link HMAC session is installed when strict header MAC is enabled.
    async fn await_link_hmac_ready(
        &self,
        conn_index: u64,
        require_header_mac: bool,
    ) -> Result<(), DataPathError> {
        if !require_header_mac {
            return Ok(());
        }

        let timeout = self.internal.link_hmac_timeout;
        let start = tokio::time::Instant::now();
        while start.elapsed() < timeout {
            match self.forwarder().get_connection(conn_index) {
                Some(conn) if conn.header_hmac().is_some() => return Ok(()),
                Some(_) => {
                    tokio::time::sleep(self.internal.link_hmac_poll_interval).await;
                }
                None => return Err(DataPathError::ConnectionNotFound(conn_index)),
            }
        }

        Err(DataPathError::NegotiationError(
            "timed out waiting for link HMAC session after negotiation".to_string(),
        ))
    }

    /// Common post-connect plumbing shared by every transport: register the
    /// new [`Connection`] in the forwarder and spawn the per-stream processor.
    /// Transport-specific code only has to produce the inbound stream + outbound
    /// channel and call this — see [`Self::try_to_connect`] for client-side
    /// usage and [`Self::handle_websocket_accepted`] for the server side.
    #[allow(clippy::too_many_arguments)]
    fn register_remote_connection<S>(
        &self,
        inbound: S,
        outbound: Channel,
        client_config: &ClientConfig,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
        existing_conn_index: Option<u64>,
        cancellation_token: CancellationToken,
        link_id: Option<String>,
        outbound_ecdh_private: Option<aws_lc_rs::agreement::EphemeralPrivateKey>,
    ) -> Result<(JoinHandle<()>, u64), DataPathError>
    where
        S: Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
    {
        let mut connection = Connection::new(client_config.connection_type, outbound)
            .with_local_addr(local)
            .with_remote_addr(remote)
            .with_config_data(Some(client_config.clone()))
            .with_require_header_mac(client_config.require_header_mac)
            .with_cancellation_token(Some(cancellation_token.clone()));
        if let Some(link_id) = link_id {
            connection = connection.with_link_id(link_id);
        }

        if let Some(ecdh_sk) = outbound_ecdh_private {
            connection.set_outbound_ecdh_private(ecdh_sk);
        }

        debug!(
            remote = ?connection.remote_addr(),
            local = ?connection.local_addr(),
            ?client_config.connection_type,
            "new connection initiated locally",
        );

        let conn_index = self
            .forwarder()
            .on_connection_established(connection, existing_conn_index)
            .ok_or(DataPathError::ConnectionTableAddError)?;

        debug!(%conn_index, is_local = false, "new connection index");

        let handle = self.process_stream(
            inbound,
            conn_index,
            Some(client_config.clone()),
            cancellation_token,
            client_config.connection_type,
            false,
        )?;

        // For peer connections established via client config (generic topology),
        // auto-register in the forwarder and perform full sync.
        // Only when no PeerSyncManager is active (it handles its own peers).
        if matches!(client_config.connection_type, ConnType::Peer) {
            let fwd = self.peer_sync();
            if !fwd.has_peer_state() {
                fwd.add_peer_conn_and_sync(self, conn_index);
            }
        }

        Ok((handle, conn_index))
    }

    pub async fn connect(
        &self,
        client_config: ClientConfig,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
    ) -> Result<(JoinHandle<()>, u64), DataPathError> {
        self.try_to_connect(client_config, local, remote, None)
            .await
    }

    pub fn disconnect(&self, conn: u64) -> Result<ClientConfig, DataPathError> {
        let connection = match self.forwarder().get_connection(conn) {
            Some(c) => c,
            None => {
                error!(%conn, "error handling disconnect: connection unknown");
                return Err(DataPathError::DisconnectionError(conn));
            }
        };

        let token = match connection.cancellation_token() {
            Some(t) => t,
            None => {
                error!(%conn, "error handling disconnect: missing cancellation token");
                return Err(DataPathError::DisconnectionError(conn));
            }
        };

        // Cancel receiving loop; this triggers deletion of connection state.
        token.cancel();

        connection
            .config_data()
            .cloned()
            .ok_or(DataPathError::DisconnectionError(conn))
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.internal.service_id))]
    pub fn register_local_connection(
        &self,
        from_control_plane: bool,
    ) -> Result<
        (
            u64,
            tokio::sync::mpsc::Sender<Result<Message, Status>>,
            tokio::sync::mpsc::Receiver<Result<Message, Status>>,
        ),
        DataPathError,
    > {
        // create a pair tx, rx to be able to send messages with the standard processing loop
        let (tx1, rx1) = mpsc::channel(512);

        debug!("establishing new local app connection");

        // create a pair tx, rx to be able to receive messages and insert it into the connection table
        let (tx2, rx2) = mpsc::channel(512);

        // if the call is coming from the control plane set the tx channel
        // we assume to talk to a single control plane so set the channel only once
        if from_control_plane && self.get_tx_control_plane().is_none() {
            self.set_tx_control_plane(tx2.clone());
        }

        // create a connection
        let cancellation_token = CancellationToken::new();
        let connection = Connection::new(ConnType::Local, Channel::Server(tx2))
            .with_cancellation_token(Some(cancellation_token.clone()));

        // add it to the connection table
        let conn_id = self
            .forwarder()
            .on_connection_established(connection, None)
            .unwrap();

        debug!(%conn_id, "local connection established");
        info!(telemetry = true, counter.num_active_connections = 1);

        // this loop will process messages from the local app
        self.process_stream(
            ReceiverStream::new(rx1),
            conn_id,
            None,
            cancellation_token,
            ConnType::Local,
            from_control_plane,
        )?;

        // return the conn_id and  handles to be used to send and receive messages
        Ok((conn_id, tx1, rx2))
    }

    pub async fn send_msg(
        &self,
        #[cfg(feature = "otel_tracing")] mut msg: Message,
        #[cfg(not(feature = "otel_tracing"))] msg: Message,
        out_conn: u64,
    ) -> Result<(), DataPathError> {
        #[cfg(feature = "otel_tracing")]
        otel_tracing::prepare_outbound_msg(
            &mut msg,
            "send_message",
            &self.internal.service_id,
            otel_tracing::SpanTarget::Connection(out_conn),
        );
        self.send_msg_raw(msg, out_conn).await
    }

    async fn send_msg_raw(&self, mut msg: Message, out_conn: u64) -> Result<(), DataPathError> {
        let connection = self.forwarder().get_connection(out_conn);
        match connection {
            Some(conn) => {
                // Link and SubscriptionAck messages have no SLIM header: skip header
                // manipulation and telemetry span creation.
                if !msg.is_link() && !msg.is_subscription_ack() {
                    msg.clear_slim_header();
                }

                if !msg.is_link()
                    && !msg.is_subscription_ack()
                    && matches!(conn.connection_type(), ConnType::Remote)
                    && conn.require_header_mac()
                    && conn.header_hmac().is_none()
                {
                    return Err(DataPathError::NegotiationError(
                        "strict header MAC required but link HMAC session is not installed"
                            .to_string(),
                    ));
                }

                if !msg.is_link()
                    && !msg.is_subscription_ack()
                    && matches!(conn.connection_type(), ConnType::Remote)
                    && let Some(mac) = conn.header_hmac()
                {
                    let link_id = conn
                        .link_id()
                        .or_else(|| conn.config_data().map(|c| c.link_id.clone()))
                        .filter(|id| !id.is_empty());
                    if let Some(ref id) = link_id {
                        let header = msg.get_slim_header_mut();

                        mac.sign_slim_header(header, id.as_str())
                            .map_err(DataPathError::HeaderIntegrity)?;

                        // Debug / integration-test builds only (`--release` omits this; env var is inert).
                        // Must run *after* sign so the tag does not cover the mutated preimage fields.
                        #[cfg(debug_assertions)]
                        if std::env::var("SLIM_TEST_TAMPER_DESTINATION").is_ok()
                            && let Some(dest) = header.destination.as_mut()
                            && let Some(sn) = dest.str_name.as_mut()
                        {
                            sn.str_component_2.push_str("-integrity-test-tamper");
                        }
                    } else {
                        return Err(DataPathError::HeaderMacAwaitingLinkNegotiation(out_conn));
                    }
                }

                if !msg.is_link()
                    && !msg.is_subscription_ack()
                    && matches!(conn.channel(), Channel::Server(_))
                    && matches!(conn.connection_type(), ConnType::Local)
                {
                    msg.get_slim_header_mut().header_mac = None;
                }

                match conn.channel() {
                    Channel::Server(s) => {
                        s.send(Ok(msg))
                            .await
                            .map_err(|e| DataPathError::MessageProcessingError {
                                source: Box::new(DataPathError::ConnectionNotFound(out_conn)),
                                msg: Box::new(e.0.unwrap_or_default()),
                            })
                    }
                    Channel::Client(s) => {
                        s.send(msg)
                            .await
                            .map_err(|e| DataPathError::MessageProcessingError {
                                source: Box::new(DataPathError::ConnectionNotFound(out_conn)),
                                msg: Box::new(e.0),
                            })
                    }
                }
            }
            None => Err(DataPathError::ConnectionNotFound(out_conn)),
        }
    }

    /// Send a gRPC status error on a server-side connection.
    /// This causes the client's stream to yield `Err(status)`.
    async fn send_status(&self, conn_index: u64, status: Status) {
        if let Some(conn) = self.forwarder().get_connection(conn_index)
            && let Channel::Server(tx) = conn.channel()
        {
            let _ = tx.send(Err(status)).await;
        }
    }

    async fn match_and_forward_msg(
        &self,
        #[cfg(feature = "otel_tracing")] mut msg: Message,
        #[cfg(not(feature = "otel_tracing"))] msg: Message,
        in_connection: u64,
        fanout: u32,
        filter: MatchFilter,
    ) -> Result<(), DataPathError> {
        let header = msg.get_slim_header();
        debug!(name = %header.get_dst(), %fanout, "match and forward message");

        // if the message already contains an output connection, use that one
        // without performing any match in the subscription table
        if let Some(val) = msg.get_forward_to() {
            debug!(conn = %val, "forwarding message to connection");
            return self.send_msg(msg, val).await;
        }

        let encoded = header.get_encoded_dst();

        match self
            .forwarder()
            .on_publish_msg_match(encoded, in_connection, fanout, filter)
        {
            Ok(out_vec) => {
                let len = out_vec.len();
                // Single destination: preserve per-connection span attributes.
                if len == 1 {
                    return self.send_msg(msg, out_vec[0]).await;
                }

                #[cfg(feature = "otel_tracing")]
                otel_tracing::prepare_fanout_msg(
                    &mut msg,
                    "send_message",
                    &self.internal.service_id,
                    len as u32,
                );

                let mut i = 0usize;
                while i < len - 1 {
                    self.send_msg_raw(msg.clone(), out_vec[i]).await?;
                    i += 1;
                }
                self.send_msg_raw(msg, out_vec[i]).await?;
                Ok(())
            }
            Err(e) => {
                debug!(name = %header.get_dst(), %fanout, error = %e, "no match for publish destination");
                Err(DataPathError::MessageProcessingError {
                    source: Box::new(e),
                    msg: Box::new(msg),
                })
            }
        }
    }

    /// Dispatch an inbound Link message to the appropriate handler.
    ///
    /// Link messages are link-local and must never be processed for local connections
    /// (they are only exchanged between SLIM nodes).
    async fn handle_link_message(
        &self,
        link: ProtoLink,
        conn_index: u64,
        category: ConnType,
    ) -> Result<(), DataPathError> {
        if category.is_local() {
            debug!(%conn_index, "ignoring link message received on local connection");
            return Ok(());
        }
        match link.link_type {
            Some(ProtoLinkType::LinkNegotiation(payload)) => {
                self.handle_link_negotiation(&payload, conn_index).await
            }
            None => {
                debug!(%conn_index, "received link message with unset link_type");
                Ok(())
            }
        }
    }

    /// Handle an inbound link negotiation message.
    ///
    /// Validates the role (client receives replies, server receives requests),
    /// parses the remote version, then delegates to the appropriate handler.
    async fn handle_link_negotiation(
        &self,
        payload: &LinkNegotiationPayload,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        let link_id = &payload.link_id;
        let remote_version = &payload.slim_version;

        debug!(
            %in_connection,
            %link_id,
            %remote_version,
            is_reply = payload.is_reply,
            "received link negotiation",
        );

        let Some(conn) = self.forwarder().get_connection(in_connection) else {
            debug!(%in_connection, "ignoring link negotiation request received on unknown connection");
            return Ok(());
        };

        // Role check: clients must only receive replies; servers must only receive requests.
        match (conn.is_outgoing(), payload.is_reply) {
            (true, false) => {
                debug!(%in_connection, "ignoring link negotiation request received on outgoing connection");
                return Ok(());
            }
            (false, true) => {
                debug!(%in_connection, "ignoring link negotiation reply received on incoming connection");
                return Ok(());
            }
            _ => {}
        }

        // Parse the remote version before any state mutation.
        let version = match semver::Version::parse(remote_version) {
            Ok(v) => v,
            Err(e) => {
                debug!(%in_connection, %remote_version, error = %e, "ignoring link negotiation with unparsable remote SLIM version");
                return Ok(());
            }
        };

        let strict = conn.require_header_mac();

        if payload.is_reply {
            self.handle_negotiation_reply(payload, in_connection, conn, link_id, version, strict)
        } else {
            self.handle_negotiation_request(payload, in_connection, conn, link_id, version, strict)
                .await
        }
    }

    /// Client path: process the server's link negotiation reply.
    ///
    /// Completes ECDH key exchange, installs header HMAC, and stores peer identity.
    fn handle_negotiation_reply(
        &self,
        payload: &LinkNegotiationPayload,
        in_connection: u64,
        conn: Arc<Connection>,
        link_id: &str,
        version: semver::Version,
        strict: bool,
    ) -> Result<(), DataPathError> {
        if strict && payload.link_ecdh_public_key.len() != X25519_PUBLIC_KEY_LEN {
            return Err(DataPathError::NegotiationError(
                "public key length is invalid".to_string(),
            ));
        }

        if !conn.complete_negotiation_as_client(link_id, version) {
            debug!(%in_connection, %link_id, "ignoring link negotiation reply");
            return Ok(());
        }

        // Store remote node identity for logging/diagnostics.
        if !payload.node_id.is_empty() {
            conn.set_peer_node_id(payload.node_id.clone());
        }

        if payload.link_ecdh_public_key.len() == X25519_PUBLIC_KEY_LEN
            && let Some(sk) = conn.take_outbound_ecdh_private()
        {
            match link_ecdh::derive_header_mac_from_ecdh(
                sk,
                payload.link_ecdh_public_key.as_slice(),
                link_id,
            ) {
                Ok(mac) => conn.install_header_hmac(mac),
                Err(e) => {
                    error!(
                        %in_connection,
                        error = %e,
                        "link ECDH key derivation failed (client path)",
                    );
                    return Err(DataPathError::NegotiationError(
                        "failed to generate client exchange key".to_string(),
                    ));
                }
            }
        }

        if strict && conn.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "strict header MAC required but link HMAC session is not installed".to_string(),
            ));
        }

        Ok(())
    }

    /// Server path: process an incoming link negotiation request.
    ///
    /// Performs ECDH key exchange, route recovery, sends reply, and handles
    /// peer upgrade if the client indicated connection_type == Peer.
    async fn handle_negotiation_request(
        &self,
        payload: &LinkNegotiationPayload,
        in_connection: u64,
        conn: Arc<Connection>,
        link_id: &str,
        version: semver::Version,
        strict: bool,
    ) -> Result<(), DataPathError> {
        if strict && payload.link_ecdh_public_key.len() != X25519_PUBLIC_KEY_LEN {
            return Err(DataPathError::NegotiationError(
                "public key length is invalid".to_string(),
            ));
        }

        if !conn.complete_negotiation_as_server(link_id, version) {
            debug!(%in_connection, %link_id, "ignoring link negotiation request");
            return Ok(());
        }

        // Store remote node identity for logging/diagnostics.
        if !payload.node_id.is_empty() {
            conn.set_peer_node_id(payload.node_id.clone());
        }

        // Server-side ECDH: generate ephemeral key, derive HMAC, include public key in reply.
        let server_reply_ecdh = self.negotiate_server_ecdh(
            &conn,
            payload.link_ecdh_public_key.as_slice(),
            link_id,
            in_connection,
            strict,
        )?;

        if strict && conn.header_hmac().is_none() {
            return Err(DataPathError::NegotiationError(
                "strict header MAC required but link HMAC session is not installed".to_string(),
            ));
        }

        // Route recovery: if the peer reconnected with a known link_id, restore routing state.
        self.recover_routes_for_link(link_id, in_connection).await;

        // Send reply (after state is committed).
        let reply_conn_type = LinkConnectionType::try_from(payload.connection_type)
            .unwrap_or(LinkConnectionType::Remote);
        let reply = ProtoMessage::builder().build_link_negotiation(
            link_id,
            local_version(),
            true,
            server_reply_ecdh,
            reply_conn_type,
            &self.internal.service_id,
            &self.internal.deployment_name,
        );
        if let Err(e) = self.send_msg(reply, in_connection).await {
            debug!(
                %in_connection,
                error = %e.chain(),
                "failed to send link negotiation reply",
            );
        }

        // Handle peer upgrade if client indicated peer connection_type.
        if payload.connection_type == LinkConnectionType::Peer as i32 {
            self.handle_peer_upgrade(payload, in_connection, link_id)
                .await?;
        }

        Ok(())
    }

    /// Server-side ECDH key exchange: generate ephemeral key pair, derive header HMAC.
    /// Returns the server's public key to include in the reply (None if ECDH was not performed).
    fn negotiate_server_ecdh(
        &self,
        conn: &Connection,
        peer_ecdh_public_key: &[u8],
        link_id: &str,
        in_connection: u64,
        strict: bool,
    ) -> Result<Option<Vec<u8>>, DataPathError> {
        if peer_ecdh_public_key.len() != X25519_PUBLIC_KEY_LEN {
            return Ok(None);
        }

        let (server_sk, server_pk) = link_ecdh::generate_x25519_ephemeral().map_err(|_| {
            error!(%in_connection, "failed to generate server link ECDH key");
            DataPathError::NegotiationError("failed to generate server exchange key".to_string())
        })?;

        match link_ecdh::derive_header_mac_from_ecdh(server_sk, peer_ecdh_public_key, link_id) {
            Ok(mac) => {
                conn.install_header_hmac(mac);
                Ok(Some(server_pk))
            }
            Err(e) => {
                error!(
                    %in_connection,
                    error = %e,
                    "link ECDH key derivation failed (server path)",
                );
                if strict {
                    Err(DataPathError::NegotiationError(
                        "failed to derive header MAC from link ECDH (server path)".to_string(),
                    ))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Restore routing state for a reconnecting peer whose link_id matches a recovery entry.
    async fn recover_routes_for_link(&self, link_id: &str, in_connection: u64) {
        let Some(entry) = self.remote_sync().recovery.take(link_id) else {
            return;
        };

        info!(%in_connection, %link_id, "recovering routes for reconnected peer");

        // Re-add local routing entries under the new connection index.
        for (name, sub_ids) in &entry.local_subs {
            for &subscription_id in sub_ids {
                if let Err(e) = self.forwarder().on_subscription_msg(
                    name.clone(),
                    in_connection,
                    ConnType::Remote,
                    true,
                    subscription_id,
                ) {
                    error!(
                        error = %e.chain(), %in_connection,
                        "error re-adding local subscription during recovery",
                    );
                }
            }
        }

        // Re-send subscriptions to the remote peer and rebuild tracking.
        self.restore_remote_subscriptions(&entry.remote_subs, in_connection, true)
            .await;
    }

    /// Upgrade a server-side connection to Peer after validating identity and deployment_name.
    /// Notifies PeerSyncManager or auto-registers in the forwarder (generic topology).
    async fn handle_peer_upgrade(
        &self,
        payload: &LinkNegotiationPayload,
        in_connection: u64,
        link_id: &str,
    ) -> Result<(), DataPathError> {
        // Reject self-connections (can happen when all replicas share the same config).
        if payload.node_id == self.internal.service_id {
            warn!(
                %in_connection, %link_id,
                "rejecting peer connection from self (same node_id)"
            );
            self.send_status(
                in_connection,
                Status::permission_denied("self-connection rejected: same node_id"),
            )
            .await;
            let _ = self.disconnect(in_connection);
            return Ok(());
        }

        // Verify deployment_name: if we have a deployment_name configured, the remote must match.
        if !self.internal.deployment_name.is_empty()
            && payload.deployment_name != self.internal.deployment_name
        {
            warn!(
                %in_connection, %link_id,
                local_group = %self.internal.deployment_name,
                remote_group = %payload.deployment_name,
                "rejecting peer upgrade: deployment_name mismatch"
            );
            self.send_status(
                in_connection,
                Status::permission_denied("deployment_name mismatch"),
            )
            .await;
            let _ = self.disconnect(in_connection);
            return Ok(());
        }

        let remote_node_id = payload.node_id.clone();
        info!(
            %in_connection, %link_id, %remote_node_id,
            "upgrading server-side connection to Peer (negotiation)"
        );
        self.connection_table().update(in_connection, |conn| {
            conn.set_connection_type(ConnType::Peer)
        });

        self.peer_sync()
            .on_incoming_peer(self, remote_node_id, in_connection);

        Ok(())
    }

    async fn process_publish(
        &self,
        msg: Message,
        in_connection: u64,
        filter: MatchFilter,
    ) -> Result<(), DataPathError> {
        debug!(
            %in_connection,
            ?msg,
            "received publication"
        );

        // telemetry /////////////////////////////////////////
        info!(
            telemetry = true,
            monotonic_counter.num_messages_by_type = 1,
            method = "publish"
        );
        //////////////////////////////////////////////////////

        // this function may panic, but at this point we are sure we are processing
        // a publish message
        let fanout = msg.get_fanout();

        self.match_and_forward_msg(msg, in_connection, fanout, filter)
            .await
    }

    pub(crate) async fn send_subscription_ack(
        &self,
        in_connection: u64,
        subscription_id: u64,
        result: &Result<(), DataPathError>,
    ) {
        let (success, error_msg) = match result {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e.to_string()),
        };

        let ack_msg =
            Message::builder().build_subscription_ack(subscription_id, success, error_msg);

        if let Err(e) = self.send_msg(ack_msg, in_connection).await {
            error!(error = %e.chain(), "failed to send subscription ack");
        }
    }

    /// Pure state update for a subscription: updates the subscription table
    /// and returns the outcome (whether a transition occurred, connection type, forward target).
    /// Does NOT perform any forwarding or event emission.
    fn update_subscription_state(
        &self,
        msg: &Message,
        conn: u64,
        forward: Option<u64>,
        add: bool,
        subscription_id: u64,
    ) -> Result<SubscriptionOutcome, DataPathError> {
        let dst = msg.get_dst();

        // As connection is deleted only after processing, at this point it must exist.
        let connection = if let Some(c) = self.forwarder().get_connection(conn) {
            c
        } else {
            return Err(DataPathError::ConnectionNotFound(conn));
        };

        debug!(
            %conn,
            %dst,
            is_local = connection.is_local_connection(),
            "processing {}subscription state",
            if add { "" } else { "un" }
        );

        let is_peer_conn = connection.is_peer_connection();

        let transition = self.forwarder().on_subscription_msg(
            dst,
            conn,
            connection.connection_type(),
            add,
            subscription_id,
        )?;

        Ok(SubscriptionOutcome {
            transition,
            is_peer_conn,
            forward_conn: forward,
        })
    }

    // Use a single function to process subscription and unsubscription packets.
    // The flag add = true is used to add a new subscription while add = false
    // is used to remove existing state.
    //
    // This is the SINGLE entry point for all subscription handling.
    // All forwarding (to peers, to controller, hub relay) goes through
    // the PeerSync — no inline forwarding anywhere else.
    async fn process_subscription(
        &self,
        msg: Message,
        in_connection: u64,
        add: bool,
    ) -> Result<(), DataPathError> {
        debug!(
            %in_connection,
            ?msg,
            "received {}subscription",
            if add { "" } else { "un" }
        );

        // telemetry /////////////////////////////////////////
        info!(
            telemetry = true,
            monotonic_counter.num_messages_by_type = 1,
            message_type = { if add { "subscribe" } else { "unsubscribe" } }
        );
        //////////////////////////////////////////////////////

        let subscription_id = msg.get_subscription_id();

        debug!(?subscription_id, "received subscription id");

        // get header
        let header = msg.get_slim_header();

        // get in and out connections
        let (in_conn, recv_from, forward) = header.get_connections();
        let in_conn = recv_from.unwrap_or(in_conn);

        // Never forward subscriptions to local connections (they are local apps whose
        // routes are already set locally).
        let forward = forward.filter(|&out| {
            self.forwarder()
                .get_connection(out)
                .map(|c| !c.is_local_connection())
                .unwrap_or(true)
        });

        // As connection is deleted only after processing, at this point it must exist.
        let Some(connection) = self.forwarder().get_connection(in_conn) else {
            if let Some(id) = subscription_id {
                debug!(%in_conn, "connection not found, sending error ack");
                self.send_subscription_ack(
                    in_connection,
                    id,
                    &Err(DataPathError::ConnectionNotFound(in_conn)),
                )
                .await;
            }
            return Err(DataPathError::MessageProcessingError {
                source: Box::new(DataPathError::ConnectionNotFound(in_conn)),
                msg: Box::new(msg),
            });
        };

        // Do not process subscriptions forwarded back to local connections.
        if recv_from.is_some() && connection.is_local_connection() {
            if let Some(id) = subscription_id {
                debug!(%in_conn, "subscription looped back to local connection, acking ok");
                self.send_subscription_ack(in_connection, id, &Ok(())).await;
            }
            return Ok(());
        }

        // Loop prevention: check if this subscription_id has already been forwarded
        // by this node. This prevents loops in ring/mesh topologies where a subscription
        // could travel around and come back. Only applies to subscribes (add=true);
        // unsubscribes with a seen sub_id are expected (they cancel a prior forwarded sub).
        // Unsubscribe loops are bounded by TTL and don't cause state corruption since
        // remove operations are idempotent.
        let sub_id = subscription_id.unwrap_or(0);
        if add && sub_id != 0 && self.peer_sync().has_seen_sub_id(sub_id) {
            debug!(
                %in_conn,
                %sub_id,
                "dropping subscription already forwarded by this node (loop prevention)"
            );
            if let Some(id) = subscription_id {
                self.send_subscription_ack(in_connection, id, &Ok(())).await;
            }
            return Ok(());
        }

        // Update local state (subscription table) — pure state change, no forwarding.
        let outcome = match self.update_subscription_state(&msg, in_conn, forward, add, sub_id) {
            Ok(o) => o,
            Err(e) => {
                if let Some(id) = subscription_id {
                    self.send_subscription_ack(in_connection, id, &Err(e)).await;
                    // Return Ok since we already sent the error ACK.
                    return Ok(());
                }
                return Err(DataPathError::MessageProcessingError {
                    source: Box::new(e),
                    msg: Box::new(msg),
                });
            }
        };

        // Determine forwarding targets:
        // - Peers (All): non-peer subscription with aggregate transition (0→1 or 1→0)
        // - Peers (ExcludeConn): peer subscription with remaining TTL >= 2 (relay)
        // - Forward conn: controller/remote node when header.forward_to is set
        //
        // TTL controls propagation depth:
        // - TTL=2 on initial send → peer decrements to 1, sees 1 < 2, no relay (full mesh)
        // - TTL=3 on initial send → hub decrements to 2, relays; spoke decrements to 1, stops
        // - TTL=6 on initial send → allows up to 5 hops of relay (generic topology)
        let remaining_ttl = msg.get_ttl();

        let (peer_target, peer_ttl) = if !outcome.is_peer_conn && outcome.transition {
            // Local/remote subscription transition → forward to ALL peers with configured TTL
            let ttl = self.peer_sync().subscription_ttl();
            (Some(crate::sync::PeerTarget::All), ttl)
        } else if outcome.is_peer_conn && remaining_ttl >= 2 {
            // Peer subscription relay: TTL allows further propagation.
            // Forward to all peers except the source, using remaining TTL.
            (
                Some(crate::sync::PeerTarget::ExcludeConn(in_conn)),
                remaining_ttl,
            )
        } else {
            (None, 0)
        };

        let targets = crate::sync::ForwardTargets {
            peers: peer_target,
            forward_conn: outcome.forward_conn,
        };

        // If there are forwarding targets, spawn the forwarder task (non-blocking).
        // The forwarder will wait for ACKs and then ACK the upstream client.
        if targets.has_any() {
            let fwd = self.peer_sync();
            let dst = msg.get_dst();
            debug!(
                %in_connection,
                %dst,
                %remaining_ttl,
                %peer_ttl,
                ?targets,
                "spawning subscription forwarder task"
            );
            let drain = self.get_drain_watch().ok();
            if let Some(drain) = drain {
                fwd.spawn_forward_and_ack(
                    self.clone(),
                    msg,
                    dst,
                    sub_id,
                    add,
                    targets,
                    in_connection,
                    subscription_id,
                    peer_ttl,
                    drain,
                );
                return Ok(());
            }
            // Fallback: drain not available (shutting down).
            // ACK immediately as best-effort.
        }

        // No forwarding needed (or no forwarder) — ACK immediately.
        if let Some(id) = subscription_id {
            debug!(%in_connection, "sending immediate subscription ack (no forwarding)");
            self.send_subscription_ack(in_connection, id, &Ok(())).await;
        }

        Ok(())
    }

    pub async fn process_message(
        &self,
        msg: Message,
        in_connection: u64,
        category: ConnType,
    ) -> Result<(), DataPathError> {
        match msg.message_type {
            Some(SubscribeType(_)) => self.process_subscription(msg, in_connection, true).await,
            Some(UnsubscribeType(_)) => self.process_subscription(msg, in_connection, false).await,
            Some(PublishType(_)) => {
                let filter = match category {
                    ConnType::Peer => {
                        if self.internal.relay_peer_publishes {
                            MatchFilter::ALL
                        } else {
                            MatchFilter::EXCLUDE_PEER
                        }
                    }
                    _ => MatchFilter::ALL,
                };
                self.process_publish(msg, in_connection, filter).await
            }
            Some(LinkType(link)) => {
                self.handle_link_message(link, in_connection, category)
                    .await
            }
            Some(SubscriptionAckType(ack)) => {
                let result = if ack.success {
                    Ok(())
                } else {
                    Err(DataPathError::RemoteSubscriptionAckError(ack.error))
                };

                self.peer_sync().resolve_ack(ack.subscription_id, result);
                Ok(())
            }
            None => unreachable!(
                "message type not set; validate() must be called before process_message"
            ),
        }
    }

    async fn handle_new_message(
        &self,
        conn_index: u64,
        category: ConnType,
        mut msg: Message,
    ) -> Result<(), DataPathError> {
        debug!(%conn_index, "received message from connection");
        info!(
            telemetry = true,
            monotonic_counter.num_processed_messages = 1
        );

        // validate message
        if let Err(err) = msg.validate() {
            info!(
                telemetry = true,
                monotonic_counter.num_messages_by_type = 1,
                message_type = "none"
            );

            let ret_err = DataPathError::MessageProcessingError {
                source: Box::new(err.into()),
                msg: Box::new(msg),
            };

            return Err(ret_err);
        }

        // Link and SubscriptionAck messages have no SLIM header: skip header processing and telemetry span.
        if !msg.is_link() && !msg.is_subscription_ack() {
            // add incoming connection to the SLIM header
            msg.set_incoming_conn(Some(conn_index));

            // TTL processing: decrement for remote messages (hop-by-hop)
            if !category.is_local() && msg.decrement_ttl() == 0 {
                debug!(%conn_index, "dropping message: TTL expired");
                return Err(DataPathError::TtlExpired);
            }

            #[cfg(feature = "otel_tracing")]
            otel_tracing::prepare_inbound_msg(
                &mut msg,
                "process_local",
                &self.internal.service_id,
                conn_index,
                category.is_local(),
            );
        }

        match self.process_message(msg, conn_index, category).await {
            Ok(_) => Ok(()),
            Err(e) => {
                // telemetry /////////////////////////////////////////
                info!(
                    telemetry = true,
                    monotonic_counter.num_message_process_errors = 1
                );
                //////////////////////////////////////////////////////

                // drop message
                Err(e)
            }
        }
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.internal.service_id, conn_index))]
    async fn send_error_to_local_app(&self, conn_index: u64, err: DataPathError) {
        debug!(%conn_index, "sending error to local application");
        let connection = self.forwarder().get_connection(conn_index);
        match connection {
            Some(conn) => {
                debug!("try to notify the error to the local application");
                if let Channel::Server(tx) = conn.channel() {
                    // If the error contains the message, try to extract some session information
                    let session_ctx = match &err {
                        DataPathError::MessageProcessingError { msg, .. } => {
                            MessageContext::from_msg(msg)
                        }
                        _ => None,
                    };

                    // Make error message with optional session context using shared type
                    let payload = crate::errors::ErrorPayload::new(err.to_string(), session_ctx);
                    let error_message = payload.to_json_string();

                    // create Status error
                    let status = Status::new(tonic::Code::Internal, error_message);

                    if tx.send(Err(status)).await.is_err() {
                        debug!(error = %err.chain(), "unable to notify the error to the local app");
                    }
                }
            }
            None => {
                error!(
                    "error sending error to local app: connection {:?} not found",
                    conn_index
                );
            }
        }
    }

    #[tracing::instrument(skip_all, fields(service_id = %self.internal.service_id, conn_index))]
    async fn reconnect(
        &self,
        client_conf: ClientConfig,
        conn_index: u64,
        cancellation_token: &CancellationToken,
    ) -> bool {
        info!("connection lost with remote endpoint, attempting to reconnect");

        let is_peer = self
            .forwarder()
            .get_connection(conn_index)
            .map(|c| c.connection_type() == ConnType::Peer)
            .unwrap_or(false);

        // For remote/controller connections: save the subscriptions we forwarded to this
        // connection so we can replay them after reconnecting.
        // For peer connections: we do a full sync instead (no need to save).
        let remote_subscriptions = if !is_peer {
            self.remote_sync()
                .get_subscriptions_for_reconnect(conn_index)
        } else {
            Default::default()
        };

        tokio::select! {
            _ = cancellation_token.cancelled() => {
                debug!("cancellation token signaled, stopping reconnection process");
                false
            }
            res = self.try_to_connect(client_conf, None, None, Some(conn_index)) => {
                match res {
                    Ok(_) => {
                        info!("connection re-established successfully");
                        if is_peer {
                            // Peer connection: full sync (send local + remote subscriptions).
                            let ttl = self.peer_sync().subscription_ttl();
                            if let Err(e) = sync_peer::send_local_remote_sync(
                                self, conn_index, ttl,
                            )
                            .await
                            {
                                warn!(
                                    error = %e,
                                    "failed to send full sync after peer reconnect"
                                );
                            }
                        } else {
                            // Remote/controller: restore only what was previously forwarded.
                            self.restore_remote_subscriptions(
                                &remote_subscriptions,
                                conn_index,
                                false,
                            )
                            .await;
                        }
                        true
                    }
                    Err(e) => {
                        error!(error = %e.chain(), "unable to reconnect to remote node");
                        false
                    }
                }
            }
        }
    }

    /// Send an UNSUBSCRIBE message to the control plane for each subscription in `local_subs`.
    ///
    /// This is the single authoritative place that constructs and delivers CP unsubscribe
    /// notifications on connection loss, used by both the immediate cleanup path and the deferred
    /// TTL-expiry path.
    async fn notify_control_plane_subscriptions_lost(
        tx_cp: Option<Sender<Result<Message, Status>>>,
        local_subs: HashMap<ProtoName, HashSet<u64>>,
        conn_index: u64,
    ) {
        let Some(tx) = tx_cp else { return };
        for local_sub in local_subs.into_keys() {
            debug!(
                %local_sub,
                "notify control plane about lost subscription",
            );
            let msg = Message::builder()
                .source(local_sub.clone())
                .destination(local_sub.clone())
                .flags(SlimHeaderFlags::default().with_recv_from(conn_index))
                .build_unsubscribe()
                .unwrap();
            if let Err(e) = tx.send(Ok(msg)).await {
                debug!(
                    %local_sub,
                    error = %e.chain(),
                    "failed to send unsubscribe to control plane",
                );
            }
        }
    }

    fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
        conn_index: u64,
        client_config: Option<ClientConfig>,
        cancellation_token: CancellationToken,
        category: ConnType,
        from_control_plane: bool,
    ) -> Result<JoinHandle<()>, DataPathError> {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();
        let client_conf_clone = client_config.clone();
        let tx_cp: Option<Sender<Result<Message, Status>>> = self.get_tx_control_plane();
        let watch = self.get_drain_watch()?;
        let is_local = category.is_local();
        let span = tracing::info_span!(
            "process_stream",
            service_id = %self.internal.service_id,
            %conn_index,
            is_local,
        );
        let require_header_mac = self
            .forwarder()
            .get_connection(conn_index)
            .map(|c| c.require_header_mac())
            .unwrap_or(false);

        let handle = tokio::spawn(async move {
            let mut try_to_reconnect = true;
            let mut category = category;

            let mut watch = std::pin::pin!(watch.signaled());
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
                                        if !is_local
                                            && !msg.is_link()
                                            && !msg.is_subscription_ack()
                                            && let Err(e) = self_clone
                                                .verify_remote_header_mac(conn_index, &msg, require_header_mac)
                                        {
                                            error!(
                                                %conn_index,
                                                error = %e.chain(),
                                                "SLIM header integrity verification failed",
                                            );
                                            continue;
                                        }
                                        // check if we need to send the message to the control plane
                                        // we send the message if
                                        // 1. the message is coming from remote
                                        // 2. it is not coming from the control plane itself
                                        // 3. the control plane exists
                                        if !is_local && !from_control_plane && let Some(txcp) = &tx_cp {
                                            match msg.get_type() {
                                                PublishType(_) | LinkType(_) | SubscriptionAckType(_) => {/* do nothing */}
                                                _ => {
                                                    // send subscriptions and unsubscriptions
                                                    // to the control plane
                                                    let _ = txcp.send(Ok(msg.clone())).await;
                                                }
                                            }
                                        }

                                        let is_link_msg = msg.is_link();

                                        if let Err(e) = self_clone.handle_new_message(conn_index, category, msg).await {
                                            // Checking if NegotiationError occurred
                                            if matches!(e, DataPathError::NegotiationError(_)) {
                                                error!(%conn_index, "fatal link negotiation error, closing connection");
                                                try_to_reconnect = false;
                                                break;
                                            }
                                            debug!(%conn_index, error = %e.chain(), "error processing incoming message");
                                            // If the message is coming from a local app, notify it
                                            if is_local {
                                                // try to forward error to the local app
                                                self_clone.send_error_to_local_app(conn_index, e).await;
                                            }
                                        }

                                        // After link negotiation the connection type may have
                                        // been upgraded (e.g. Remote → Peer). Cache the new
                                        // value so subsequent messages use the correct category.
                                        if is_link_msg {
                                            if let Some(conn) = self_clone.forwarder().get_connection(conn_index) {
                                                category = conn.connection_type();
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if e.code() == tonic::Code::PermissionDenied {
                                            warn!(
                                                %conn_index,
                                                message = %e.message(),
                                                "connection rejected by remote, will not reconnect"
                                            );
                                            try_to_reconnect = false;
                                        } else if let Some(io_err) = MessageProcessor::match_for_io_error(&e) {
                                            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                                                info!(%conn_index, "connection closed by peer");
                                            }
                                        } else {
                                            error!(error = %e.chain(), "error receiving messages");
                                        }
                                        break;
                                    }
                                }
                            }
                            None => {
                                debug!(%conn_index, "end of stream");
                                break;
                            }
                        }
                    }
                    _ = &mut watch => {
                        info!(%conn_index, "shutting down stream on drain");
                        try_to_reconnect = false;
                        break;
                    }
                    _ = token_clone.cancelled() => {
                        info!(%conn_index, "shutting down stream on cancellation token");
                        try_to_reconnect = false;
                        break;
                    }
                }
            }

            // we drop rx now as otherwise the connection will be closed only
            // when the task is dropped and we want to make sure that the rx
            // stream is closed as soon as possible
            drop(stream);

            // Save whether this is a client-initiated connection before client_conf_clone
            // is consumed by the if-let below.
            let is_client_connection = client_conf_clone.is_some();
            let mut connected = false;

            if try_to_reconnect && let Some(config) = client_conf_clone {
                // Break the span chain: reconnect → try_to_connect → process_stream
                // would otherwise nest under the current process_stream span on every
                // reconnection, growing the span hierarchy unboundedly.
                connected = self_clone.reconnect(config, conn_index, &token_clone)
                    .instrument(tracing::Span::none())
                    .await;
            } else {
                debug!(%conn_index, "close connection")
            }

            if !connected {
                // For incoming (server) connections capture the link_id before
                // on_connection_drop removes the connection from the table.
                let link_id = if !is_local && !is_client_connection {
                    self_clone
                        .forwarder()
                        .get_connection(conn_index)
                        .and_then(|c| c.link_id())
                } else {
                    None
                };

                // Delete connection state from all tables.
                let local_subs = self_clone
                    .forwarder()
                    .on_connection_drop(conn_index, category);
                let remote_subs = self_clone
                    .remote_sync()
                    .on_connection_drop(conn_index);

                // Remove peer connection from forwarder's peer list if applicable.
                if matches!(category, ConnType::Peer) {
                    self_clone.peer_sync().remove_peer_conn(conn_index);
                }

                // Notify peer sync about names that are no longer reachable.
                // For generic topologies (TTL-based relay), we also need to notify
                // when a peer drops — the seen_sub_ids tracking ensures we only
                // send unsubscribes for subscriptions we actually forwarded.
                {
                    let fwd = self_clone.peer_sync();
                    for name in local_subs.keys() {
                        let still_reachable = name.name.is_some_and(|enc| {
                            self_clone
                                .forwarder()
                                .on_publish_msg_match(enc, u64::MAX, u32::MAX, MatchFilter::ALL)
                                .is_ok()
                        });
                        if !still_reachable {
                            debug!(
                                %name,
                                %conn_index,
                                ?category,
                                "notifying peers of unsubscription (connection drop)"
                            );
                            fwd.notify_peers_unsubscribe(&self_clone, name).await;
                        } else {
                            debug!(
                                %name,
                                %conn_index,
                                ?category,
                                "name still reachable, not emitting removal"
                            );
                        }
                    }
                }

                let recovery_enabled =
                    !self_clone.remote_sync().recovery.ttl().is_zero();

                // Peer connections use full sync on reconnect — no recovery table needed.
                let use_recovery = recovery_enabled && !matches!(category, ConnType::Peer);

                if let Some(lid) = link_id.filter(|_| use_recovery) {
                    // Server connection with a known link_id: preserve routing state and
                    // suppress the control-plane notification for the duration of the TTL
                    // to give the peer a chance to reconnect.
                    info!(
                        %conn_index, %lid,
                        "connection lost, storing recovery state (TTL: {:?})",
                        self_clone.remote_sync().recovery.ttl(),
                    );
                    self_clone
                        .remote_sync()
                        .recovery
                        .store(lid.clone(), local_subs, remote_subs);

                    // Spawn a TTL task that fires the CP notification if recovery never happens.
                    if let Ok(drain) = self_clone.get_drain_watch() {
                        let tx_cp_ttl = tx_cp;
                        let mp = self_clone.clone();
                        self_clone.remote_sync().recovery.spawn_ttl_task(
                            lid,
                            drain,
                            move |entry| async move {
                                info!("recovery window expired, notifying control plane");
                                // Only unsubscribe names that are no longer reachable.
                                // If the peer reconnected with a different link_id, the
                                // CP will have already pushed the same subscriptions on
                                // the new connection — those names are still in the
                                // subscription table and must not be torn down.
                                let unreachable = entry
                                    .local_subs
                                    .into_iter()
                                    .filter(|(name, _)| {
                                        mp.forwarder()
                                            .on_publish_msg_match(name.name.unwrap(), u64::MAX, u32::MAX, MatchFilter::ALL)
                                            .is_err()
                                    })
                                    .collect();
                                MessageProcessor::notify_control_plane_subscriptions_lost(
                                    tx_cp_ttl,
                                    unreachable,
                                    conn_index,
                                )
                                .await;
                            },
                        );
                    }
                } else {
                    // No link_id (local connection, client that failed to reconnect, or a peer
                    // that does not support link negotiation): notify the control plane now.
                    if !is_local {
                        MessageProcessor::notify_control_plane_subscriptions_lost(
                            tx_cp, local_subs, conn_index,
                        )
                        .await;
                    }
                }

                info!(telemetry = true, counter.num_active_connections = -1);
            }
        }.instrument(span));

        Ok(handle)
    }

    fn match_for_io_error(err_status: &Status) -> Option<&std::io::Error> {
        let mut err: &(dyn std::error::Error + 'static) = err_status;

        loop {
            if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
                return Some(io_err);
            }

            // h2::Error do not expose std::io::Error with `source()`
            // https://github.com/hyperium/h2/pull/462
            if let Some(h2_err) = err.downcast_ref::<h2::Error>()
                && let Some(io_err) = h2_err.get_io()
            {
                return Some(io_err);
            }

            err = err.source()?;
        }
    }

    pub fn subscription_table(&self) -> &SubscriptionTableImpl {
        &self.internal.forwarder.subscription_table
    }

    pub fn connection_table(&self) -> &ConnectionTable<Connection> {
        &self.internal.forwarder.connection_table
    }

    /// The node identity used for cross-node communication.
    pub fn service_id(&self) -> &str {
        &self.internal.service_id
    }

    /// Set the peer sync component.
    pub fn set_peer_sync(&self, peer_sync: crate::sync::PeerSync) {
        *self.internal.peer_sync.write() = peer_sync;
    }

    /// Get a clone of the peer sync component.
    pub(crate) fn peer_sync(&self) -> crate::sync::PeerSync {
        self.internal.peer_sync.read().clone()
    }
}

impl ServerHandler for MessageProcessor {
    fn grpc_routes(&self) -> Option<tonic::service::Routes> {
        let svc = DataPlaneServiceServer::from_arc(Arc::new(self.clone()));
        Some(tonic::service::Routes::new(svc))
    }

    fn on_websocket_accepted(&self) -> Option<websocket_server::OnAcceptedWebSocket> {
        let processor = self.clone();
        Some(Arc::new(move |accepted| {
            let processor = processor.clone();
            Box::pin(async move { processor.handle_websocket_accepted(accepted).await })
        }))
    }
}

#[tonic::async_trait]
impl DataPlaneService for MessageProcessor {
    type OpenChannelStream = Pin<Box<dyn Stream<Item = Result<Message, Status>> + Send + 'static>>;

    async fn open_channel(
        &self,
        request: Request<tonic::Streaming<Message>>,
    ) -> Result<Response<Self::OpenChannelStream>, Status> {
        let remote_addr = request.remote_addr();
        let local_addr = request.local_addr();

        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        let connection = Connection::new(ConnType::Remote, Channel::Server(tx))
            .with_remote_addr(remote_addr)
            .with_local_addr(local_addr)
            .with_require_header_mac(self.internal.server_require_header_mac);

        debug!(
            remote = ?connection.remote_addr(),
            local = ?connection.local_addr(),
            "new connection received from remote",
        );
        info!(telemetry = true, counter.num_active_connections = 1);

        // insert connection into connection table
        let conn_index = self
            .forwarder()
            .on_connection_established(connection, None)
            .unwrap();

        self.process_stream(
            stream,
            conn_index,
            None,
            CancellationToken::new(),
            ConnType::Remote,
            false,
        )
        .map_err(|e| {
            error!(error = %e.chain(), "error starting new processing stream");
            Status::unavailable(format!("error processing stream: {:?}", e))
        })?;

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}

#[cfg(test)]
mod tests {
    use slim_config::client::ClientConfig;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::api::{ProtoName, ProtoSubscriptionAck};
    use crate::header_mac::HeaderMacSession;
    use crate::sync::remote::SubscriptionInfo;
    use tonic::Status;

    async fn assert_failed_subscription_ack_is_sent(add: bool) {
        let processor = MessageProcessor::new();
        let (in_connection, _tx, mut rx) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");

        let source = ProtoName::from_strings(["org", "ns", "source"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "destination"]).with_id(2);
        let ack_id: u64 = if add { 1 } else { 2 };
        let invalid_connection = u64::MAX - 1;

        let builder = Message::builder()
            .source(source.clone())
            .destination(destination.clone())
            .incoming_conn(invalid_connection)
            .subscription_id(ack_id);

        let msg = if add {
            builder.build_subscribe().unwrap()
        } else {
            builder.build_unsubscribe().unwrap()
        };

        let result = processor
            .process_subscription(msg, in_connection, add)
            .await;
        assert!(matches!(
            result,
            Err(DataPathError::MessageProcessingError { .. })
        ));

        let ack_msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting for ack")
            .expect("ack channel closed")
            .expect("failed to receive ack message");

        assert!(matches!(ack_msg.get_type(), SubscriptionAckType(_)));
        let ack = ack_msg.get_subscription_ack();
        assert_eq!(ack.subscription_id, ack_id);
        assert!(!ack.success, "failed ack should have success=false");
        assert!(
            !ack.error.is_empty(),
            "failed ack should include an error message"
        );
    }

    #[tokio::test]
    async fn test_process_subscription_sends_failed_ack_on_subscribe_error() {
        assert_failed_subscription_ack_is_sent(true).await;
    }

    #[tokio::test]
    async fn test_process_subscription_sends_failed_ack_on_unsubscribe_error() {
        assert_failed_subscription_ack_is_sent(false).await;
    }

    // ── handle_link_message ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_link_message_is_local_ignored() {
        let processor = MessageProcessor::new();
        let link = ProtoLink { link_type: None };
        assert!(
            processor
                .handle_link_message(link, 0, ConnType::Local)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_handle_link_message_none_link_type_ignored() {
        let processor = MessageProcessor::new();
        let link = ProtoLink { link_type: None };
        assert!(
            processor
                .handle_link_message(link, 0, ConnType::Remote)
                .await
                .is_ok()
        );
    }

    // ── handle_link_negotiation ───────────────────────────────────────────────

    fn make_server_conn(
        processor: &MessageProcessor,
    ) -> (u64, tokio::sync::mpsc::Receiver<Result<Message, Status>>) {
        let (tx, rx) = mpsc::channel(16);
        let conn = Connection::new(ConnType::Remote, Channel::Server(tx))
            .with_require_header_mac(processor.internal.server_require_header_mac);
        let conn_id = processor
            .forwarder()
            .on_connection_established(conn, None)
            .unwrap();
        (conn_id, rx)
    }

    fn make_client_conn(
        processor: &MessageProcessor,
    ) -> (u64, tokio::sync::mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel(16);
        let conn = Connection::new(ConnType::Remote, Channel::Client(tx))
            .with_config_data(Some(ClientConfig::default()));
        let conn_id = processor
            .forwarder()
            .on_connection_established(conn, None)
            .unwrap();
        (conn_id, rx)
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_unknown_connection_ignored() {
        let processor = MessageProcessor::new();
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, u64::MAX)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_role_outgoing_receives_request_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_client_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "1.0.0".into(),
            is_reply: false, // request on outgoing connection → ignored
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(
            processor
                .forwarder()
                .get_connection(conn_id)
                .unwrap()
                .remote_slim_version()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_role_incoming_receives_reply_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_server_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "1.0.0".into(),
            is_reply: true, // reply on incoming connection → ignored
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(
            processor
                .forwarder()
                .get_connection(conn_id)
                .unwrap()
                .remote_slim_version()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_unparsable_version_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_server_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "not-semver".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(
            processor
                .forwarder()
                .get_connection(conn_id)
                .unwrap()
                .remote_slim_version()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_server_empty_link_id_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_server_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: "".into(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(
            processor
                .forwarder()
                .get_connection(conn_id)
                .unwrap()
                .remote_slim_version()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_server_strict_rejects_missing_ecdh() {
        let mut server_config = ServerConfig::with_endpoint("127.0.0.1:0");
        server_config.require_header_mac = true;
        let processor = MessageProcessor::new_with_server_config(
            "test".into(),
            String::new(),
            &server_config,
            None,
            false,
        );
        let (conn_id, _rx) = make_server_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "1.2.3".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        let err = processor
            .handle_link_negotiation(&payload, conn_id)
            .await
            .expect_err("strict mode must reject negotiation without peer ECDH");
        assert!(matches!(err, DataPathError::NegotiationError(_)));
        let conn = processor.forwarder().get_connection(conn_id).unwrap();
        assert!(conn.remote_slim_version().is_none());
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_server_happy_path() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);
        let link_id = uuid::Uuid::new_v4().to_string();
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.2.3".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        let conn = processor.forwarder().get_connection(conn_id).unwrap();
        assert_eq!(conn.link_id(), Some(link_id));
        assert_eq!(
            conn.remote_slim_version(),
            Some(semver::Version::parse("1.2.3").unwrap())
        );
        // A reply must have been sent.
        let reply = rx.try_recv().expect("reply should be sent").unwrap();
        assert!(reply.is_link());
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_server_replay_protection() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);
        let link_id = uuid::Uuid::new_v4().to_string();
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        // First request: accepted, reply sent.
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(rx.try_recv().is_ok());
        // Second request: replay protection must suppress it, no reply.
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_client_happy_path() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_client_conn(&processor);
        let link_id = uuid::Uuid::new_v4().to_string();
        let conn = processor.forwarder().get_connection(conn_id).unwrap();
        conn.set_link_id(link_id.clone());
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "2.0.0".into(),
            is_reply: true,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert_eq!(
            conn.remote_slim_version(),
            Some(semver::Version::parse("2.0.0").unwrap())
        );
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_client_link_id_mismatch_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_client_conn(&processor);
        let conn = processor.forwarder().get_connection(conn_id).unwrap();
        conn.set_link_id("correct-id".to_string());
        let payload = LinkNegotiationPayload {
            link_id: "wrong-id".into(),
            slim_version: "1.0.0".into(),
            is_reply: true,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert!(conn.remote_slim_version().is_none());
    }

    #[tokio::test]
    async fn test_handle_link_negotiation_client_replay_protection() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_client_conn(&processor);
        let link_id = uuid::Uuid::new_v4().to_string();
        let conn = processor.forwarder().get_connection(conn_id).unwrap();
        conn.set_link_id(link_id.clone());
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: true,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        // First reply: accepted.
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        let stored = conn.remote_slim_version();
        assert!(stored.is_some());
        // Second reply: replay protection must reject it; version unchanged.
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
        assert_eq!(conn.remote_slim_version(), stored);
    }

    // ── process_subscription: remote ack path ─────────────────────────────────

    /// Helper: negotiate a server connection to version `v` and install a test HMAC session
    /// so `subscription_ack::supports` matches a fully established inter-node link.
    fn negotiate_conn(processor: &MessageProcessor, conn_id: u64, version: &str) {
        let c = processor.forwarder().get_connection(conn_id).unwrap();
        c.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::parse(version).unwrap(),
        );
        c.test_install_header_mac(Arc::new(
            HeaderMacSession::new(b"01234567890123456789012345678901").unwrap(),
        ));
    }

    #[tokio::test]
    async fn test_await_link_hmac_ready_timeout_configurable() {
        let server_config = ServerConfig {
            endpoint: "localhost:12345".to_string(),
            link_hmac_timeout_secs: 1,      // 1 second timeout
            link_hmac_poll_interval_ms: 10, // 10 milliseconds poll interval
            ..Default::default()
        };
        let processor = MessageProcessor::new_with_server_config(
            "test_service".to_string(),
            String::new(),
            &server_config,
            None,
            false,
        );

        assert_eq!(
            processor.internal.link_hmac_timeout,
            std::time::Duration::from_secs(1)
        );
        assert_eq!(
            processor.internal.link_hmac_poll_interval,
            std::time::Duration::from_millis(10)
        );

        // Register a connection but do not install any HMAC session
        let (conn_id, _tx, _rx) = processor
            .register_local_connection(false)
            .expect("failed to register local connection");

        // Measure time taken to fail
        let start = std::time::Instant::now();
        let result = processor.await_link_hmac_ready(conn_id, true).await;
        let elapsed = start.elapsed();

        assert!(result.is_err());
        assert!(elapsed >= std::time::Duration::from_millis(900));
        assert!(elapsed < std::time::Duration::from_secs(3));
    }

    #[test]
    fn verify_remote_header_mac_strict_rejects_publish_without_mac_session() {
        let processor = MessageProcessor::new();
        let (remote_conn, _rx) = make_server_conn(&processor);
        let conn = processor.forwarder().get_connection(remote_conn).unwrap();
        conn.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::parse("1.2.0").unwrap(),
        );
        assert!(conn.header_hmac().is_none());

        let source = ProtoName::from_strings(["org", "default", "a"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "default", "b"]).with_id(2);
        let msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
            .application_payload("text/plain", b"hey".to_vec())
            .build_publish()
            .expect("publish");

        let err = processor
            .verify_remote_header_mac(remote_conn, &msg, true)
            .expect_err("unsigned publish must fail in strict mode without MAC session");
        assert!(matches!(err, DataPathError::NegotiationError(_)));
    }

    #[test]
    fn verify_remote_header_mac_accepts_signed_inter_node_publish() {
        let processor = MessageProcessor::new();
        let (remote_conn, _rx) = make_server_conn(&processor);
        negotiate_conn(&processor, remote_conn, "1.2.0");
        let link_id = processor
            .forwarder()
            .get_connection(remote_conn)
            .unwrap()
            .link_id()
            .expect("link id after negotiation");

        let source = ProtoName::from_strings(["org", "default", "a"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "default", "b"]).with_id(2);
        let require_header_mac = true;
        let mut msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
            .application_payload("text/plain", b"hey".to_vec())
            .build_publish()
            .expect("publish");

        let mac = HeaderMacSession::new(b"01234567890123456789012345678901").unwrap();
        mac.sign_slim_header(msg.get_slim_header_mut(), &link_id)
            .expect("sign header");

        assert!(
            processor
                .verify_remote_header_mac(remote_conn, &msg, require_header_mac)
                .is_ok()
        );
    }

    #[test]
    fn verify_remote_header_mac_rejects_destination_tamper_after_sign() {
        let processor = MessageProcessor::new();
        let (remote_conn, _rx) = make_server_conn(&processor);
        negotiate_conn(&processor, remote_conn, "1.2.0");
        let link_id = processor
            .forwarder()
            .get_connection(remote_conn)
            .unwrap()
            .link_id()
            .expect("link id after negotiation");

        let source = ProtoName::from_strings(["org", "default", "a"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "default", "b"]).with_id(2);
        let mut msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
            .application_payload("text/plain", b"hey".to_vec())
            .build_publish()
            .expect("publish");

        let mac = HeaderMacSession::new(b"01234567890123456789012345678901").unwrap();
        let require_header_mac = true;
        mac.sign_slim_header(msg.get_slim_header_mut(), &link_id)
            .expect("sign header");

        let header = msg.get_slim_header_mut();
        if let Some(dest) = header.destination.as_mut()
            && let Some(sn) = dest.str_name.as_mut()
        {
            sn.str_component_2.push_str("-integrity-test-tamper");
        }

        let err = processor
            .verify_remote_header_mac(remote_conn, &msg, require_header_mac)
            .expect_err("tampered header must fail MAC verify");
        assert!(matches!(err, DataPathError::HeaderIntegrity(_)));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods)]
    async fn test_send_msg_raw_tamper_destination_env_var() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            std::env::set_var("SLIM_TEST_TAMPER_DESTINATION", "1");
        }

        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);
        negotiate_conn(&processor, conn_id, "1.2.0");

        let source = ProtoName::from_strings(["org", "default", "a"]).with_id(1);
        let dest = ProtoName::from_strings(["org", "default", "b"]).with_id(2);
        let msg = ProtoMessage::builder()
            .source(source)
            .destination(dest)
            .application_payload("text/plain", b"hey".to_vec())
            .build_publish()
            .expect("publish");

        processor
            .send_msg_raw(msg, conn_id)
            .await
            .expect("send_msg_raw failed");

        let sent_msg = rx.recv().await.unwrap().unwrap();
        let header = sent_msg.get_slim_header();
        let dest_name = header.destination.as_ref().expect("destination");
        let str_name = dest_name.str_name.as_ref().expect("str_name");
        let require_header_mac = true;

        // The tampering happens in send_msg_raw if the env var is set.
        assert!(str_name.str_component_2.ends_with("-integrity-test-tamper"));

        // Also verify that verify_remote_header_mac rejects it.
        let err = processor
            .verify_remote_header_mac(conn_id, &sent_msg, require_header_mac)
            .expect_err("tampered header must fail MAC verify");
        assert!(matches!(err, DataPathError::HeaderIntegrity(_)));

        unsafe {
            std::env::remove_var("SLIM_TEST_TAMPER_DESTINATION");
        }
    }

    #[tokio::test]
    async fn test_process_subscription_remote_ack_path_success() {
        // Arrange: relay processor, local app connection, and a "remote" server
        // connection whose version is ≥ 1.2.0.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");

        let (remote_conn, mut rx_remote) = make_server_conn(&processor);
        negotiate_conn(&processor, remote_conn, "1.2.0");

        let source = ProtoName::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "dst"]).with_id(2);
        let upstream_ack_id: u64 = 100;

        // Build subscribe: forward_to = remote_conn, with upstream ack ID.
        let sub_msg = Message::builder()
            .source(source.clone())
            .destination(destination.clone())
            .incoming_conn(local_conn)
            .forward_to(remote_conn)
            .subscription_id(upstream_ack_id)
            .build_subscribe()
            .unwrap();

        // Act: process_subscription should spawn the retry task and return Ok(()).
        let result = processor
            .process_subscription(sub_msg, local_conn, true)
            .await;
        assert!(result.is_ok());

        // The relay must have forwarded the subscribe to the remote connection.
        // Give the spawned task a moment to send the message.
        let forwarded = tokio::time::timeout(Duration::from_secs(1), rx_remote.recv())
            .await
            .expect("timeout waiting for forwarded subscribe")
            .expect("forwarded subscribe channel closed")
            .unwrap();
        assert!(matches!(forwarded.get_type(), SubscribeType(_)));

        // The forwarded message must carry the same subscription_id as the original.
        let forwarded_sub_id = forwarded
            .get_subscription_id()
            .expect("forwarded subscribe must carry the same subscription_id");
        assert_eq!(
            forwarded_sub_id, upstream_ack_id,
            "subscription_id must not change when forwarding"
        );

        // Simulate the remote node sending back a success SubscriptionAck.
        let ack = ProtoSubscriptionAck {
            subscription_id: upstream_ack_id,
            success: true,
            error: String::new(),
        };
        processor.peer_sync().resolve_ack(
            ack.subscription_id,
            if ack.success {
                Ok(())
            } else {
                Err(DataPathError::RemoteSubscriptionAckError(ack.error.clone()))
            },
        );

        // The relay must now forward the upstream ACK to the local connection.
        let upstream_ack = tokio::time::timeout(Duration::from_secs(2), rx_local.recv())
            .await
            .expect("timeout waiting for upstream ack")
            .expect("upstream ack channel closed")
            .expect("upstream ack should be Ok");

        assert!(matches!(upstream_ack.get_type(), SubscriptionAckType(_)));
        let ack_inner = upstream_ack.get_subscription_ack();
        assert_eq!(ack_inner.subscription_id, upstream_ack_id);
        assert!(ack_inner.success);
    }

    #[tokio::test]
    async fn test_process_subscription_remote_ack_error_forwarded_upstream() {
        // Remote node (v1.2.0) sends back a failure ACK; relay must forward it upstream.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");

        let (remote_conn, mut rx_remote) = make_server_conn(&processor);
        negotiate_conn(&processor, remote_conn, "1.2.0");

        let source = ProtoName::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "dst"]).with_id(2);
        let upstream_ack_id: u64 = 102;

        let sub_msg = Message::builder()
            .source(source.clone())
            .destination(destination.clone())
            .incoming_conn(local_conn)
            .forward_to(remote_conn)
            .subscription_id(upstream_ack_id)
            .build_subscribe()
            .unwrap();

        processor
            .process_subscription(sub_msg, local_conn, true)
            .await
            .unwrap();

        let forwarded = tokio::time::timeout(Duration::from_secs(1), rx_remote.recv())
            .await
            .expect("timeout")
            .expect("channel closed")
            .unwrap();

        let forwarded_sub_id = forwarded
            .get_subscription_id()
            .expect("forwarded subscribe must carry the same subscription_id");
        assert_eq!(
            forwarded_sub_id, upstream_ack_id,
            "subscription_id must not change when forwarding"
        );

        // Simulate remote failure via SubscriptionAck.
        let ack = ProtoSubscriptionAck {
            subscription_id: upstream_ack_id,
            success: false,
            error: "remote error".to_string(),
        };
        processor.peer_sync().resolve_ack(
            ack.subscription_id,
            if ack.success {
                Ok(())
            } else {
                Err(DataPathError::RemoteSubscriptionAckError(ack.error.clone()))
            },
        );

        let upstream_ack = tokio::time::timeout(Duration::from_secs(2), rx_local.recv())
            .await
            .expect("timeout")
            .expect("channel closed")
            .expect("must be Ok");

        assert!(matches!(upstream_ack.get_type(), SubscriptionAckType(_)));
        let ack_inner = upstream_ack.get_subscription_ack();
        assert_eq!(ack_inner.subscription_id, upstream_ack_id);
        assert!(!ack_inner.success);
        assert!(!ack_inner.error.is_empty());
    }

    // ── new_with_options ──────────────────────────────────────────────────────

    #[test]
    fn test_new_with_options_custom_ttl() {
        let processor =
            MessageProcessor::new_with_options("svc".into(), Some(Duration::from_secs(5)));
        assert_eq!(
            processor.remote_sync().recovery.ttl(),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn test_new_with_options_none_uses_default() {
        let processor = MessageProcessor::new_with_options("svc".into(), None);
        assert_eq!(
            processor.remote_sync().recovery.ttl(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_new_with_options_zero_ttl() {
        let processor = MessageProcessor::new_with_options("svc".into(), Some(Duration::ZERO));
        assert!(processor.remote_sync().recovery.ttl().is_zero());
    }

    // ── notify_control_plane_subscriptions_lost ───────────────────────────────

    #[tokio::test]
    async fn test_notify_cp_subs_lost_sends_unsubscribes() {
        let (tx, mut rx) = mpsc::channel::<Result<Message, Status>>(16);
        let mut subs = HashMap::new();
        let name = ProtoName::from_strings(["org", "default", "svc"]);
        subs.insert(name.clone(), HashSet::from([1u64, 2u64]));

        MessageProcessor::notify_control_plane_subscriptions_lost(Some(tx), subs, 42).await;

        let msg = rx.recv().await.unwrap().unwrap();
        assert!(matches!(msg.get_type(), UnsubscribeType(_)));
        assert_eq!(msg.get_source(), name.clone());
    }

    #[tokio::test]
    async fn test_notify_cp_subs_lost_no_tx_is_noop() {
        let subs = HashMap::from([(
            ProtoName::from_strings(["org", "default", "svc"]),
            HashSet::from([1u64]),
        )]);
        // Should not panic or hang.
        MessageProcessor::notify_control_plane_subscriptions_lost(None, subs, 1).await;
    }

    #[tokio::test]
    async fn test_notify_cp_subs_lost_empty_subs() {
        let (tx, mut rx) = mpsc::channel::<Result<Message, Status>>(16);
        MessageProcessor::notify_control_plane_subscriptions_lost(Some(tx), HashMap::new(), 1)
            .await;
        // No messages should be sent.
        assert!(rx.try_recv().is_err());
    }

    // ── route recovery on link negotiation ────────────────────────────────────

    #[tokio::test]
    async fn test_link_negotiation_server_triggers_route_recovery() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_server_conn(&processor);

        let link_id = uuid::Uuid::new_v4().to_string();
        let sub_name = ProtoName::from_strings(["org", "default", "recovered"]);

        // Pre-populate the recovery table as if a prior connection dropped.
        let mut local_subs = HashMap::new();
        local_subs.insert(sub_name.clone(), HashSet::from([99u64]));
        processor
            .remote_sync()
            .recovery
            .store(link_id.clone(), local_subs, HashSet::new());

        // Simulate the peer reconnecting with the same link_id.
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        processor
            .handle_link_negotiation(&payload, conn_id)
            .await
            .unwrap();

        // The subscription should have been restored in the routing table.
        let result = processor.forwarder().on_publish_msg_match(
            sub_name.name.unwrap(),
            u64::MAX,
            1,
            MatchFilter::ALL,
        );
        assert!(result.is_ok(), "recovered subscription should be routable");
        assert_eq!(result.unwrap(), vec![conn_id]);
    }

    #[tokio::test]
    async fn test_link_negotiation_server_recovery_restores_remote_subs() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);

        let link_id = uuid::Uuid::new_v4().to_string();
        let source = ProtoName::from_strings(["org", "default", "src"]);
        let dest = ProtoName::from_strings(["org", "default", "dst"]);

        let remote_sub =
            SubscriptionInfo::new(source.clone(), dest.clone(), "identity".into(), conn_id, 42);

        // Store recovery entry with remote subscriptions.
        processor.remote_sync().recovery.store(
            link_id.clone(),
            HashMap::new(),
            HashSet::from([remote_sub]),
        );

        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        processor
            .handle_link_negotiation(&payload, conn_id)
            .await
            .unwrap();

        // The restored subscribe is sent before the link negotiation reply.
        let sub_msg = rx.recv().await.unwrap().unwrap();
        assert!(matches!(sub_msg.get_type(), SubscribeType(_)));
        let reply = rx.recv().await.unwrap().unwrap();
        assert!(reply.is_link());
    }

    #[tokio::test]
    async fn test_link_negotiation_server_no_recovery_entry() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);

        let link_id = uuid::Uuid::new_v4().to_string();
        // No recovery entry stored — normal negotiation, no restoration.
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
            connection_type: LinkConnectionType::Remote.into(),
            node_id: String::new(),
            deployment_name: String::new(),
        };
        processor
            .handle_link_negotiation(&payload, conn_id)
            .await
            .unwrap();

        // Only the reply should have been sent.
        let reply = rx.try_recv().unwrap().unwrap();
        assert!(reply.is_link());
        assert!(rx.try_recv().is_err());
    }

    // ── restore_remote_subscriptions ──────────────────────────────────────────

    #[tokio::test]
    async fn test_restore_remote_subscriptions_with_tracking() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);

        let source = ProtoName::from_strings(["org", "default", "src"]);
        let dest = ProtoName::from_strings(["org", "default", "dst"]);
        let sub = SubscriptionInfo::new(source.clone(), dest.clone(), "id1".into(), conn_id, 7);
        let subs = HashSet::from([sub]);

        processor
            .restore_remote_subscriptions(&subs, conn_id, true)
            .await;

        // The subscribe message should have been sent.
        let msg = rx.recv().await.unwrap().unwrap();
        assert!(matches!(msg.get_type(), SubscribeType(_)));

        // With restore_tracking=true, the forwarded subscription should be tracked.
        let tracked = processor
            .remote_sync()
            .get_subscriptions_for_reconnect(conn_id);
        assert_eq!(tracked.len(), 1);
    }

    #[tokio::test]
    async fn test_restore_remote_subscriptions_without_tracking() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);

        let source = ProtoName::from_strings(["org", "default", "src"]);
        let dest = ProtoName::from_strings(["org", "default", "dst"]);
        let sub = SubscriptionInfo::new(source.clone(), dest.clone(), "id1".into(), conn_id, 7);
        let subs = HashSet::from([sub]);

        processor
            .restore_remote_subscriptions(&subs, conn_id, false)
            .await;

        // Message sent.
        let msg = rx.recv().await.unwrap().unwrap();
        assert!(matches!(msg.get_type(), SubscribeType(_)));

        // With restore_tracking=false, forwarded subscription table should NOT be updated.
        let tracked = processor
            .remote_sync()
            .get_subscriptions_for_reconnect(conn_id);
        assert!(tracked.is_empty());
    }
}

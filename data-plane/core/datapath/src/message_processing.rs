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
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use tonic::{Request, Response, Status};
use tracing::{Instrument, debug, error, info, warn};

#[cfg(feature = "otel_tracing")]
use crate::otel_tracing;

use crate::api::ProtoPublishType as PublishType;
use crate::api::ProtoSubscribeType as SubscribeType;
use crate::api::ProtoSubscriptionAckType as SubscriptionAckType;
use crate::api::ProtoUnsubscribeType as UnsubscribeType;
use crate::api::proto::dataplane::v1::Message;

use crate::api::proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
use crate::api::proto::dataplane::v1::data_plane_service_server::DataPlaneService;
use crate::api::{
    LinkNegotiationPayload, ProtoLink, ProtoLinkMessageType as LinkType, ProtoLinkType, ProtoName,
};
use crate::connection::{Channel, Connection};
use crate::errors::{DataPathError, MessageContext};
use crate::forwarder::Forwarder;
use crate::messages::utils::SlimHeaderFlags;
use crate::recovery::RecoveryTable;
use crate::tables::connection_table::ConnectionTable;
use crate::tables::remote_subscription_table::SubscriptionInfo;
use crate::tables::subscription_table::SubscriptionTableImpl;
use crate::tables::{ConnType, MatchFilter};
use crate::websocket;

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

    /// Pending route-recovery state for server-side connections (see [`RecoveryTable`]).
    recovery_table: RecoveryTable,

    /// Remote subscription ACK manager
    sub_ack_manager: crate::subscription_ack::RemoteSubAckManager,

    /// Service ID for tracing
    service_id: String,

    /// Default strict header MAC policy for server-accepted inter-node connections (see [`ServerConfig::require_header_mac`]).
    server_require_header_mac: bool,

    /// Timeout for link negotiation to complete.
    negotiation_timeout: std::time::Duration,
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

/// Describes how a connection enters [`MessageProcessor::process_stream`].
///
/// Local connections are pre-registered in the table; remote connections are
/// only inserted after the mandatory link negotiation completes.
enum StreamSetup {
    /// Connection already in the table (local connections).
    Registered(u64),
    /// Remote connection not yet in the table; will be inserted after negotiation.
    Pending {
        connection: Box<Connection>,
        existing_index: Option<u64>,
    },
}

impl MessageProcessor {
    pub fn new_with_service_id(service_id: String) -> Self {
        Self::new_with_options(service_id, None)
    }

    pub fn new_with_options(service_id: String, recovery_ttl: Option<std::time::Duration>) -> Self {
        Self::new_internal(
            service_id,
            recovery_ttl,
            false,
            std::time::Duration::from_secs(5),
        )
    }

    /// Create a processor with the server strict header MAC policy from `server_config`.
    pub fn new_with_server_config(
        service_id: String,
        server_config: &ServerConfig,
        recovery_ttl: Option<std::time::Duration>,
    ) -> Self {
        Self::new_internal(
            service_id,
            recovery_ttl,
            server_config.require_header_mac,
            std::time::Duration::from_secs(server_config.negotiation_timeout_secs),
        )
    }

    fn new_internal(
        service_id: String,
        recovery_ttl: Option<std::time::Duration>,
        server_require_header_mac: bool,
        negotiation_timeout: std::time::Duration,
    ) -> Self {
        let (signal, watch) = drain::channel();
        let recovery_table = match recovery_ttl {
            Some(ttl) => RecoveryTable::new(ttl),
            None => RecoveryTable::default(),
        };
        let internal = MessageProcessorInternal {
            forwarder: Forwarder::new(),
            drain_signal: RwLock::new(Some(signal)),
            drain_watch: RwLock::new(Some(watch)),
            tx_control_plane: RwLock::new(None),
            recovery_table,
            sub_ack_manager: crate::subscription_ack::RemoteSubAckManager::new(),
            service_id,
            server_require_header_mac,
            negotiation_timeout,
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

        if let Err(err) = self.process_stream(
            streams.inbound,
            StreamSetup::Pending {
                connection: Box::new(connection),
                existing_index: None,
            },
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

    pub fn forwarder(&self) -> &Forwarder<Connection> {
        &self.internal.forwarder
    }

    pub(crate) fn recovery_table(&self) -> &RecoveryTable {
        &self.internal.recovery_table
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
        if !matches!(conn.category(), ConnType::Remote) {
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

    pub(crate) fn remove_sub_ack(&self, subscription_id: u64) {
        self.internal.sub_ack_manager.remove(subscription_id);
    }

    fn get_drain_watch(&self) -> Result<drain::Watch, DataPathError> {
        self.internal
            .drain_watch
            .read()
            .clone()
            .ok_or(DataPathError::AlreadyClosedError)
    }

    /// Re-send `remote_subs` as subscribe messages to `conn_index`.
    ///
    /// When `restore_tracking` is `true` (server-side recovery), also re-registers each
    /// subscription in the local forwarded-subscription table.  This is necessary because
    /// [`Forwarder::on_connection_drop`] already wiped that state.
    ///
    /// When `restore_tracking` is `false` (client-side reconnect), the forwarded-subscription
    /// table was never cleaned up (reconnect reuses the same slot), so no re-registration is
    /// needed and double-counting must be avoided.
    async fn restore_remote_subscriptions(
        &self,
        remote_subs: &HashSet<SubscriptionInfo>,
        conn_index: u64,
        restore_tracking: bool,
    ) {
        for r in remote_subs {
            let sub_msg = Message::builder()
                .source(r.source().clone())
                .destination(r.name().clone())
                .identity(r.source_identity())
                .build_subscribe()
                .unwrap();
            if let Err(e) = self.send_msg(sub_msg, conn_index).await {
                error!(
                    error = %e.chain(), %conn_index,
                    "error restoring subscription on remote node",
                );
            } else if restore_tracking {
                self.forwarder().on_forwarded_subscription(
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

                let (handle, conn_index_rx) = self.register_remote_connection(
                    stream.into_inner(),
                    Channel::Client(tx),
                    &client_config,
                    local,
                    remote,
                    existing_conn_index,
                    cancellation_token,
                    Some(link_id.clone()),
                )?;

                let conn_index = conn_index_rx.await.map_err(|_| {
                    DataPathError::NegotiationError(
                        "negotiation task terminated unexpectedly".to_string(),
                    )
                })??;

                Ok((handle, conn_index))
            }
            TransportChannel::Websocket(ws_channel) => {
                let websocket = ws_channel
                    .take_websocket()
                    .expect("websocket channel already consumed");
                let streams =
                    websocket::spawn_transport_tasks(websocket, cancellation_token.clone());

                let (handle, conn_index_rx) = self.register_remote_connection(
                    streams.inbound,
                    Channel::Client(streams.outbound),
                    &client_config,
                    local.or(ws_channel.local_addr()),
                    remote.or(ws_channel.remote_addr()),
                    existing_conn_index,
                    cancellation_token,
                    Some(link_id.clone()),
                )?;

                let conn_index = conn_index_rx.await.map_err(|_| {
                    DataPathError::NegotiationError(
                        "negotiation task terminated unexpectedly".to_string(),
                    )
                })??;

                Ok((handle, conn_index))
            }
        }
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
    ) -> Result<
        (
            JoinHandle<()>,
            oneshot::Receiver<Result<u64, DataPathError>>,
        ),
        DataPathError,
    >
    where
        S: Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
    {
        let mut connection = Connection::new(ConnType::Remote, outbound)
            .with_local_addr(local)
            .with_remote_addr(remote)
            .with_config_data(Some(client_config.clone()))
            .with_require_header_mac(client_config.require_header_mac)
            .with_cancellation_token(Some(cancellation_token.clone()));
        if let Some(link_id) = link_id {
            connection = connection.with_link_id(link_id);
        }

        debug!(
            remote = ?connection.remote_addr(),
            local = ?connection.local_addr(),
            "new connection initiated locally",
        );

        let (handle, conn_index_rx) = self.process_stream(
            inbound,
            StreamSetup::Pending {
                connection: Box::new(connection),
                existing_index: existing_conn_index,
            },
            Some(client_config.clone()),
            cancellation_token,
            ConnType::Remote,
            false,
        )?;

        Ok((handle, conn_index_rx))
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
            StreamSetup::Registered(conn_id),
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
                    && matches!(conn.category(), ConnType::Remote)
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
                    && matches!(conn.category(), ConnType::Remote)
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
                    && matches!(conn.category(), ConnType::Local)
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

    /// Handle an inbound link negotiation message arriving in the main loop.
    ///
    /// Since negotiation is mandatory and completes before the connection is
    /// inserted into the table, any link negotiation message arriving here is
    /// either a duplicate or a protocol error — we log and ignore it.
    async fn handle_link_negotiation(
        &self,
        payload: &LinkNegotiationPayload,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        debug!(
            %in_connection,
            link_id = %payload.link_id,
            is_reply = payload.is_reply,
            "ignoring link negotiation message on already-negotiated connection",
        );
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

    async fn process_subscription_update_and_forward(
        &self,
        msg: Message,
        conn: u64,
        forward: Option<u64>,
        add: bool,
        subscription_id: u64,
    ) -> Result<(), DataPathError> {
        let dst = msg.get_dst();

        // As connection is deleted only after processing, at this point it must exist.
        let connection = if let Some(c) = self.forwarder().get_connection(conn) {
            c
        } else {
            return Err(DataPathError::MessageProcessingError {
                source: Box::new(DataPathError::ConnectionNotFound(conn)),
                msg: Box::new(msg),
            });
        };

        debug!(
            %conn,
            %dst,
            is_local = connection.is_local_connection(),
            "processing {}subscription",
            if add { "" } else { "un" }
        );

        self.forwarder().on_subscription_msg(
            dst.clone(),
            conn,
            connection.category(),
            add,
            subscription_id,
        )?;

        match forward {
            None => Ok(()),
            Some(out_conn) => {
                debug!(
                    %out_conn,
                    "forwarding {}subscription to connection",
                    if add { "" } else { "un" }
                );

                let source = msg.get_source();
                let identity = msg.get_identity();

                self.send_msg(msg, out_conn).await.map(|_| {
                    self.forwarder().on_forwarded_subscription(
                        source,
                        dst,
                        identity,
                        out_conn,
                        add,
                        subscription_id,
                    );
                })
            }
        }
    }

    // Use a single function to process subscription and unsubscription packets.
    // The flag add = true is used to add a new subscription while add = false
    // is used to remove existing state
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

        // If forwarding to a remote node, use the remote ack path:
        // update local state now, then asynchronously forward and wait for the remote ACK
        // before notifying the upstream requester.
        let use_remote_ack = forward.is_some();

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

        debug!(use_remote_ack, dst = %msg.get_dst(), forward_to = forward, "subscription: ack path decision");

        let sub_id = subscription_id.unwrap_or(0);

        // Always register subscription as at this point we might not have received the link negotiaiion
        // yet, so the other side might reply just after
        let rx = self.internal.sub_ack_manager.register(sub_id);

        // Update local state and forward the subscription.
        let result = self
            .process_subscription_update_and_forward(msg.clone(), in_conn, forward, add, sub_id)
            .await;

        // Remote-ack path: on success, spawn a retry loop that waits for the
        // downstream ACK (the initial send was already done above) and retries
        // on timeout.
        if use_remote_ack && result.is_ok() {
            let out_conn = forward.unwrap();

            tokio::spawn(crate::subscription_ack::retry_loop(
                self.clone(),
                sub_id,
                msg,
                out_conn,
                in_connection,
                subscription_id,
                rx,
            ));

            return Ok(());
        }

        // Default path (or remote-ack error fallback): ACK the requester immediately.
        if let Some(id) = subscription_id {
            debug!(%in_connection, ok = result.is_ok(), "sending immediate subscription ack");
            self.send_subscription_ack(in_connection, id, &result).await;
        }

        result
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
                    ConnType::Peer => MatchFilter::EXCLUDE_PEER,
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
                    Err(DataPathError::RemoteSubscriptionAckError(ack.error.clone()))
                };
                self.internal
                    .sub_ack_manager
                    .resolve(ack.subscription_id, result);
                Ok(())
            }
            None => unreachable!(
                "message type not set; validate() must be called before process_message"
            ),
        }
    }

    pub(crate) async fn handle_new_message(
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

        // These are the subscriptions that we forwarded to the remote SLIM on
        // this connection. It is necessary to restore them to keep receive the messages
        // The connections on the local subscription table (created using the set_route command)
        // are still there and will be removed only if the reconnection process fails.
        let remote_subscriptions = self
            .forwarder()
            .get_subscriptions_forwarded_on_connection(conn_index);

        tokio::select! {
            _ = cancellation_token.cancelled() => {
                debug!("cancellation token signaled, stopping reconnection process");
                false
            }
            res = self.try_to_connect(client_conf, None, None, Some(conn_index)) => {
                match res {
                    Ok(_) => {
                        info!("connection re-established successfully");
                        // Restore subscriptions on the remote node.
                        // restore_tracking = false: the forwarded-subscription table was not
                        // cleaned up (same conn_index is reused), so we only replay the
                        // messages without re-registering local tracking state.
                        self.restore_remote_subscriptions(
                            &remote_subscriptions,
                            conn_index,
                            false,
                        )
                        .await;
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
        setup: StreamSetup,
        client_config: Option<ClientConfig>,
        cancellation_token: CancellationToken,
        category: ConnType,
        from_control_plane: bool,
    ) -> Result<
        (
            JoinHandle<()>,
            oneshot::Receiver<Result<u64, DataPathError>>,
        ),
        DataPathError,
    > {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();
        let client_conf_clone = client_config.clone();
        let tx_cp: Option<Sender<Result<Message, Status>>> = self.get_tx_control_plane();
        let watch = self.get_drain_watch()?;
        let is_local = category.is_local();

        let (conn_index_tx, conn_index_rx) = oneshot::channel();

        // For registered (local) connections, we know conn_index immediately.
        let (pre_conn_index, pending_connection, require_header_mac): (
            Option<u64>,
            Option<(Connection, Option<u64>)>,
            bool,
        ) = match setup {
            StreamSetup::Registered(idx) => {
                let rhm = self
                    .forwarder()
                    .get_connection(idx)
                    .map(|c| c.require_header_mac())
                    .unwrap_or(false);
                (Some(idx), None, rhm)
            }
            StreamSetup::Pending {
                connection,
                existing_index,
            } => {
                let rhm = connection.require_header_mac();
                (None, Some((*connection, existing_index)), rhm)
            }
        };

        let span = tracing::info_span!(
            "process_stream",
            service_id = %self.internal.service_id,
            conn_index = pre_conn_index.unwrap_or(0),
            is_local,
        );

        let handle = tokio::spawn(async move {
            let mut try_to_reconnect = true;

            // Resolve the conn_index: either we already have it (local), or we
            // must perform negotiation and then insert into the table (remote).
            let conn_index = if let Some(idx) = pre_conn_index {
                // Local: already registered.
                let _ = conn_index_tx.send(Ok(idx));
                idx
            } else if let Some((mut connection, existing_index)) = pending_connection {
                // Remote: perform mandatory link negotiation before inserting.
                let timeout = self_clone.internal.negotiation_timeout;
                let negotiation_result = tokio::select! {
                    result = tokio::time::timeout(
                        timeout,
                        crate::negotiation::run_negotiation(
                            &mut connection,
                            &mut stream,
                            self_clone.recovery_table(),
                        ),
                    ) => match result {
                        Ok(r) => r,
                        Err(_) => Err(DataPathError::NegotiationError(
                            "timed out waiting for link negotiation".to_string(),
                        )),
                    },
                    _ = watch.clone().signaled() => {
                        info!("shutting down during link negotiation");
                        let _ = conn_index_tx.send(Err(DataPathError::ShuttingDownError));
                        return;
                    }
                    _ = token_clone.cancelled() => {
                        info!("connection cancelled during link negotiation");
                        let _ = conn_index_tx.send(Err(DataPathError::ShuttingDownError));
                        return;
                    }
                };

                match negotiation_result {
                    Err(e) => {
                        error!(error = %e.chain(), "link negotiation failed, closing connection");
                        let _ = conn_index_tx.send(Err(e));
                        info!(telemetry = true, counter.num_active_connections = -1);
                        return;
                    }
                    Ok(result) => {
                        // Negotiation succeeded — insert the fully-negotiated connection.
                        let idx = match self_clone
                            .forwarder()
                            .on_connection_established(connection, existing_index)
                        {
                            Some(idx) => idx,
                            None => {
                                let _ = conn_index_tx.send(Err(DataPathError::ConnectionTableAddError));
                                info!(telemetry = true, counter.num_active_connections = -1);
                                return;
                            }
                        };

                        debug!(%idx, "connection registered after link negotiation");

                        // Apply route recovery (server side) now that we have a conn_index.
                        if let Some(entry) = result.recovery_entry {
                            info!(%idx, "recovering routes for reconnected peer");
                            for (name, sub_ids) in &entry.local_subs {
                                for &subscription_id in sub_ids {
                                    if let Err(e) = self_clone.forwarder().on_subscription_msg(
                                        name.clone(),
                                        idx,
                                        ConnType::Remote,
                                        true,
                                        subscription_id,
                                    ) {
                                        error!(
                                            error = %e.chain(), %idx,
                                            "error re-adding local subscription during recovery",
                                        );
                                    }
                                }
                            }
                            self_clone
                                .restore_remote_subscriptions(&entry.remote_subs, idx, true)
                                .await;
                        }

                        let _ = conn_index_tx.send(Ok(idx));
                        idx
                    }
                }
            } else {
                unreachable!("process_stream called with neither registered nor pending setup");
            };

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

                                        if let Err(e) = self_clone.handle_new_message(conn_index, category, msg).await {
                                            // Checking if NegotiationError occurred
                                            if matches!(e, DataPathError::NegotiationError(_)) {
                                                error!(%conn_index, "fatal link negotiation error, closing connection");
                                                break;
                                            }
                                            debug!(%conn_index, error = %e.chain(), "error processing incoming message");
                                            // If the message is coming from a local app, notify it
                                            if is_local {
                                                // try to forward error to the local app
                                                self_clone.send_error_to_local_app(conn_index, e).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if let Some(io_err) = MessageProcessor::match_for_io_error(&e) {
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
                let (local_subs, remote_subs) = self_clone
                    .forwarder()
                    .on_connection_drop(conn_index, category);

                let recovery_enabled =
                    !self_clone.internal.recovery_table.ttl().is_zero();

                if let Some(lid) = link_id.filter(|_| recovery_enabled) {
                    // Server connection with a known link_id: preserve routing state and
                    // suppress the control-plane notification for the duration of the TTL
                    // to give the peer a chance to reconnect.
                    info!(
                        %conn_index, %lid,
                        "connection lost, storing recovery state (TTL: {:?})",
                        self_clone.internal.recovery_table.ttl(),
                    );
                    self_clone
                        .internal
                        .recovery_table
                        .store(lid.clone(), local_subs, remote_subs);

                    // Spawn a TTL task that fires the CP notification if recovery never happens.
                    if let Ok(drain) = self_clone.get_drain_watch() {
                        let tx_cp_ttl = tx_cp;
                        let mp = self_clone.clone();
                        self_clone.internal.recovery_table.spawn_ttl_task(
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

        Ok((handle, conn_index_rx))
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

        self.process_stream(
            stream,
            StreamSetup::Pending {
                connection: Box::new(connection),
                existing_index: None,
            },
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
    use std::time::Duration;

    use super::*;
    use crate::api::{ProtoMessage, ProtoName, ProtoSubscriptionAck};
    use crate::header_mac::HeaderMacSession;
    use crate::tables::remote_subscription_table::SubscriptionInfo;
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

    /// After negotiation completes and the connection is inserted into the table,
    /// any further link negotiation messages arriving in the main loop are simply
    /// logged and ignored (the handler is a no-op). This test verifies that.
    #[tokio::test]
    async fn test_handle_link_negotiation_post_negotiation_is_noop() {
        let processor = MessageProcessor::new();
        let payload = LinkNegotiationPayload {
            link_id: uuid::Uuid::new_v4().to_string(),
            slim_version: "1.0.0".into(),
            is_reply: false,
            link_ecdh_public_key: vec![],
        };
        // Unknown connection: handler returns Ok without panic.
        assert!(
            processor
                .handle_link_negotiation(&payload, u64::MAX)
                .await
                .is_ok()
        );
        // Known connection: handler still returns Ok (noop).
        let (conn_id, _rx) = make_negotiated_server_conn(&processor, "1.2.0");
        assert!(
            processor
                .handle_link_negotiation(&payload, conn_id)
                .await
                .is_ok()
        );
    }

    // ── process_subscription: remote ack path ─────────────────────────────────

    /// Helper: create a server connection that is already negotiated with given version and
    /// a test HMAC session, suitable for testing routing and MAC verification.
    fn make_negotiated_server_conn(
        processor: &MessageProcessor,
        version: &str,
    ) -> (u64, tokio::sync::mpsc::Receiver<Result<Message, Status>>) {
        let (tx, rx) = mpsc::channel(16);
        let conn = Connection::new(ConnType::Remote, Channel::Server(tx))
            .with_require_header_mac(processor.internal.server_require_header_mac)
            .with_negotiation(&uuid::Uuid::new_v4().to_string(), version)
            .with_header_hmac(HeaderMacSession::new(b"01234567890123456789012345678901").unwrap());
        let conn_id = processor
            .forwarder()
            .on_connection_established(conn, None)
            .unwrap();
        (conn_id, rx)
    }

    #[tokio::test]
    async fn test_negotiation_timeout_configurable() {
        let server_config = ServerConfig {
            endpoint: "localhost:12345".to_string(),
            negotiation_timeout_secs: 1, // 1 second timeout
            ..Default::default()
        };
        let processor = MessageProcessor::new_with_server_config(
            "test_service".to_string(),
            &server_config,
            None,
        );

        assert_eq!(
            processor.internal.negotiation_timeout,
            std::time::Duration::from_secs(1)
        );
    }

    #[test]
    fn verify_remote_header_mac_strict_rejects_publish_without_mac_session() {
        let processor = MessageProcessor::new();
        // Create a negotiated connection WITHOUT header HMAC installed.
        let (tx, _rx) = mpsc::channel(16);
        let conn = Connection::new(ConnType::Remote, Channel::Server(tx))
            .with_require_header_mac(true)
            .with_negotiation(&uuid::Uuid::new_v4().to_string(), "1.2.0");
        let remote_conn = processor
            .forwarder()
            .on_connection_established(conn, None)
            .unwrap();
        let c = processor.forwarder().get_connection(remote_conn).unwrap();
        assert!(c.header_hmac().is_none());

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
        let (remote_conn, _rx) = make_negotiated_server_conn(&processor, "1.2.0");
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
        let (remote_conn, _rx) = make_negotiated_server_conn(&processor, "1.2.0");
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
        let (conn_id, mut rx) = make_negotiated_server_conn(&processor, "1.2.0");

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

        let (remote_conn, mut rx_remote) = make_negotiated_server_conn(&processor, "1.2.0");

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
        processor.internal.sub_ack_manager.resolve(
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

        let (remote_conn, mut rx_remote) = make_negotiated_server_conn(&processor, "1.2.0");

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
        processor.internal.sub_ack_manager.resolve(
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

    // ── retry_loop tests ──────────────────────────────────────────────────────

    fn make_test_subscribe(sub_id: u64) -> Message {
        let source = ProtoName::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "dst"]).with_id(2);
        Message::builder()
            .source(source)
            .destination(destination)
            .subscription_id(sub_id)
            .build_subscribe()
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_ack_received_before_timeout() {
        // ACK arrives within the first wait window → no retry send.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, mut rx_remote) = make_negotiated_server_conn(&processor, "1.2.0");

        let sub_id: u64 = 1000;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            local_conn,
            Some(sub_id),
            rx,
        ));

        // Resolve immediately — the loop should receive it before the timeout.
        processor.internal.sub_ack_manager.resolve(sub_id, Ok(()));

        handle.await.unwrap();

        // No retry sends should have been made.
        assert!(
            rx_remote.try_recv().is_err(),
            "no retry send expected when ack arrives before timeout"
        );

        // Upstream ack must have been sent.
        let ack = rx_local
            .try_recv()
            .expect("upstream ack should have been sent")
            .unwrap();
        assert!(ack.get_subscription_ack().success);
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_timeout_then_retry_send_then_ack() {
        // First wait times out → retry send → ACK arrives on second wait.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, mut rx_remote) = make_negotiated_server_conn(&processor, "1.2.0");

        let sub_id: u64 = 1001;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            local_conn,
            Some(sub_id),
            rx,
        ));

        // Let the first timeout elapse → triggers a retry send.
        tokio::time::sleep(crate::subscription_ack::TIMEOUT + Duration::from_millis(100)).await;

        // A retry message should have been sent.
        let retried = rx_remote
            .try_recv()
            .expect("retry send expected after first timeout")
            .unwrap();
        assert!(retried.get_subscription_id().is_some());

        // Now resolve so the second wait succeeds.
        processor.internal.sub_ack_manager.resolve(sub_id, Ok(()));

        handle.await.unwrap();

        // Upstream success ack.
        let ack = rx_local
            .try_recv()
            .expect("upstream ack should have been sent")
            .unwrap();
        assert!(ack.get_subscription_ack().success);
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_retry_send_fails() {
        // Timeout → retry send fails because the connection is gone → loop
        // exits with the send error, upstream receives a failure ack.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, _rx_remote) = make_server_conn(&processor);

        let sub_id: u64 = 1002;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        // Drop the remote connection so send_msg fails on retry.
        processor.connection_table().remove(remote_conn);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            local_conn,
            Some(sub_id),
            rx,
        ));

        // Let the first timeout elapse → triggers a retry send which fails.
        tokio::time::sleep(crate::subscription_ack::TIMEOUT + Duration::from_millis(100)).await;

        handle.await.unwrap();

        // Upstream failure ack.
        let ack = rx_local
            .try_recv()
            .expect("upstream ack should have been sent")
            .unwrap();
        assert!(!ack.get_subscription_ack().success);
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_all_retries_exhausted() {
        // No ACK ever arrives → all waits time out → final_result is timeout error.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, mut rx_remote) = make_negotiated_server_conn(&processor, "1.2.0");

        let sub_id: u64 = 1003;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            local_conn,
            Some(sub_id),
            rx,
        ));

        // Advance time past all retry windows: (MAX_RETRIES + 1) timeouts.
        for _ in 0..=crate::subscription_ack::MAX_RETRIES {
            tokio::time::sleep(crate::subscription_ack::TIMEOUT + Duration::from_millis(100)).await;
        }

        handle.await.unwrap();

        // Should have exactly MAX_RETRIES retry sends (attempts 0..MAX_RETRIES-1
        // trigger resends; the last attempt only waits).
        let mut retry_count = 0;
        while rx_remote.try_recv().is_ok() {
            retry_count += 1;
        }
        assert_eq!(
            retry_count,
            crate::subscription_ack::MAX_RETRIES as usize,
            "expected {} retry sends",
            crate::subscription_ack::MAX_RETRIES,
        );

        // Upstream ack must indicate failure (timeout).
        let ack = rx_local
            .try_recv()
            .expect("upstream ack should have been sent")
            .unwrap();
        let ack_inner = ack.get_subscription_ack();
        assert!(
            !ack_inner.success,
            "ack must indicate failure after exhausting retries"
        );
        assert!(!ack_inner.error.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_no_upstream_subscription_id() {
        // When upstream_subscription_id is None, no upstream ack is sent.
        let processor = MessageProcessor::new();
        let (_local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, _rx_remote) = make_server_conn(&processor);

        let sub_id: u64 = 1004;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            0, // in_connection — irrelevant since upstream_subscription_id is None
            None,
            rx,
        ));

        // Resolve immediately.
        processor.internal.sub_ack_manager.resolve(sub_id, Ok(()));

        handle.await.unwrap();

        // No upstream ack should be sent.
        assert!(
            rx_local.try_recv().is_err(),
            "no upstream ack when upstream_subscription_id is None"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_retry_loop_sender_dropped() {
        // If the oneshot sender is dropped (e.g. processor shutdown), the loop
        // must exit promptly without panicking.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");
        let (remote_conn, _rx_remote) = make_server_conn(&processor);

        let sub_id: u64 = 1005;
        let msg = make_test_subscribe(sub_id);
        let rx = processor.internal.sub_ack_manager.register(sub_id);

        // Drop the sender by removing the pending entry.
        processor.internal.sub_ack_manager.remove(sub_id);

        let proc_clone = processor.clone();
        let handle = tokio::spawn(crate::subscription_ack::retry_loop(
            proc_clone,
            sub_id,
            msg,
            remote_conn,
            local_conn,
            Some(sub_id),
            rx,
        ));

        handle.await.unwrap();

        // Upstream failure ack (timeout error since we never got a result).
        let ack = rx_local
            .try_recv()
            .expect("upstream ack should have been sent")
            .unwrap();
        assert!(!ack.get_subscription_ack().success);
    }

    // ── new_with_options ──────────────────────────────────────────────────────

    #[test]
    fn test_new_with_options_custom_ttl() {
        let processor =
            MessageProcessor::new_with_options("svc".into(), Some(Duration::from_secs(5)));
        assert_eq!(
            processor.internal.recovery_table.ttl(),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn test_new_with_options_none_uses_default() {
        let processor = MessageProcessor::new_with_options("svc".into(), None);
        assert_eq!(
            processor.internal.recovery_table.ttl(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_new_with_options_zero_ttl() {
        let processor = MessageProcessor::new_with_options("svc".into(), Some(Duration::ZERO));
        assert!(processor.internal.recovery_table.ttl().is_zero());
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

    // ── restore_remote_subscriptions ──────────────────────────────────────────

    #[tokio::test]
    async fn test_restore_remote_subscriptions_with_tracking() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_negotiated_server_conn(&processor, "1.2.0");

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
            .forwarder()
            .get_subscriptions_forwarded_on_connection(conn_id);
        assert_eq!(tracked.len(), 1);
    }

    #[tokio::test]
    async fn test_restore_remote_subscriptions_without_tracking() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_negotiated_server_conn(&processor, "1.2.0");

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
            .forwarder()
            .get_subscriptions_forwarded_on_connection(conn_id);
        assert!(tracked.is_empty());
    }
}

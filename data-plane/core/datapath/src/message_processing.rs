// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use crate::api::DataPlaneServiceServer;
use display_error_chain::ErrorChainExt;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use parking_lot::RwLock;
use slim_config::component::configuration::Configuration;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_tracing::otel_propagation_enabled;
use slim_tracing::utils::INSTANCE_ID;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use tonic::{Request, Response, Status};
use tracing::{Instrument, Span, debug, error, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::api::ProtoMessage;
use crate::api::ProtoPublishType as PublishType;
use crate::api::ProtoSubscribeType as SubscribeType;
use crate::api::ProtoSubscriptionAckType as SubscriptionAckType;
use crate::api::ProtoUnsubscribeType as UnsubscribeType;
use crate::api::proto::dataplane::v1::Message;
use crate::api::{
    LinkNegotiationPayload, ProtoLink, ProtoLinkMessageType as LinkType, ProtoLinkType,
};
use semver;

use crate::api::proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
use crate::api::proto::dataplane::v1::data_plane_service_server::DataPlaneService;
use crate::connection::{Channel, Connection, Type as ConnectionType};
use crate::errors::{DataPathError, MessageContext};
use crate::forwarder::Forwarder;
use crate::messages::Name;
use crate::messages::utils::SlimHeaderFlags;
use crate::recovery::RecoveryTable;
use crate::tables::connection_table::ConnectionTable;
use crate::tables::remote_subscription_table::SubscriptionInfo;
use crate::tables::subscription_table::SubscriptionTableImpl;

// Implementation based on: https://docs.rs/opentelemetry-tonic/latest/src/opentelemetry_tonic/lib.rs.html#1-134
struct MetadataExtractor<'a>(&'a std::collections::HashMap<String, String>);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|s| s.as_str()).collect()
    }
}

struct MetadataInjector<'a>(&'a mut std::collections::HashMap<String, String>);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

// Helper function to extract the parent OpenTelemetry context from metadata
fn extract_parent_context(msg: &Message) -> Option<opentelemetry::Context> {
    let extractor = MetadataExtractor(&msg.metadata);
    let parent_context =
        opentelemetry::global::get_text_map_propagator(|propagator| propagator.extract(&extractor));

    if parent_context.span().span_context().is_valid() {
        Some(parent_context)
    } else {
        None
    }
}

// Helper function to inject the current OpenTelemetry context into metadata
fn inject_current_context(msg: &mut Message) {
    let cx = tracing::Span::current().context();
    let mut injector = MetadataInjector(&mut msg.metadata);
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut injector)
    });
}

/// Per-connection span vs one span covering all fan-out subscribers.
enum ProcessSpanTarget {
    Connection(u64),
    Fanout { subscribers: u32 },
}

#[inline]
fn message_otel_active(msg: &Message) -> bool {
    otel_propagation_enabled() && !msg.is_link() && !msg.is_subscription_ack()
}

fn apply_publish_span_attributes(span: &Span, msg: &Message) {
    if let PublishType(_) = msg.get_type() {
        span.set_attribute("session_type", msg.get_session_message_type().as_str_name());
        span.set_attribute(
            "session_id",
            msg.get_session_header().get_session_id() as i64,
        );
        span.set_attribute(
            "message_id",
            msg.get_session_header().get_message_id() as i64,
        );
    }
}

fn attach_trace_to_message(msg: &mut Message, span: Span, parent: Option<opentelemetry::Context>) {
    if let Some(ctx) = parent
        && let Err(e) = span.set_parent(ctx)
    {
        error!(error = %e.chain(), "error setting parent context");
    }
    let _guard = span.enter();
    inject_current_context(msg);
}

fn local_version() -> &'static str {
    slim_version::version()
}

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
        };
        Self {
            internal: Arc::new(internal),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    fn create_process_span(
        &self,
        function: &str,
        msg: &Message,
        target: ProcessSpanTarget,
    ) -> Span {
        let span = match target {
            ProcessSpanTarget::Connection(connection_id) => tracing::span!(
                tracing::Level::INFO,
                "slim_process_message",
                function = function,
                service_id = %self.internal.service_id,
                source = %msg.get_source(),
                destination = %msg.get_dst(),
                instance_id = %INSTANCE_ID.as_str(),
                connection_id = connection_id,
                message_type = %msg.get_type(),
                telemetry = true
            ),
            ProcessSpanTarget::Fanout { subscribers } => tracing::span!(
                tracing::Level::INFO,
                "slim_process_message",
                function = function,
                service_id = %self.internal.service_id,
                source = %msg.get_source(),
                destination = %msg.get_dst(),
                instance_id = %INSTANCE_ID.as_str(),
                fanout_subscribers = subscribers,
                connection_id = 0u64,
                message_type = %msg.get_type(),
                telemetry = true
            ),
        };
        apply_publish_span_attributes(&span, msg);
        span
    }

    /// Run a data plane gRPC server using this message processor's drain watch.
    /// Returns a cancellation token that can be used to stop the server task.
    pub async fn run_server(
        &self,
        config: &ServerConfig,
    ) -> Result<CancellationToken, DataPathError> {
        debug!(%config, "starting dataplane server");
        let watch = self.get_drain_watch()?;
        // Wrap self in an Arc since the server builder expects an Arc<MessageProcessor>
        let svc = Arc::new(self.clone());
        let res = config
            .run_server(&[DataPlaneServiceServer::from_arc(svc)], watch)
            .await?;

        Ok(res)
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

    fn forwarder(&self) -> &Forwarder<Connection> {
        &self.internal.forwarder
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

        let mut client = DataPlaneServiceClient::new(channel);
        let (tx, rx) = mpsc::channel(128);

        let stream = client
            .open_channel(Request::new(ReceiverStream::new(rx)))
            .await?;

        // The link_id is embedded in the Connection before it enters the table so that
        // it is never None during the connection's lifetime — including reconnects where
        // the old Connection object is replaced by a fresh one.
        let link_id = client_config.link_id.clone();

        let cancellation_token = CancellationToken::new();
        let connection = Connection::new(ConnectionType::Remote, Channel::Client(tx))
            .with_local_addr(local)
            .with_remote_addr(remote)
            .with_config_data(Some(client_config.clone()))
            .with_cancellation_token(Some(cancellation_token.clone()))
            .with_link_id(link_id.clone());

        debug!(
            remote = ?connection.remote_addr(),
            local = ?connection.local_addr(),
            "new connection initiated locally",
        );

        // insert connection into connection table
        let conn_index = self
            .forwarder()
            .on_connection_established(connection, existing_conn_index)
            .ok_or(DataPathError::ConnectionTableAddError)?;

        debug!(
            %conn_index,
            is_local = false,
            "new connection index",
        );

        // Start loop to process messages
        let handle = self.process_stream(
            stream.into_inner(),
            conn_index,
            Some(client_config.clone()),
            cancellation_token,
            false,
            false,
        )?;

        // Send the link negotiation message to the remote peer.
        // Old SLIM instances that do not understand this message will silently drop it.
        let negotiation_msg =
            ProtoMessage::builder().build_link_negotiation(&link_id, local_version(), false);
        if let Err(e) = self.send_msg(negotiation_msg, conn_index).await {
            debug!(
                %conn_index,
                error = %e.chain(),
                "failed to send link negotiation (remote may be an older SLIM instance)",
            );
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
        let connection = Connection::new(ConnectionType::Local, Channel::Server(tx2))
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
            true,
            from_control_plane,
        )?;

        // return the conn_id and  handles to be used to send and receive messages
        Ok((conn_id, tx1, rx2))
    }

    pub async fn send_msg(&self, msg: Message, out_conn: u64) -> Result<(), DataPathError> {
        self.send_msg_inner(msg, out_conn, false).await
    }

    /// Sends a message on `out_conn`. When `outbound_otel_already_prepared` is true, trace
    /// context was applied once for a fanout (see [`Self::match_and_forward_msg`]).
    async fn send_msg_inner(
        &self,
        mut msg: Message,
        out_conn: u64,
        outbound_otel_already_prepared: bool,
    ) -> Result<(), DataPathError> {
        let connection = self.forwarder().get_connection(out_conn);
        match connection {
            Some(conn) => {
                // Link and SubscriptionAck messages have no SLIM header: skip header
                // manipulation and telemetry span creation.
                if !msg.is_link() && !msg.is_subscription_ack() {
                    // reset header fields
                    msg.clear_slim_header();

                    if message_otel_active(&msg) && !outbound_otel_already_prepared {
                        let parent = extract_parent_context(&msg);
                        let span = self.create_process_span(
                            "send_message",
                            &msg,
                            ProcessSpanTarget::Connection(out_conn),
                        );
                        attach_trace_to_message(&mut msg, span, parent);
                    }
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
        mut msg: Message,
        name: Name,
        in_connection: u64,
        fanout: u32,
    ) -> Result<(), DataPathError> {
        debug!(
            %name,
            %fanout,
            "match and forward message"
        );

        // if the message already contains an output connection, use that one
        // without performing any match in the subscription table
        if let Some(val) = msg.get_forward_to() {
            debug!(conn = %val, "forwarding message to connection");
            return self.send_msg(msg, val).await;
        }

        match self
            .forwarder()
            .on_publish_msg_match(name, in_connection, fanout)
        {
            Ok(out_vec) => {
                let len = out_vec.len();
                // Single destination: preserve per-connection span attributes.
                if len == 1 {
                    return self.send_msg(msg, out_vec[0]).await;
                }

                // Fan-out: one extract + span + inject for all copies (same propagated context).
                let fanout_otel_prepped = message_otel_active(&msg);
                if fanout_otel_prepped {
                    msg.clear_slim_header();
                    let parent = extract_parent_context(&msg);
                    let span = self.create_process_span(
                        "send_message",
                        &msg,
                        ProcessSpanTarget::Fanout {
                            subscribers: len as u32,
                        },
                    );
                    attach_trace_to_message(&mut msg, span, parent);
                }

                let mut i = 0usize;
                while i < len - 1 {
                    self.send_msg_inner(msg.clone(), out_vec[i], fanout_otel_prepped)
                        .await?;
                    i += 1;
                }
                self.send_msg_inner(msg, out_vec[i], fanout_otel_prepped)
                    .await?;
                Ok(())
            }
            Err(e) => Err(DataPathError::MessageProcessingError {
                source: Box::new(e),
                msg: Box::new(msg),
            }),
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
        is_local: bool,
    ) -> Result<(), DataPathError> {
        if is_local {
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
    /// On request (`is_reply == false`): validate the client-provided `link_id` as UUID v4,
    /// atomically store both fields under one lock, then echo back a reply.
    ///
    /// On reply (`is_reply == true`): verify the echoed `link_id` matches what we sent, then
    /// atomically store the remote version.  No further reply is sent, preventing echo loops.
    ///
    /// Both methods hold a single write lock for validation and mutation, eliminating TOCTOU races.
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

        if payload.is_reply {
            // Client path: verifies the echoed link_id matches what we sent and stores the remote
            // version atomically (replay-protected).
            if !conn.complete_negotiation_as_client(link_id, version) {
                debug!(%in_connection, %link_id, "ignoring link negotiation reply");
            }
        } else {
            // Server path: validates link_id as UUID v4, stores it together with the remote
            // version atomically (replay-protected), then echoes a reply.
            if !conn.complete_negotiation_as_server(link_id, version) {
                debug!(%in_connection, %link_id, "ignoring link negotiation request");
                return Ok(());
            }

            // Route recovery: if the peer reconnected with a known link_id, restore all
            // routing state that was preserved during the recovery window.
            if let Some(entry) = self.internal.recovery_table.take(link_id) {
                info!(%in_connection, %link_id, "recovering routes for reconnected peer");

                // Re-add local routing entries.  A new conn_index was allocated for this
                // connection, so we must re-register each name under the current index,
                // preserving the original subscription IDs so UNSUBSCRIBE messages work.
                for (name, sub_ids) in &entry.local_subs {
                    for &subscription_id in sub_ids {
                        if let Err(e) = self.forwarder().on_subscription_msg(
                            name.clone(),
                            in_connection,
                            false,
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

                // Re-send subscriptions to the remote peer and re-register tracking.
                // restore_tracking = true: on_connection_drop already wiped the
                // forwarded-subscription table, so we must rebuild it here.
                self.restore_remote_subscriptions(&entry.remote_subs, in_connection, true)
                    .await;
            }

            // Send reply only after state is committed.
            let reply =
                ProtoMessage::builder().build_link_negotiation(link_id, local_version(), true);
            if let Err(e) = self.send_msg(reply, in_connection).await {
                debug!(
                    %in_connection,
                    error = %e.chain(),
                    "failed to send link negotiation reply",
                );
            }
        }

        Ok(())
    }

    async fn process_publish(&self, msg: Message, in_connection: u64) -> Result<(), DataPathError> {
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

        // get header
        let header = msg.get_slim_header();

        let dst = header.get_dst();

        // this function may panic, but at this point we are sure we are processing
        // a publish message
        let fanout = msg.get_fanout();

        self.match_and_forward_msg(msg, dst, in_connection, fanout)
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
            connection.is_local_connection(),
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

        // If forwarding to a remote ACK-capable node (v≥1.2.0), use the remote ack path:
        // update local state now, then asynchronously forward and wait for the remote ACK
        // before notifying the upstream requester.
        let use_remote_ack = forward
            .and_then(|out| self.forwarder().get_connection(out))
            .map(|c| crate::subscription_ack::supports(&c))
            .unwrap_or(false);

        if forward.is_some() && !use_remote_ack {
            debug!(
                forward_to = forward,
                "subscription: remote ack not available, link negotiation may not have completed yet"
            );
        }

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
        is_local: bool,
    ) -> Result<(), DataPathError> {
        match msg.message_type {
            Some(SubscribeType(_)) => self.process_subscription(msg, in_connection, true).await,
            Some(UnsubscribeType(_)) => self.process_subscription(msg, in_connection, false).await,
            Some(PublishType(_)) => self.process_publish(msg, in_connection).await,
            Some(LinkType(link)) => {
                self.handle_link_message(link, in_connection, is_local)
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

    async fn handle_new_message(
        &self,
        conn_index: u64,
        is_local: bool,
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

            if message_otel_active(&msg) {
                let parent = if is_local {
                    None
                } else {
                    extract_parent_context(&msg)
                };
                let span = self.create_process_span(
                    "process_local",
                    &msg,
                    ProcessSpanTarget::Connection(conn_index),
                );
                attach_trace_to_message(&mut msg, span, parent);
            }
        }

        match self.process_message(msg, conn_index, is_local).await {
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
        local_subs: HashMap<Name, HashSet<u64>>,
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
        is_local: bool,
        from_control_plane: bool,
    ) -> Result<JoinHandle<()>, DataPathError> {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();
        let client_conf_clone = client_config.clone();
        let tx_cp: Option<Sender<Result<Message, Status>>> = self.get_tx_control_plane();
        let watch = self.get_drain_watch()?;
        let span = tracing::info_span!(
            "process_stream",
            service_id = %self.internal.service_id,
            %conn_index,
            is_local,
        );
        let handle = tokio::spawn(async move {
            let mut try_to_reconnect = true;

            let mut watch = std::pin::pin!(watch.signaled());
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
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

                                        if let Err(e) = self_clone.handle_new_message(conn_index, is_local, msg).await {
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
                    .on_connection_drop(conn_index, is_local);

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
                                            .on_publish_msg_match(name.clone(), u64::MAX, u32::MAX)
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

        let connection = Connection::new(ConnectionType::Remote, Channel::Server(tx))
            .with_remote_addr(remote_addr)
            .with_local_addr(local_addr);

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
            false,
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
    use slim_config::grpc::client::is_valid_uuid_v4;
    use std::time::Duration;

    use super::*;
    use crate::api::ProtoSubscriptionAck;
    use crate::tables::remote_subscription_table::SubscriptionInfo;
    use tonic::Status;

    async fn assert_failed_subscription_ack_is_sent(add: bool) {
        let processor = MessageProcessor::new();
        let (in_connection, _tx, mut rx) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");

        let source = Name::from_strings(["org", "ns", "source"]).with_id(1);
        let destination = Name::from_strings(["org", "ns", "destination"]).with_id(2);
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

    #[test]
    fn test_is_valid_uuid_v4_accepts_v4() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(is_valid_uuid_v4(&id));
    }

    #[test]
    fn test_is_valid_uuid_v4_rejects_non_uuid_string() {
        assert!(!is_valid_uuid_v4("not-a-uuid"));
        assert!(!is_valid_uuid_v4(""));
    }

    #[test]
    fn test_is_valid_uuid_v4_rejects_non_v4_uuid() {
        // Version 1 UUID (time-based).
        assert!(!is_valid_uuid_v4("00000000-0000-1000-8000-000000000000"));
    }

    // ── handle_link_message ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_link_message_is_local_ignored() {
        let processor = MessageProcessor::new();
        let link = ProtoLink { link_type: None };
        assert!(processor.handle_link_message(link, 0, true).await.is_ok());
    }

    #[tokio::test]
    async fn test_handle_link_message_none_link_type_ignored() {
        let processor = MessageProcessor::new();
        let link = ProtoLink { link_type: None };
        assert!(processor.handle_link_message(link, 0, false).await.is_ok());
    }

    // ── handle_link_negotiation ───────────────────────────────────────────────

    fn make_server_conn(
        processor: &MessageProcessor,
    ) -> (u64, tokio::sync::mpsc::Receiver<Result<Message, Status>>) {
        let (tx, rx) = mpsc::channel(16);
        let conn = Connection::new(ConnectionType::Remote, Channel::Server(tx));
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
        let conn = Connection::new(ConnectionType::Remote, Channel::Client(tx));
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
    async fn test_handle_link_negotiation_server_invalid_uuid_ignored() {
        let processor = MessageProcessor::new();
        let (conn_id, _rx) = make_server_conn(&processor);
        let payload = LinkNegotiationPayload {
            link_id: "not-a-uuid".into(),
            slim_version: "1.0.0".into(),
            is_reply: false,
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
    async fn test_handle_link_negotiation_server_happy_path() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);
        let link_id = uuid::Uuid::new_v4().to_string();
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.2.3".into(),
            is_reply: false,
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

    /// Helper: negotiate a server connection to version `v` so
    /// `subscription_ack::supports` returns the expected value.
    fn negotiate_conn(processor: &MessageProcessor, conn_id: u64, version: &str) {
        let c = processor.forwarder().get_connection(conn_id).unwrap();
        c.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::parse(version).unwrap(),
        );
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

        let source = Name::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = Name::from_strings(["org", "ns", "dst"]).with_id(2);
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
    async fn test_process_subscription_remote_ack_path_old_node_immediate_ack() {
        // Old remote node (v < 1.2.0): should use the existing immediate-ack path.
        let processor = MessageProcessor::new();
        let (local_conn, _tx_local, mut rx_local) = processor
            .register_local_connection(false)
            .expect("failed to create local connection");

        let (remote_conn, mut rx_remote) = make_server_conn(&processor);
        negotiate_conn(&processor, remote_conn, "1.1.0");

        let source = Name::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = Name::from_strings(["org", "ns", "dst"]).with_id(2);
        let upstream_ack_id: u64 = 101;

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

        // Forwarded subscribe must have been sent to remote.
        // The subscription_id is a globally unique identifier that always travels
        // with the subscription, regardless of whether the remote supports acks.
        let forwarded = tokio::time::timeout(Duration::from_secs(1), rx_remote.recv())
            .await
            .expect("timeout waiting for forwarded subscribe")
            .expect("channel closed")
            .unwrap();
        assert!(matches!(forwarded.get_type(), SubscribeType(_)));
        let forwarded_sub_id = forwarded
            .get_subscription_id()
            .expect("forwarded subscribe must carry the subscription_id");
        assert_eq!(
            forwarded_sub_id, upstream_ack_id,
            "subscription_id must not change when forwarding"
        );

        // Upstream ACK must be sent immediately (without waiting for remote).
        let upstream_ack = tokio::time::timeout(Duration::from_secs(1), rx_local.recv())
            .await
            .expect("timeout waiting for upstream ack")
            .expect("channel closed")
            .expect("upstream ack must be Ok");

        assert!(matches!(upstream_ack.get_type(), SubscriptionAckType(_)));
        let ack = upstream_ack.get_subscription_ack();
        assert_eq!(ack.subscription_id, upstream_ack_id);
        assert!(ack.success);
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

        let source = Name::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = Name::from_strings(["org", "ns", "dst"]).with_id(2);
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
        let source = Name::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = Name::from_strings(["org", "ns", "dst"]).with_id(2);
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
        let (remote_conn, mut rx_remote) = make_server_conn(&processor);

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
        let (remote_conn, mut rx_remote) = make_server_conn(&processor);

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
        let (remote_conn, mut rx_remote) = make_server_conn(&processor);

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
        let name = Name::from_strings(["org", "default", "svc"]);
        subs.insert(name.clone(), HashSet::from([1u64, 2u64]));

        MessageProcessor::notify_control_plane_subscriptions_lost(Some(tx), subs, 42).await;

        let msg = rx.recv().await.unwrap().unwrap();
        assert!(matches!(msg.get_type(), UnsubscribeType(_)));
        assert_eq!(msg.get_source(), name.clone());
    }

    #[tokio::test]
    async fn test_notify_cp_subs_lost_no_tx_is_noop() {
        let subs = HashMap::from([(
            Name::from_strings(["org", "default", "svc"]),
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
        let sub_name = Name::from_strings(["org", "default", "recovered"]);

        // Pre-populate the recovery table as if a prior connection dropped.
        let mut local_subs = HashMap::new();
        local_subs.insert(sub_name.clone(), HashSet::from([99u64]));
        processor
            .internal
            .recovery_table
            .store(link_id.clone(), local_subs, HashSet::new());

        // Simulate the peer reconnecting with the same link_id.
        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
        };
        processor
            .handle_link_negotiation(&payload, conn_id)
            .await
            .unwrap();

        // The subscription should have been restored in the routing table.
        let result = processor
            .forwarder()
            .on_publish_msg_match(sub_name, u64::MAX, 1);
        assert!(result.is_ok(), "recovered subscription should be routable");
        assert_eq!(result.unwrap(), vec![conn_id]);
    }

    #[tokio::test]
    async fn test_link_negotiation_server_recovery_restores_remote_subs() {
        let processor = MessageProcessor::new();
        let (conn_id, mut rx) = make_server_conn(&processor);

        let link_id = uuid::Uuid::new_v4().to_string();
        let source = Name::from_strings(["org", "default", "src"]);
        let dest = Name::from_strings(["org", "default", "dst"]);

        let remote_sub =
            SubscriptionInfo::new(source.clone(), dest.clone(), "identity".into(), conn_id, 42);

        // Store recovery entry with remote subscriptions.
        processor.internal.recovery_table.store(
            link_id.clone(),
            HashMap::new(),
            HashSet::from([remote_sub]),
        );

        let payload = LinkNegotiationPayload {
            link_id: link_id.clone(),
            slim_version: "1.0.0".into(),
            is_reply: false,
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

        let source = Name::from_strings(["org", "default", "src"]);
        let dest = Name::from_strings(["org", "default", "dst"]);
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
        let (conn_id, mut rx) = make_server_conn(&processor);

        let source = Name::from_strings(["org", "default", "src"]);
        let dest = Name::from_strings(["org", "default", "dst"]);
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

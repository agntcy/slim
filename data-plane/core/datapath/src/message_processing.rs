// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use crate::api::DataPlaneServiceServer;
use display_error_chain::ErrorChainExt;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use parking_lot::RwLock;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_tracing::utils::INSTANCE_ID;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use tonic::{Request, Response, Status};
use tracing::{Span, debug, error, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::api::ProtoPublishType as PublishType;
use crate::api::ProtoSubscribeType as SubscribeType;
use crate::api::ProtoUnsubscribeType as UnsubscribeType;
use crate::api::proto::dataplane::v1::Message;

use crate::api::proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
use crate::api::proto::dataplane::v1::data_plane_service_server::DataPlaneService;
use crate::connection::{Channel, Connection, Type as ConnectionType};
use crate::errors::{DataPathError, MessageContext};
use crate::forwarder::Forwarder;
use crate::messages::Name;
use crate::messages::utils::SlimHeaderFlags;
use crate::tables::connection_table::ConnectionTable;
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

// Helper function to create the trace span
fn create_span(function: &str, out_conn: u64, msg: &Message) -> Span {
    let span = tracing::span!(
        tracing::Level::INFO,
        "slim_process_message",
        function = function,
        source = format!("{}", msg.get_source()),
        destination =  format!("{}", msg.get_dst()),
        instance_id = %INSTANCE_ID.as_str(),
        connection_id = out_conn,
        message_type = msg.get_type().to_string(),
        telemetry = true
    );

    if let PublishType(_) = msg.get_type() {
        span.set_attribute("session_type", msg.get_session_message_type().as_str_name());
        span.set_attribute(
            "session_id",
            msg.get_session_header().get_session_id().to_string(),
        );
        span.set_attribute(
            "message_id",
            msg.get_session_header().get_message_id().to_string(),
        );
    }

    span
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
}

#[derive(Debug, Clone)]
pub struct MessageProcessor {
    internal: Arc<MessageProcessorInternal>,
}

impl Default for MessageProcessor {
    fn default() -> Self {
        let (signal, watch) = drain::channel();
        let forwarder = Forwarder::new();
        let forwarder = MessageProcessorInternal {
            forwarder,
            drain_signal: RwLock::new(Some(signal)),
            drain_watch: RwLock::new(Some(watch)),
            tx_control_plane: RwLock::new(None),
        };

        Self {
            internal: Arc::new(forwarder),
        }
    }
}

impl MessageProcessor {
    pub fn new() -> Self {
        Self::default()
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

    fn get_drain_watch(&self) -> Result<drain::Watch, DataPathError> {
        self.internal
            .drain_watch
            .read()
            .clone()
            .ok_or(DataPathError::AlreadyClosedError)
    }

    async fn try_to_connect(
        &self,
        client_config: ClientConfig,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
        existing_conn_index: Option<u64>,
    ) -> Result<(JoinHandle<()>, u64), DataPathError> {
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

        let cancellation_token = CancellationToken::new();
        let connection = Connection::new(ConnectionType::Remote, Channel::Client(tx))
            .with_local_addr(local)
            .with_remote_addr(remote)
            .with_config_data(Some(client_config.clone()))
            .with_cancellation_token(Some(cancellation_token.clone()));

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
            Some(client_config),
            cancellation_token,
            false,
            false,
        )?;

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

    pub async fn send_msg(&self, mut msg: Message, out_conn: u64) -> Result<(), DataPathError> {
        let connection = self.forwarder().get_connection(out_conn);
        match connection {
            Some(conn) => {
                // reset header fields
                msg.clear_slim_header();

                // telemetry ////////////////////////////////////////////////////////
                let parent_context = extract_parent_context(&msg);
                let span = create_span("send_message", out_conn, &msg);

                if let Some(ctx) = parent_context
                    && let Err(e) = span.set_parent(ctx)
                {
                    // log the error but don't fail the message sending
                    error!(error = %e.chain(), "error setting parent context");
                }
                let _guard = span.enter();
                inject_current_context(&mut msg);
                ///////////////////////////////////////////////////////////////////

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
        msg: Message,
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
                // in case out_vec.len = 1, do not clone the message.
                // in the other cases clone only len - 1 times.
                let mut i = 0;
                while i < out_vec.len() - 1 {
                    self.send_msg(msg.clone(), out_vec[i]).await?;
                    i += 1;
                }
                self.send_msg(msg, out_vec[i]).await?;
                Ok(())
            }
            Err(e) => Err(DataPathError::MessageProcessingError {
                source: Box::new(e),
                msg: Box::new(msg),
            }),
        }
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

        let dst = msg.get_dst();

        // get header
        let header = msg.get_slim_header();

        // get in and out connections
        let (conn, forward) = header.get_in_out_connections();

        // get input connection. As connection is deleted only after the processing,
        // it is safe to assume that at this point the connection must exist.
        let maybe_connection = self.forwarder().get_connection(conn);

        // Required to make the borrow checker happy
        let connection = if let Some(c) = maybe_connection {
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
        )?;

        match forward {
            None => {
                // if the subscription is not forwarded, we are done
                Ok(())
            }
            Some(out_conn) => {
                debug!(%out_conn, "forwarding {}subscription to connection", if add { "" } else { "un" });

                // get source name and identity
                let source = msg.get_source();
                let identity = msg.get_identity();

                // send message
                self.send_msg(msg, out_conn).await.map(|_| {
                    self.forwarder()
                        .on_forwarded_subscription(source, dst, identity, out_conn, add);
                })
            }
        }
    }

    pub async fn process_message(
        &self,
        msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        // process each kind of message in a different path
        match msg.get_type() {
            SubscribeType(_) => self.process_subscription(msg, in_connection, true).await,
            UnsubscribeType(_) => self.process_subscription(msg, in_connection, false).await,
            PublishType(_) => self.process_publish(msg, in_connection).await,
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

        // add incoming connection to the SLIM header
        msg.set_incoming_conn(Some(conn_index));

        // message is valid - from now on we access the field without checking for None

        // telemetry /////////////////////////////////////////
        if is_local {
            // handling the message from the local application
            let span = create_span("process_local", conn_index, &msg);

            let _guard = span.enter();

            inject_current_context(&mut msg);
        } else {
            // handling the message from a remote SLIM instance
            let parent_context = extract_parent_context(&msg);

            let span = create_span("process_local", conn_index, &msg);

            if let Some(ctx) = parent_context
                && let Err(e) = span.set_parent(ctx)
            {
                // log the error but don't fail the message processing
                error!(error = %e.chain(), "error setting parent context");
            }
            let _guard = span.enter();

            inject_current_context(&mut msg);
        }
        //////////////////////////////////////////////////////

        match self.process_message(msg, conn_index).await {
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
                        // Restore subscriptions on the remote node
                        for r in remote_subscriptions.iter() {
                            let sub_msg = Message::builder()
                                .source(r.source().clone())
                                .destination(r.name().clone())
                                .identity(r.source_identity())
                                .build_subscribe()
                                .unwrap();
                            if let Err(e) = self.send_msg(sub_msg, conn_index).await {
                                error!(error = %e.chain(), "error restoring subscription on remote node");
                            }
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
                                                PublishType(_) => {/* do nothing */}
                                                _ => {
                                                    // send subscriptions and unsupcriptions
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

            let mut connected = false;

            if try_to_reconnect && let Some(config) = client_conf_clone {
                connected = self_clone.reconnect(config, conn_index, &token_clone).await;
            } else {
                debug!(%conn_index, "close connection")
            }

            if !connected {
                //delete connection state
                let (local_subs, _remote_subs) = self_clone
                    .forwarder()
                    .on_connection_drop(conn_index, is_local);

                // if connection is not local and controller exists, notify about lost subscriptions on the connection
                if let (false, Some(tx)) = (is_local, tx_cp) {
                    for local_sub in local_subs {
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
                                "failed to send unsubscribe message to control plane for subscription",
                            );
                        }
                    }
                }

                info!(telemetry = true, counter.num_active_connections = -1);
            }
        });

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

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use parking_lot::RwLock;
use slim_config::grpc::client::ClientConfig;
use slim_tracing::utils::INSTANCE_ID;
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tonic::codegen::{Body, StdError};
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
use crate::errors::DataPathError;
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
    forwarder: Forwarder<Connection>,
    drain_channel: drain::Watch,
    tx_control_plane: RwLock<Option<Sender<Result<Message, Status>>>>,
}

#[derive(Debug, Clone)]
pub struct MessageProcessor {
    internal: Arc<MessageProcessorInternal>,
}

impl MessageProcessor {
    pub fn new() -> (Self, drain::Signal) {
        let (signal, watch) = drain::channel();
        let forwarder = Forwarder::new();
        let forwarder = MessageProcessorInternal {
            forwarder,
            drain_channel: watch,
            tx_control_plane: RwLock::new(None),
        };

        (
            Self {
                internal: Arc::new(forwarder),
            },
            signal,
        )
    }

    pub fn with_drain_channel(watch: drain::Watch) -> Self {
        let forwarder = Forwarder::new();
        let forwarder = MessageProcessorInternal {
            forwarder,
            drain_channel: watch,
            tx_control_plane: RwLock::new(None),
        };
        Self {
            internal: Arc::new(forwarder),
        }
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

    fn get_drain_watch(&self) -> drain::Watch {
        self.internal.drain_channel.clone()
    }

    async fn try_to_connect<C>(
        &self,
        channel: C,
        client_config: Option<ClientConfig>,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
        existing_conn_index: Option<u64>,
        max_retry: u32,
    ) -> Result<(tokio::task::JoinHandle<()>, u64), DataPathError>
    where
        C: tonic::client::GrpcService<tonic::body::Body>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        let mut client: DataPlaneServiceClient<C> = DataPlaneServiceClient::new(channel);
        let mut i = 0;
        while i < max_retry {
            let (tx, rx) = mpsc::channel(128);
            match client
                .open_channel(Request::new(ReceiverStream::new(rx)))
                .await
            {
                Ok(stream) => {
                    let cancellation_token = CancellationToken::new();
                    let connection = Connection::new(ConnectionType::Remote)
                        .with_local_addr(local)
                        .with_remote_addr(remote)
                        .with_channel(Channel::Client(tx))
                        .with_config_data(client_config.clone())
                        .with_cancellation_token(Some(cancellation_token.clone()));

                    debug!(
                        "new connection initiated locally: (remote: {:?} - local: {:?})",
                        connection.remote_addr(),
                        connection.local_addr()
                    );

                    // insert connection into connection table
                    let opt = self
                        .forwarder()
                        .on_connection_established(connection, existing_conn_index);
                    if opt.is_none() {
                        error!("error adding connection to the connection table");
                        return Err(DataPathError::ConnectionError(
                            "error adding connection to the connection tables".to_string(),
                        ));
                    }

                    let conn_index = opt.unwrap();
                    debug!(
                        "new connection index = {:?}, is local {:?}",
                        conn_index, false
                    );

                    // Start loop to process messages
                    let ret = self.process_stream(
                        stream.into_inner(),
                        conn_index,
                        client_config,
                        cancellation_token,
                        false,
                        false,
                    );
                    return Ok((ret, conn_index));
                }
                Err(e) => {
                    error!("connection error: {:?}.", e.to_string());
                }
            }
            i += 1;

            // sleep 1 sec between each connection retry
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        error!("unable to connect to the endpoint");
        Err(DataPathError::ConnectionError(
            "reached max connection retries".to_string(),
        ))
    }

    pub async fn connect<C>(
        &self,
        channel: C,
        client_config: Option<ClientConfig>,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
    ) -> Result<(tokio::task::JoinHandle<()>, u64), DataPathError>
    where
        C: tonic::client::GrpcService<tonic::body::Body>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        self.try_to_connect(channel, client_config, local, remote, None, 10)
            .await
    }

    pub fn disconnect(&self, conn: u64) -> Result<ClientConfig, DataPathError> {
        let connection = match self.forwarder().get_connection(conn) {
            Some(c) => c,
            None => {
                error!("error handling disconnect: connection unknown");
                return Err(DataPathError::DisconnectionError(
                    "connection not found".to_string(),
                ));
            }
        };

        let token = match connection.cancellation_token() {
            Some(t) => t,
            None => {
                error!("error handling disconnect: missing cancellation token");
                return Err(DataPathError::DisconnectionError(
                    "missing cancellation token".to_string(),
                ));
            }
        };

        // Cancel receiving loop; this triggers deletion of connection state.
        token.cancel();

        connection
            .config_data()
            .cloned()
            .ok_or(DataPathError::DisconnectionError(
                "missing client config data".to_string(),
            ))
    }

    pub fn register_local_connection(
        &self,
        from_control_plane: bool,
    ) -> (
        u64,
        tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tokio::sync::mpsc::Receiver<Result<Message, Status>>,
    ) {
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
        let connection = Connection::new(ConnectionType::Local)
            .with_channel(Channel::Server(tx2))
            .with_cancellation_token(Some(cancellation_token.clone()));

        // add it to the connection table
        let conn_id = self
            .forwarder()
            .on_connection_established(connection, None)
            .unwrap();

        debug!("local connection established with id: {:?}", conn_id);
        info!(telemetry = true, counter.num_active_connections = 1);

        // this loop will process messages from the local app
        self.process_stream(
            ReceiverStream::new(rx1),
            conn_id,
            None,
            cancellation_token,
            true,
            from_control_plane,
        );

        // return the conn_id and  handles to be used to send and receive messages
        (conn_id, tx1, rx2)
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
                    error!("error setting parent context: {:?}", e);
                }
                let _guard = span.enter();
                inject_current_context(&mut msg);
                ///////////////////////////////////////////////////////////////////

                match conn.channel() {
                    Channel::Server(s) => s
                        .send(Ok(msg))
                        .await
                        .map_err(|e| DataPathError::MessageSendError(e.to_string())),
                    Channel::Client(s) => s
                        .send(msg)
                        .await
                        .map_err(|e| DataPathError::MessageSendError(e.to_string())),
                    _ => Err(DataPathError::MessageSendError(
                        "connection not found".to_string(),
                    )),
                }
            }
            None => Err(DataPathError::MessageSendError(format!(
                "connection {:?} not found",
                out_conn
            ))),
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
            "match and forward message: name: {} - fanout: {}",
            name, fanout,
        );

        // if the message already contains an output connection, use that one
        // without performing any match in the subscription table
        if let Some(val) = msg.get_forward_to() {
            debug!("forwarding message to connection {}", val);
            return self
                .send_msg(msg, val)
                .await
                .map_err(|e| DataPathError::PublicationError(e.to_string()));
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
                    self.send_msg(msg.clone(), out_vec[i])
                        .await
                        .map_err(|e| DataPathError::PublicationError(e.to_string()))?;
                    i += 1;
                }
                self.send_msg(msg, out_vec[i])
                    .await
                    .map_err(|e| DataPathError::PublicationError(e.to_string()))?;
                Ok(())
            }
            Err(e) => Err(DataPathError::PublicationError(e.to_string())),
        }
    }

    async fn process_publish(&self, msg: Message, in_connection: u64) -> Result<(), DataPathError> {
        debug!(
            "received publication from connection {}: {:?}",
            in_connection, msg
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

        // if we get valid type also the name is valid so we can safely unwrap
        return self
            .match_and_forward_msg(msg, dst, in_connection, fanout)
            .await;
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
            "received subscription (add = {}) from connection {}: {:?}",
            add, in_connection, msg
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
        let connection = self
            .forwarder()
            .get_connection(conn)
            .ok_or_else(|| DataPathError::SubscriptionError("connection not found".to_string()))?;

        debug!(
            "subscription update (add = {}) for name: {} - connection: {}",
            add, dst, conn
        );

        if let Err(e) = self.forwarder().on_subscription_msg(
            dst.clone(),
            conn,
            connection.is_local_connection(),
            add,
        ) {
            return Err(DataPathError::SubscriptionError(e.to_string()));
        }

        match forward {
            None => {
                // if the subscription is not forwarded, we are done
                Ok(())
            }
            Some(out_conn) => {
                debug!("forward subscription (add = {}) to {}", add, out_conn);

                // get source name and identity
                let source = msg.get_source();
                let identity = msg.get_identity();

                // send message
                match self.send_msg(msg, out_conn).await {
                    Ok(_) => {
                        self.forwarder()
                            .on_forwarded_subscription(source, dst, identity, out_conn, add);
                        Ok(())
                    }
                    Err(e) => Err(DataPathError::UnsubscriptionError(e.to_string())),
                }
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

            return Err(DataPathError::InvalidMessage(err.to_string()));
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
                error!("error setting parent context: {:?}", e);
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
                Err(DataPathError::ProcessingError(e.to_string()))
            }
        }
    }

    async fn send_error_to_local_app(&self, conn_index: u64, err: DataPathError) {
        let connection = self.forwarder().get_connection(conn_index);
        match connection {
            Some(conn) => {
                debug!("try to notify the error to the local application");
                if let Channel::Server(tx) = conn.channel() {
                    // create Status error
                    let status = Status::new(
                        tonic::Code::Internal,
                        format!("error processing message: {:?}", err),
                    );

                    if tx.send(Err(status)).await.is_err() {
                        debug!("unable to notify the error to the local app: {:?}", err);
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
        client_conf: Option<ClientConfig>,
        conn_index: u64,
        cancellation_token: &CancellationToken,
    ) -> bool {
        let config = client_conf.unwrap();
        match config.to_channel().await {
            Err(e) => {
                error!(
                    "cannot parse connection config, unable to reconnect {:?}",
                    e.to_string()
                );
                false
            }
            Ok(channel) => {
                info!("connection lost with remote endpoint, try to reconnect");
                // These are the subscriptions that we forwarded to the remote SLIM on
                // this connection. It is necessary to restore them to keep receive the messages
                // The connections on the local subscription table (created using the set_route command)
                // are still there and will be removed only if the reconnection process will fail.
                let remote_subscriptions = self
                    .forwarder()
                    .get_subscriptions_forwarded_on_connection(conn_index);

                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        debug!("cancellation token signaled, stopping reconnection process");
                        false
                    }
                    _ = self.get_drain_watch().signaled() => {
                        debug!("drain watch signaled, stopping reconnection process");
                        false
                    }
                    res = self.try_to_connect(channel, Some(config), None, None, Some(conn_index), 120) => {
                        match res {
                            Ok(_) => {
                                info!("connection re-established");
                                // the subscription table should be ok already
                                for r in remote_subscriptions.iter() {
                                    let sub_msg = Message::builder()
                                        .source(r.source().clone())
                                        .destination(r.name().clone())
                                        .identity(r.source_identity())
                                        .build_subscribe()
                                        .unwrap();
                                    if self.send_msg(sub_msg, conn_index).await.is_err() {
                                        error!("error restoring subscription on remote node");
                                    }
                                }
                                true
                            }
                            Err(e) => {
                                // TODO: notify the app that the connection is not working anymore
                                error!("unable to connect to remote node {:?}", e.to_string());
                                false
                            }
                        }
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
    ) -> tokio::task::JoinHandle<()> {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();
        let client_conf_clone = client_config.clone();
        let tx_cp: Option<Sender<Result<Message, Status>>> = self.get_tx_control_plane();

        tokio::spawn(async move {
            let mut try_to_reconnect = true;
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
                                        if !is_local && !from_control_plane && tx_cp.is_some(){
                                            match msg.get_type() {
                                                PublishType(_) => {/* do nothing */}
                                                _ => {
                                                    // send subscriptions and unsupcriptions
                                                    // to the control plane
                                                    let _ = tx_cp.as_ref().unwrap().send(Ok(msg.clone())).await;
                                                }
                                            }
                                        }

                                        if let Err(e) = self_clone.handle_new_message(conn_index, is_local, msg).await {
                                            error!(%conn_index, %e, "error processing incoming message");
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
                                            error!("error receiving messages {:?}", e);
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
                    _ = self_clone.get_drain_watch().signaled() => {
                        debug!("shutting down stream on drain: {}", conn_index);
                        try_to_reconnect = false;
                        break;
                    }
                    _ = token_clone.cancelled() => {
                        debug!("shutting down stream on cancellation token: {}", conn_index);
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

            if try_to_reconnect && client_conf_clone.is_some() {
                connected = self_clone
                    .reconnect(client_conf_clone, conn_index, &token_clone)
                    .await;
            } else {
                debug!("close connection {}", conn_index)
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
                            "notify control plane about lost subscription: {}",
                            local_sub
                        );
                        let msg = Message::builder()
                            .source(local_sub.clone())
                            .destination(local_sub.clone())
                            .flags(SlimHeaderFlags::default().with_recv_from(conn_index))
                            .build_unsubscribe()
                            .unwrap();
                        if let Err(e) = tx.send(Ok(msg)).await {
                            error!(
                                "failed to send unsubscribe message to control plane for subscription {}: {}",
                                local_sub, e
                            );
                        }
                    }
                }

                info!(telemetry = true, counter.num_active_connections = -1);
            }
        })
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

        let connection = Connection::new(ConnectionType::Remote)
            .with_remote_addr(remote_addr)
            .with_local_addr(local_addr)
            .with_channel(Channel::Server(tx));

        debug!(
            "new connection received from remote: (remote: {:?} - local: {:?})",
            connection.remote_addr(),
            connection.local_addr()
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
        );

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}

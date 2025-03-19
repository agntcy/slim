// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use agp_config::grpc::client::ClientConfig;
use agp_tracing::utils::INSTANCE_ID;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tonic::codegen::{Body, StdError};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::connection::{Channel, Connection, Type as ConnectionType};
use crate::errors::DataPathError;
use crate::forwarder::Forwarder;
use crate::messages::utils::{
    clear_agp_header, create_agp_header, create_error_publication, create_subscription, get_fanout,
    get_forward_to, get_name, get_recv_from, get_source, message_type_to_str,
    set_incoming_connection,
};
use crate::messages::AgentType;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Publish as PublishType;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Subscribe as SubscribeType;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Unsubscribe as UnsubscribeType;
use crate::pubsub::proto::pubsub::v1::pub_sub_service_client::PubSubServiceClient;
use crate::pubsub::proto::pubsub::v1::{pub_sub_service_server::PubSubService, Message};

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

#[derive(Debug)]
struct MessageProcessorInternal {
    forwarder: Forwarder<Connection>,
    drain_channel: drain::Watch,
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
        };
        Self {
            internal: Arc::new(forwarder),
        }
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
        C: tonic::client::GrpcService<tonic::body::BoxBody>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        let mut client: PubSubServiceClient<C> = PubSubServiceClient::new(channel);
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
                        .with_cancellation_token(Some(cancellation_token.clone()));

                    info!(
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
                    info!(
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
        C: tonic::client::GrpcService<tonic::body::BoxBody>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        self.try_to_connect(channel, client_config, local, remote, None, 10)
            .await
    }

    pub fn disconnect(&self, conn: u64) -> Result<(), DataPathError> {
        match self.forwarder().get_connection(conn) {
            None => {
                error!("error handling disconnect: connection unknown");
                return Err(DataPathError::DisconnectionError(
                    "connection not found".to_string(),
                ));
            }
            Some(c) => {
                match c.cancellation_token() {
                    None => {
                        error!("error handling disconnect: missing cancellation token");
                    }
                    Some(t) => {
                        // here token cancel will stop the receiving loop on
                        // conn and this will cause the delition of the state
                        // for this connection
                        t.cancel();
                    }
                }
            }
        }

        Ok(())
    }

    pub fn register_local_connection(
        &self,
    ) -> (
        tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tokio::sync::mpsc::Receiver<Result<Message, Status>>,
    ) {
        // create a pair tx, rx to be able to send messages with the standard processing loop
        let (tx1, rx1) = mpsc::channel(128);

        info!("establishing new local app connection");

        // create a pair tx, rx to be able to receive messages and insert it into the connection table
        let (tx2, rx2) = mpsc::channel(128);

        // create a connection
        let connection = Connection::new(ConnectionType::Local).with_channel(Channel::Server(tx2));

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
            CancellationToken::new(),
            true,
        );

        // return the handles to be used to send and receive messages
        (tx1, rx2)
    }

    pub async fn send_msg(
        &self,
        mut msg: Message,
        out_conn: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let connection = self.forwarder().get_connection(out_conn);
        match connection {
            Some(conn) => {
                // reset header fields
                clear_agp_header(&mut msg)?;

                let parent_context = extract_parent_context(&msg);
                let span = tracing::span!(
                tracing::Level::DEBUG,
                    "send_message",
                    instance_id = %INSTANCE_ID.as_str(),
                    connection_id = out_conn,
                    message_type = message_type_to_str(&msg),
                    telemetry = true
                );

                if let Some(ctx) = parent_context {
                    span.set_parent(ctx);
                }
                let _guard = span.enter();
                inject_current_context(&mut msg);

                match conn.channel() {
                    Channel::Server(s) => s.send(Ok(msg)).await?,
                    Channel::Client(s) => s.send(msg).await?,
                    _ => error!("error reading channel"),
                }
            }
            None => error!("connection {:?} not found", out_conn),
        }
        Ok(())
    }

    async fn match_and_forward_msg(
        &self,
        msg: Message,
        agent_type: AgentType,
        in_connection: u64,
        fanout: u32,
        agent_id: Option<u64>,
    ) -> Result<(), DataPathError> {
        debug!(
            "match and forward message: class: {:?} - agent_id: {:?} - fanout: {:?}",
            agent_type, agent_id, fanout,
        );

        // if the message already contains an output connection, use that one
        // without performing any match in the subscription table
        if let Some(val) = get_forward_to(&msg).unwrap_or(None) {
            info!("forwarding message to connection {:?}", val);
            return self.send_msg(msg, val).await.map_err(|e| {
                error!("error sending a message {:?}", e);
                DataPathError::PublicationError(e.to_string())
            });
        }

        match self
            .forwarder()
            .on_publish_msg_match(agent_type, agent_id, in_connection, fanout)
        {
            Ok(out_vec) => {
                // in case out_vec.len = 1, do not clone the message.
                // in the other cases clone only len - 1 times.
                let mut i = 0;
                while i < out_vec.len() - 1 {
                    self.send_msg(msg.clone(), out_vec[i]).await.map_err(|e| {
                        error!("error sending a message {:?}", e);
                        DataPathError::PublicationError(e.to_string())
                    })?;
                    i += 1;
                }
                self.send_msg(msg, out_vec[i]).await.map_err(|e| {
                    error!("error sending a message {:?}", e);
                    DataPathError::PublicationError(e.to_string())
                })?;
                Ok(())
            }
            Err(e) => {
                error!("error sending a message {:?}", e);
                Err(DataPathError::PublicationError(e.to_string()))
            }
        }
    }

    async fn process_publish(&self, msg: Message, in_connection: u64) -> Result<(), DataPathError> {
        let pubmsg = match &msg.message_type {
            Some(PublishType(p)) => p,
            // this should never happen
            _ => panic!("wrong message type"),
        };

        match get_name(&msg) {
            Ok((agent_type, agent_id)) => {
                let fanout = get_fanout(pubmsg);

                debug!(
                    "received publication from connection {}: {:?}",
                    in_connection, pubmsg
                );

                // if we get valid type also the name is valid so we can safely unwrap
                return self
                    .match_and_forward_msg(msg, agent_type, in_connection, fanout, agent_id)
                    .await;
            }
            Err(e) => {
                error!("error processing publication message {:?}", e);
                Err(DataPathError::PublicationError(e.to_string()))
            }
        }
    }

    // returns the connection to use to process correctly the message
    // first connection is from where we received the packet
    // the second is where to forward the packet if needed
    fn process_agp_header(
        &self,
        msg: &Message,
        in_connection: u64,
    ) -> Result<(u64, Option<u64>), DataPathError> {
        match get_recv_from(msg) {
            Ok(recv_from) => {
                if let Some(val) = recv_from {
                    debug!(
                        "received recv_from command, update state on connection {}",
                        val
                    );
                    return Ok((val, None));
                }
            }
            Err(e) => {
                error! {"error agp header: {:?}", e};
                return Err(DataPathError::CommandError(e.to_string()));
            }
        }

        match get_forward_to(msg) {
            Ok(fwd_to) => {
                if fwd_to.is_some() {
                    debug!(
                        "received forward_to command, update state and forward to connection {}",
                        fwd_to.unwrap()
                    );
                    return Ok((in_connection, fwd_to));
                }
            }
            Err(e) => {
                error! {"error agp header: {:?}", e};
                return Err(DataPathError::CommandError(e.to_string()));
            }
        }

        Ok((in_connection, None))
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

        match get_name(&msg) {
            Ok((agent_type, agent_id)) => {
                let (conn, forward) = match self.process_agp_header(&msg, in_connection) {
                    Ok((c, f)) => (c, f),
                    Err(e) => return Err(e),
                };

                let connection = self.forwarder().get_connection(conn);
                if connection.is_none() {
                    // this should never happen
                    error!("incoming connection does not exists");
                    return Err(DataPathError::SubscriptionError(
                        "incoming connection does not exists".to_string(),
                    ));
                }

                match self.forwarder().on_subscription_msg(
                    agent_type.clone(),
                    agent_id,
                    conn,
                    connection.unwrap().is_local_connection(),
                    add,
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(DataPathError::SubscriptionError(e.to_string()));
                    }
                }

                if forward.is_some() {
                    debug!("forward subscription (add = {}) to {:?}", add, forward);
                    let out_conn = forward.unwrap();

                    let (source_type, source_id) = match get_source(&msg) {
                        Ok((c, f)) => (c, f),
                        Err(e) => {
                            error!(
                                "error processing subscription (add = {}) source: {:?}",
                                add, e
                            );
                            return Err(DataPathError::UnsubscriptionError(e.to_string()));
                        }
                    };
                    match self.send_msg(msg, out_conn).await {
                        Ok(_) => {
                            self.forwarder().on_forwarded_subscription(
                                source_type,
                                source_id,
                                agent_type,
                                agent_id,
                                out_conn,
                                add,
                            );
                        }
                        Err(e) => {
                            error!("error sending a message: {:?}", e);
                            return Err(DataPathError::UnsubscriptionError(e.to_string()));
                        }
                    };
                }
                Ok(())
            }
            Err(e) => {
                error!(
                    "error processing subscription message (add = {}): {:?}",
                    add, e
                );
                Err(DataPathError::SubscriptionError(e.to_string()))
            }
        }
    }

    pub async fn process_message(
        &self,
        mut msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        // add incoming connection to the AGP header
        match set_incoming_connection(&mut msg, Some(in_connection)) {
            Ok(_) => {}
            Err(e) => {
                error!("error setting incoming connection {:?}", e);
                return Err(DataPathError::ErrorSettingInConnection(e.to_string()));
            }
        }

        match &msg.message_type {
            None => {
                error!(
                    "received message without message type from connection {}: {:?}",
                    in_connection, msg
                );
                info!(
                    telemetry = true,
                    monotonic_counter.num_messages_by_type = 1,
                    message_type = "none"
                );
                Err(DataPathError::UnknownMsgType("".to_string()))
            }
            Some(msg_type) => match msg_type {
                SubscribeType(s) => {
                    debug!(
                        "received subscription from connection {}: {:?}",
                        in_connection, s
                    );
                    info!(
                        telemetry = true,
                        monotonic_counter.num_messages_by_type = 1,
                        message_type = "subscribe"
                    );
                    self.process_subscription(msg, in_connection, true).await?;
                    Ok(())
                }
                UnsubscribeType(u) => {
                    debug!(
                        "Received ubsubscription from client {}: {:?}",
                        in_connection, u
                    );
                    info!(
                        telemetry = true,
                        monotonic_counter.num_messages_by_type = 1,
                        message_type = "unsubscribe"
                    );
                    self.process_subscription(msg, in_connection, false).await?;
                    Ok(())
                }
                PublishType(p) => {
                    debug!("Received publish from client {}: {:?}", in_connection, p);
                    info!(
                        telemetry = true,
                        monotonic_counter.num_messages_by_type = 1,
                        method = "publish"
                    );
                    self.process_publish(msg, in_connection).await?;
                    Ok(())
                }
            },
        }
    }

    async fn handle_new_message(
        &self,
        conn_index: u64,
        is_local: bool,
        mut msg: Message,
    ) -> Result<(), DataPathError> {
        debug!(%conn_index, "Received message from connection");
        info!(
            telemetry = true,
            monotonic_counter.num_processed_messages = 1
        );

        if is_local {
            // handling the message from the local application
            let span = tracing::span!(
                tracing::Level::DEBUG,
                "handle_local_message",
                instance_id = %INSTANCE_ID.as_str(),
                connection_id = conn_index,
                message_type = message_type_to_str(&msg),
                telemetry = true
            );
            let _guard = span.enter();

            inject_current_context(&mut msg);
        } else {
            // handling the message from a remote gateway
            let parent_context = extract_parent_context(&msg);

            let span = tracing::span!(
                tracing::Level::DEBUG,
                "handle_remote_message",
                instance_id = %INSTANCE_ID.as_str(),
                connection_id = conn_index,
                message_type = message_type_to_str(&msg),
                telemetry = true
            );

            if let Some(ctx) = parent_context {
                span.set_parent(ctx);
            }
            let _guard = span.enter();

            inject_current_context(&mut msg);
        }

        match self.process_message(msg, conn_index).await {
            Ok(_) => Ok(()),
            Err(e) => {
                // drop message and log
                error!(
                    "error processing message from connection {:?}: {:?}",
                    conn_index, e
                );
                info!(
                    telemetry = true,
                    monotonic_counter.num_message_process_errors = 1
                );
                Err(DataPathError::ProcessingError(e.to_string()))
            }
        }
    }

    async fn send_error_to_local_app(&self, conn_index: u64, err: DataPathError) {
        let connection = self.forwarder().get_connection(conn_index);
        match connection {
            Some(conn) => {
                debug!("try to notify the error to the local application");
                let err_msg = create_error_publication(err.to_string());
                if let Channel::Server(tx) = conn.channel() {
                    if tx.send(Ok(err_msg)).await.is_err() {
                        debug!("unable to notify the error to the local app");
                    }
                }
            }
            None => {
                error!("connection {:?} not found", conn_index);
            }
        }
    }

    async fn reconnect(&self, client_conf: Option<ClientConfig>, conn_index: u64) -> bool {
        let config = client_conf.unwrap();
        match config.to_channel() {
            Err(e) => {
                error!(
                    "cannot parse connection config, unable to reconnect {:?}",
                    e.to_string()
                );
                false
            }
            Ok(channel) => {
                info!("connection lost with remote endpoint, try to reconnect");
                // These are the subscriptions that we forwarded to the remote gateway on
                // this connection. It is necessary to restore them to keep receive the messages
                // The connections on the local subscription table (created using the set_route command)
                // are still there and will be removed only if the reconnection process will fail.
                let remote_subscriptions = self
                    .forwarder()
                    .get_subscriptions_forwarded_on_connection(conn_index);

                match self
                    .try_to_connect(channel, Some(config), None, None, Some(conn_index), 120)
                    .await
                {
                    Ok(_) => {
                        info!("connection re-established");
                        // the subscription table should be ok already
                        for r in remote_subscriptions.iter() {
                            let header = create_agp_header(
                                r.source(),
                                r.name().agent_type(),
                                r.name().agent_id_option(),
                                None,
                                None,
                                None,
                                None,
                            );
                            let sub_msg = create_subscription(header, HashMap::new());
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

    fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
        conn_index: u64,
        client_config: Option<ClientConfig>,
        cancellation_token: CancellationToken,
        is_local: bool,
    ) -> tokio::task::JoinHandle<()> {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token_clone = cancellation_token.clone();
        let client_conf_clone = client_config.clone();
        let handle = tokio::spawn(async move {
            let mut try_to_reconnect = true;
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
                                        if let Err(e) = self_clone.handle_new_message(conn_index, is_local, msg).await {
                                            error!("error processing incoming messages {:?}", e);
                                            // If the message is coming from a local app, notify it
                                            if is_local {
                                                self_clone.send_error_to_local_app(conn_index, e).await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if let Some(io_err) = MessageProcessor::match_for_io_error(&e) {
                                            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                                                info!("connection {:?} closed by peer", conn_index);
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
                        info!("shutting down stream on drain: {}", conn_index);
                        try_to_reconnect = false;
                        break;
                    }
                    _ = token_clone.cancelled() => {
                        info!("shutting down stream cancellation token: {}", conn_index);
                        try_to_reconnect = false;
                        break;
                    }
                }
            }

            let mut connected = false;

            if try_to_reconnect && client_conf_clone.is_some() {
                connected = self_clone.reconnect(client_conf_clone, conn_index).await;
            } else {
                info!("close connection {}", conn_index)
            }

            if !connected {
                // delete connection state
                self_clone
                    .forwarder()
                    .on_connection_drop(conn_index, is_local);

                info!(telemetry = true, counter.num_active_connections = -1);
            }
        });

        handle
    }

    fn match_for_io_error(err_status: &Status) -> Option<&std::io::Error> {
        let mut err: &(dyn std::error::Error + 'static) = err_status;

        loop {
            if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
                return Some(io_err);
            }

            // h2::Error do not expose std::io::Error with `source()`
            // https://github.com/hyperium/h2/pull/462
            if let Some(h2_err) = err.downcast_ref::<h2::Error>() {
                if let Some(io_err) = h2_err.get_io() {
                    return Some(io_err);
                }
            }

            err = err.source()?;
        }
    }
}

#[tonic::async_trait]
impl PubSubService for MessageProcessor {
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

        info!(
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

        self.process_stream(stream, conn_index, None, CancellationToken::new(), false);

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}

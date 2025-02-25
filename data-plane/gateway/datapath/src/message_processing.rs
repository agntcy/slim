// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tonic::codegen::{Body, StdError};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, trace};

use crate::connection::{Channel, Connection, Type as ConnectionType};
use crate::errors::DataPathError;
use crate::forwarder::Forwarder;
use crate::messages::utils::{
    add_incoming_connection, get_agent_id, get_fanout, process_name, CommandType,
};
use crate::messages::AgentClass;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Publish as PublishType;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Subscribe as SubscribeType;
use crate::pubsub::proto::pubsub::v1::message::MessageType::Unsubscribe as UnsubscribeType;
use crate::pubsub::proto::pubsub::v1::pub_sub_service_client::PubSubServiceClient;
use crate::pubsub::proto::pubsub::v1::{pub_sub_service_server::PubSubService, Message};

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

    pub async fn connect<C>(
        &self,
        channel: C,
        local: Option<SocketAddr>,
        remote: Option<SocketAddr>,
    ) -> Result<(tokio::task::JoinHandle<()>, CancellationToken, u64), DataPathError>
    where
        C: tonic::client::GrpcService<tonic::body::BoxBody>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        let mut client = PubSubServiceClient::new(channel);
        let (tx, rx) = mpsc::channel(128);
        let stream = client
            .open_channel(Request::new(ReceiverStream::new(rx)))
            .await
            .map_err(|e| DataPathError::ConnectionError(e.to_string()))?
            .into_inner();

        let connection = Connection::new(ConnectionType::Remote)
            .with_local_addr(local)
            .with_remote_addr(remote)
            .with_channel(Channel::Client(tx));

        info!(
            "new connection initiated locally: (remote: {:?} - local: {:?})",
            connection.remote_addr(),
            connection.local_addr()
        );

        // insert connection into connection table
        let conn_index = self.forwarder().on_connection_established(connection);

        // Start loop to process messages
        let ret = self.process_stream(stream, conn_index, false);
        Ok((ret.0, ret.1, conn_index))
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
        let conn_id = self.forwarder().on_connection_established(connection);

        debug!("local connection established with id: {:?}", conn_id);

        // this loop will process messages from the local app
        self.process_stream(ReceiverStream::new(rx1), conn_id, true);

        // return the handles to be used to send and receive messages
        (tx1, rx2)
    }

    pub async fn send_msg(
        &self,
        msg: Message,
        out_conn: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let connection = self.forwarder().get_connection(out_conn);
        match connection {
            Some(conn) => match conn.channel() {
                Channel::Server(s) => s.send(Ok(msg)).await?,
                Channel::Client(s) => s.send(msg).await?,
                _ => error!("error reading channel"),
            },
            None => error!("connection {:?} not found", out_conn),
        }
        Ok(())
    }

    async fn match_and_forward_msg(
        &self,
        msg: Message,
        class: AgentClass,
        in_connection: u64,
        fanout: u32,
        agent_id: Option<u64>,
    ) -> Result<(), DataPathError> {
        debug!(
            "match and forward message: class: {:?} - agent_id: {:?} - fanout: {:?}",
            class, agent_id, fanout,
        );

        if fanout == 1 {
            match self
                .forwarder()
                .on_publish_msg_match_one(class, agent_id, in_connection)
            {
                Ok(out) => match self.send_msg(msg, out).await {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        error!("error sending a message {:?}", e);
                        Err(DataPathError::PublicationError(e.to_string()))
                    }
                },
                Err(e) => {
                    error!("error matching a message {:?}", e);
                    Err(DataPathError::PublicationError(e.to_string()))
                }
            }
        } else {
            match self
                .forwarder()
                .on_publish_msg_match_all(class, agent_id, in_connection)
            {
                Ok(out_set) => {
                    for out in out_set {
                        match self.send_msg(msg.clone(), out).await {
                            Ok(_) => {}
                            Err(e) => {
                                error!("error sending a message {:?}", e);
                                return Err(DataPathError::PublicationError(e.to_string()));
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) => {
                    error!("error sending a message {:?}", e);
                    Err(DataPathError::PublicationError(e.to_string()))
                }
            }
        }
    }

    async fn process_publish(
        &self,
        mut msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        let pubmsg = match &msg.message_type {
            Some(PublishType(p)) => p,
            // this should never happen
            _ => panic!("wrong message type"),
        };

        match process_name(&pubmsg.name) {
            Ok(class) => {
                let fanout = get_fanout(pubmsg);
                let agent_id = get_agent_id(&pubmsg.name);

                debug!(
                    "received publication from connection {}: {:?}",
                    in_connection, pubmsg
                );

                // add incoming connection to the metadata
                add_incoming_connection(&mut msg, in_connection);

                // if we get valid class also the name is valid so we can safely unwrap
                return self
                    .match_and_forward_msg(msg, class, in_connection, fanout, agent_id)
                    .await;
            }
            Err(e) => {
                error!("error processing publication message {:?}", e);
                Err(DataPathError::PublicationError(e.to_string()))
            }
        }
    }

    fn process_command(&self, msg: &Message) -> Result<(CommandType, u64), DataPathError> {
        if !msg.metadata.is_empty() {
            match msg.metadata.get(&CommandType::ReceivedFrom.to_string()) {
                None => {}
                Some(out_str) => match out_str.parse::<u64>() {
                    Err(e) => {
                        error! {"error parsing the connection in command type ReceivedFrom: {:?}", e};
                        return Err(DataPathError::CommandError(e.to_string()));
                    }
                    Ok(out) => {
                        debug!(%out, "received subscription_from command, register subscription");
                        return Ok((CommandType::ReceivedFrom, out));
                    }
                },
            }
            match msg.metadata.get(&CommandType::ForwardTo.to_string()) {
                None => {}
                Some(out_str) => match out_str.parse::<u64>() {
                    Err(e) => {
                        error! {"error parsing the connection in command type ForwardTo: {:?}", e};
                        return Err(DataPathError::CommandError(e.to_string()));
                    }
                    Ok(out) => {
                        debug!(%out, "received forward_to command, register subscription and forward");
                        return Ok((CommandType::ForwardTo, out));
                    }
                },
            }
        }
        Ok((CommandType::Unknown, 0))
    }

    async fn process_unsubscription(
        &self,
        mut msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        let unsubmsg = match &msg.message_type {
            Some(UnsubscribeType(s)) => s,
            // this should never happen
            _ => panic!("wrong message type"),
        };

        match process_name(&unsubmsg.name) {
            Ok(class) => {
                // process command
                let command = self.process_command(&msg);
                let mut conn = in_connection;
                let mut forward = false;
                // only used if the subscription needs to be forwarded
                let mut out_conn = in_connection;
                match command {
                    Err(e) => {
                        return Err(e);
                    }
                    Ok(tuple) => match tuple.0 {
                        CommandType::ReceivedFrom => {
                            conn = tuple.1;
                        }
                        CommandType::ForwardTo => {
                            forward = true;
                            out_conn = tuple.1;
                        }
                        _ => {}
                    },
                }
                let connection = self.forwarder().get_connection(in_connection);
                if connection.is_none() {
                    // this should never happen
                    error!("incoming connection does not exists");
                    return Err(DataPathError::SubscriptionError(
                        "incoming connection does not exists".to_string(),
                    ));
                }
                match self.forwarder().on_unsubscription_msg(
                    class,
                    get_agent_id(&unsubmsg.name),
                    conn,
                    connection.unwrap().is_local_connection(),
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(DataPathError::UnsubscriptionError(e.to_string()));
                    }
                }
                if forward {
                    debug!("forward subscription to {:?}", out_conn);
                    msg.metadata.clear();
                    match self.send_msg(msg, out_conn).await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("error sending a message {:?}", e);
                            return Err(DataPathError::SubscriptionError(e.to_string()));
                        }
                    };
                }
                Ok(())
            }
            Err(e) => {
                error!("error processing unsubscription message {:?}", e);
                Err(DataPathError::UnsubscriptionError(e.to_string()))
            }
        }
    }

    async fn process_subscription(
        &self,
        mut msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        let submsg = match &msg.message_type {
            Some(SubscribeType(s)) => s,
            // this should never happen
            _ => panic!("wrong message type"),
        };

        debug!(
            "received subscription from connection {}: {:?}",
            in_connection, submsg
        );

        match process_name(&submsg.name) {
            Ok(class) => {
                // process command
                trace!("process command");
                let command = self.process_command(&msg);
                let mut conn = in_connection;
                let mut forward = false;

                // only used if the subscription needs to be forwarded
                let mut out_conn = in_connection;
                match command {
                    Err(e) => {
                        return Err(e);
                    }
                    Ok(tuple) => match tuple.0 {
                        CommandType::ReceivedFrom => {
                            conn = tuple.1;
                            trace!("received subscription_from command, register subscription with conn id {:?}", tuple.1);
                        }
                        CommandType::ForwardTo => {
                            forward = true;
                            out_conn = tuple.1;
                            trace!("received forward_to command, register subscription and forward to conn id {:?}", out_conn);
                        }
                        _ => {}
                    },
                }
                let connection = self.forwarder().get_connection(in_connection);
                if connection.is_none() {
                    // this should never happen
                    error!("incoming connection does not exists");
                    return Err(DataPathError::SubscriptionError(
                        "incoming connection does not exists".to_string(),
                    ));
                }
                match self.forwarder().on_subscription_msg(
                    class,
                    get_agent_id(&submsg.name),
                    conn,
                    connection.unwrap().is_local_connection(),
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(DataPathError::SubscriptionError(e.to_string()));
                    }
                }

                if forward {
                    debug!("forward subscription {:?} to {:?}", msg, out_conn);
                    msg.metadata.clear();
                    match self.send_msg(msg, out_conn).await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("error sending a message {:?}", e);
                            return Err(DataPathError::SubscriptionError(e.to_string()));
                        }
                    };
                }
                Ok(())
            }
            Err(e) => {
                error!("error processing subscription message {:?}", e);
                Err(DataPathError::SubscriptionError(e.to_string()))
            }
        }
    }

    pub async fn process_message(
        &self,
        msg: Message,
        in_connection: u64,
    ) -> Result<(), DataPathError> {
        match &msg.message_type {
            None => {
                error!(
                    "received message without message type from connection {}: {:?}",
                    in_connection, msg
                );
                info!(monotonic_counter.num_messages_by_type = 1, method = "none");
                Err(DataPathError::UnknownMsgType("".to_string()))
            }
            Some(msg_type) => match msg_type {
                SubscribeType(s) => {
                    debug!(
                        "received subscription from connection {}: {:?}",
                        in_connection, s
                    );
                    info!(monotonic_counter.num_messages_by_type = 1, method = "subscribe");
                    match self.process_subscription(msg, in_connection).await {
                        Err(e) => {
                            error! {"error processing subscription {:?}", e}
                            Err(e)
                        }
                        Ok(_) => Ok(()),
                    }
                }
                UnsubscribeType(u) => {
                    debug!(
                        "Received ubsubscription from client {}: {:?}",
                        in_connection, u
                    );
                    info!(monotonic_counter.num_messages_by_type = 1, method = "unsubscribe");
                    match self.process_unsubscription(msg, in_connection).await {
                        Err(e) => {
                            error! {"error processing unsubscription {:?}", e}
                            Err(e)
                        }
                        Ok(_) => Ok(()),
                    }
                }
                PublishType(p) => {
                    debug!("Received publish from client {}: {:?}", in_connection, p);
                    info!(monotonic_counter.num_messages_by_type = 1, method = "publish");
                    match self.process_publish(msg, in_connection).await {
                        Err(e) => {
                            error! {"error processing publication {:?}", e}
                            Err(e)
                        }
                        Ok(_) => Ok(()),
                    }
                }
            },
        }
    }

    async fn handle_new_message(
        &self,
        conn_index: u64,
        result: Result<Message, Status>,
    ) -> Result<(), DataPathError> {
        debug!(%conn_index, "Received message from connection");
        info!(monotonic_counter.num_processed_messages = 1);

        match result {
            Ok(msg) => {
                match self.process_message(msg, conn_index).await {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        // drop message and log
                        error!(
                            "error processing message from connection {:?}: {:?}",
                            conn_index, e
                        );
                        info!(monotonic_counter.num_message_process_errors = 1);
                        Ok(())
                    }
                }
            }
            Err(e) => {
                if let Some(io_err) = MessageProcessor::match_for_io_error(&e) {
                    if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                        info!("Connection {:?} closed by peer", conn_index);
                        return Err(DataPathError::StreamError(e.to_string()));
                    }
                }
                error!("error receiving messages {:?}", e);
                let connection = self.forwarder().get_connection(conn_index);
                match connection {
                    Some(conn) => {
                        match conn.channel() {
                            Channel::Server(tx) => tx
                                .send(Err(e))
                                .await
                                .map_err(|e| DataPathError::MessageSendError(e.to_string())),
                            _ => Err(DataPathError::WrongChannelType), // error
                        }
                    }
                    None => {
                        error!("connection {:?} not found", conn_index);
                        Err(DataPathError::ConnectionNotFound(conn_index.to_string()))
                    }
                }
            }
        }
    }

    fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<Message, Status>> + Unpin + Send + 'static,
        conn_index: u64,
        is_local: bool,
    ) -> (tokio::task::JoinHandle<()>, CancellationToken) {
        // Clone self to be able to move it into the spawned task
        let self_clone = self.clone();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    res = stream.next() => {
                        match res {
                            Some(msg) => {
                                if let Err(e) = self_clone.handle_new_message(conn_index, msg).await {
                                    error!("error handling stream {:?}", e);
                                    break;
                                }
                            }
                            None => {
                                info!(%conn_index, "end of stream");
                                break;
                            }
                        }
                    }
                    _ = self_clone.get_drain_watch().signaled() => {
                        info!("shutting down stream on drain: {}", conn_index);
                        break;
                    }
                    _ = token_clone.cancelled() => {
                        info!("shutting down stream cancellation token: {}", conn_index);
                        break;
                    }
                }
            }

            self_clone
                .forwarder()
                .on_connection_drop(conn_index, is_local);
        });

        (handle, token)
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
        // TODO(zkacsand): decrease on disconnect
        info!(counter.num_active_connections = 1);

        // insert connection into connection table
        let conn_index = self.forwarder().on_connection_established(connection);

        self.process_stream(stream, conn_index, false);

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenChannelStream
        ))
    }
}

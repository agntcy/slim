// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::net::SocketAddr;
use std::{pin::Pin, sync::Arc};

use agp_config::grpc::client::ClientConfig;
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
    add_incoming_connection, create_subscription, get_agent_id, get_fanout, process_name,
    CommandType,
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
                            "rror adding connection to the connection tables".to_string(),
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
                        // here token cancell will stop the receiving loop on
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
                let agent_id = get_agent_id(&unsubmsg.name);
                match self.forwarder().on_unsubscription_msg(
                    class.clone(),
                    agent_id,
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
                        Ok(_) => {
                            self.forwarder()
                                .on_forwarded_unsubscription(class, agent_id, conn);
                        }
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

        info!(
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
                            info!("received subscription_from command, register subscription with conn id {:?}", tuple.1);
                        }
                        CommandType::ForwardTo => {
                            forward = true;
                            out_conn = tuple.1;
                            info!("received forward_to command, register subscription and forward to conn id {:?}", out_conn);
                        }
                        _ => {}
                    },
                }

                let connection = self.forwarder().get_connection(conn);
                if connection.is_none() {
                    // this should never happen
                    error!("incoming connection does not exists");
                    return Err(DataPathError::SubscriptionError(
                        "incoming connection does not exists".to_string(),
                    ));
                }
                let agent_id = get_agent_id(&submsg.name);
                match self.forwarder().on_subscription_msg(
                    class.clone(),
                    agent_id,
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
                        Ok(_) => {
                            self.forwarder()
                                .on_forwarded_subscription(class, agent_id, out_conn);
                        }
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
                Err(DataPathError::UnknownMsgType("".to_string()))
            }
            Some(msg_type) => match msg_type {
                SubscribeType(s) => {
                    debug!(
                        "received subscription from connection {}: {:?}",
                        in_connection, s
                    );
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

            let mut delete_connection = true;

            info!(
                "SUB TABLE: {}",
                self_clone.forwarder().print_subscription_table()
            );

            if try_to_reconnect && client_conf_clone.is_some() {
                let config = client_conf_clone.unwrap();
                match config.to_channel() {
                    Err(e) => {
                        error!(
                            "cannot parse connection config, unable to reconnect {:?}",
                            e.to_string()
                        );
                    }
                    Ok(channel) => {
                        info!("connection lost with remote endpoint, try to reconnect");
                        // this are the subscriptions set for the connections that we are removing
                        // subscriptions is the list of subscriptions set with set_route
                        // remote_subscriptions is the list of subscriptions forwarded to the next node
                        let remote_subscriptions = self_clone
                            .forwarder()
                            .get_subscriptions_forwarded_on_connection(conn_index);

                        // 1. try to reconnect
                        // 2. if fails
                        //      2.a remove connection
                        //      2.b notify the application (next sprint)
                        // 3. if it is restored
                        //      3.a send the remote connections

                        info!(
                            "SUB TABLE: {}",
                            self_clone.forwarder().print_subscription_table()
                        );

                        match self_clone
                            .try_to_connect(
                                channel,
                                Some(config),
                                None,
                                None,
                                Some(conn_index),
                                120,
                            )
                            .await
                        {
                            Ok(_) => {
                                info!("connection re-established");
                                // the subscription table should be ok already
                                delete_connection = false;
                                for r in remote_subscriptions.iter() {
                                    // TODO in remote subscription put also the source name of the subscription
                                    // so that I can reuse it here
                                    let sub_msg = create_subscription(
                                        r,
                                        &r.agent_class,
                                        Some(r.agent_id),
                                        HashMap::new(),
                                    );
                                    if self_clone.send_msg(sub_msg, conn_index).await.is_err() {
                                        error!("error restoring subscription on remote node");
                                    }
                                }
                            }
                            Err(e) => {
                                error!("unable to connect to remote node {:?}", e.to_string());
                            }
                        }
                    }
                }
            } else {
                info!(
                    "DO NOT RECONNECT BECUASE try_reconnect {}, client_config {}",
                    try_to_reconnect,
                    client_conf_clone.is_some()
                )
            }

            if delete_connection {
                self_clone
                    .forwarder()
                    .on_connection_drop(conn_index, is_local);
            }

            info!(
                "SUB TABLE AFTER: {}",
                self_clone.forwarder().print_subscription_table()
            );
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

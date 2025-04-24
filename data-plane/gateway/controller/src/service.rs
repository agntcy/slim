// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::codegen::{Body, StdError};
use tonic::{Request, Response, Status};
use tracing::{debug, error};

use crate::api::proto::api::v1::{
    Ack, ControlMessage, controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::errors::ControllerError;

use agp_config::grpc::client::ClientConfig;
use agp_datapath::message_processing::MessageProcessor;
use agp_datapath::messages::utils::AgpHeaderFlags;
use agp_datapath::messages::{Agent, AgentType};
use agp_datapath::pubsub::proto::pubsub::v1::Message as PubsubMessage;

#[derive(Debug, Clone)]
pub struct ControllerService {
    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// channel to send messages into the datapath
    tx_gw: mpsc::Sender<Result<PubsubMessage, Status>>,

    /// map of connection IDs to their configuration
    connections: Arc<parking_lot::RwLock<HashMap<String, u64>>>,
}

impl ControllerService {
    pub fn new(message_processor: Arc<MessageProcessor>) -> Self {
        let (_, tx_gw, _) = message_processor.register_local_connection();

        ControllerService {
            message_processor: message_processor,
            tx_gw,
            connections: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    async fn handle_new_message(
        &self,
        msg: ControlMessage,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> Result<(), ControllerError> {
        match msg.payload {
            Some(ref payload) => {
                match payload {
                    crate::api::proto::api::v1::control_message::Payload::ConfigCommand(config) => {
                        for conn in &config.connections_to_create {
                            // connect to an endpoint if it's not already connected
                            let client_endpoint =
                                format!("{}:{}", conn.remote_address, conn.remote_port);

                            if !self.connections.read().contains_key(&client_endpoint) {
                                let client_config = ClientConfig {
                                    endpoint: client_endpoint,
                                    ..ClientConfig::default()
                                };

                                match client_config.to_channel() {
                                    Err(e) => {
                                        error!("error reading channel config {:?}", e);
                                        //Err(ControllerError::ConfigError(e.to_string()))
                                    }
                                    Ok(channel) => {
                                        let ret = self
                                            .message_processor
                                            .connect(
                                                channel,
                                                Some(client_config.clone()),
                                                None,
                                                None,
                                            )
                                            .await
                                            .map_err(|e| {
                                                ControllerError::ConnectionError(e.to_string())
                                            });

                                        let conn_id = match ret {
                                            Err(e) => {
                                                error!("connection error: {:?}", e);
                                                return Err(ControllerError::ConnectionError(
                                                    e.to_string(),
                                                ));
                                            }
                                            Ok(conn_id) => conn_id.1,
                                        };

                                        self.connections
                                            .write()
                                            .insert(client_config.endpoint.clone(), conn_id);
                                    }
                                }
                            }
                        }

                        for route in &config.routes_to_set {
                            if !self.connections.read().contains_key(&route.connection_id) {
                                error!("connection {} not found", route.connection_id);
                                continue;
                            }

                            // TODO: handle error if it fails
                            let conn = self
                                .connections
                                .read()
                                .get(&route.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                "gateway",
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                "gateway",
                            );

                            let msg = PubsubMessage::new_subscribe(
                                &source,
                                &agent_type,
                                route.agent_id,
                                Some(AgpHeaderFlags::default().with_recv_from(conn)),
                            );

                            if let Err(e) = self.send_message(msg).await {
                                error!("failed to subscribe: {}", e);
                            }
                        }

                        for route in &config.routes_to_delete {
                            if !self.connections.read().contains_key(&route.connection_id) {
                                error!("connection {} not found", route.connection_id);
                                continue;
                            }

                            // TODO: handle error if it fails
                            let conn = self
                                .connections
                                .read()
                                .get(&route.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                "gateway",
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                "gateway",
                            );

                            let msg = PubsubMessage::new_unsubscribe(
                                &source,
                                &agent_type,
                                route.agent_id,
                                Some(AgpHeaderFlags::default().with_recv_from(conn)),
                            );

                            if let Err(e) = self.send_message(msg).await {
                                error!("failed to unsubscribe: {}", e);
                            }
                        }

                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            //TODO: add (error) message
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(
                                crate::api::proto::api::v1::control_message::Payload::Ack(ack),
                            ),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            eprintln!("failed to send ACK: {}", e);
                        }
                    }
                    crate::api::proto::api::v1::control_message::Payload::Ack(_ack) => {
                        // received an ack, do nothing - this should not happen
                    }
                }
            }
            None => {
                println!(
                    "received control message {} with no payload",
                    msg.message_id
                );
            }
        }

        Ok(())
    }

    async fn send_message(&self, msg: PubsubMessage) -> Result<(), ControllerError> {
        self.tx_gw.send(Ok(msg)).await.map_err(|e| {
            error!("error sending message into datapath: {}", e);
            ControllerError::DatapathError(e.to_string())
        })
    }

    async fn process_stream(
        &self,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> tokio::task::JoinHandle<()> {
        let svc = self.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(result) => {
                                match result {
                                    Ok(msg) => {
                                        if let Err(_e) = svc.handle_new_message(msg, tx.clone()).await {
                                            //TODO: handle error
                                        }
                                    }
                                    Err(_e) => {
                                        //TODO: handle error
                                    }
                                }
                            }
                            None => {
                                debug!("end of stream");
                                break;
                            }
                        }
                    }
                }
            }
        });

        handle
    }

    pub async fn connect<C>(
        &self,
        channel: C,
    ) -> Result<tokio::task::JoinHandle<()>, ControllerError>
    where
        C: tonic::client::GrpcService<tonic::body::Body>,
        C::Error: Into<StdError>,
        C::ResponseBody: Body<Data = bytes::Bytes> + std::marker::Send + 'static,
        <C::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        let mut client: ControllerServiceClient<C> = ControllerServiceClient::new(channel);

        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
        let out_stream = ReceiverStream::new(rx).map(|res| res.expect("mapping error"));

        match client.open_control_channel(Request::new(out_stream)).await {
            Ok(stream) => {
                let ret = self.process_stream(stream.into_inner(), tx).await;
                return Ok(ret);
            }
            Err(_) => Err(ControllerError::ConnectionError(
                "reached max connection retries".to_string(),
            ))
        }
    }
}

#[tonic::async_trait]
impl GrpcControllerService for ControllerService {
    type OpenControlChannelStream =
        Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send + 'static>>;

    async fn open_control_channel(
        &self,
        request: Request<tonic::Streaming<ControlMessage>>,
    ) -> Result<Response<Self::OpenControlChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);

        self.process_stream(stream, tx.clone()).await;

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenControlChannelStream
        ))
    }
}

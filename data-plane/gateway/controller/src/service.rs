// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use agp_config::tls::client::TlsClientConfig;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::codegen::{Body, StdError};
use tonic::{Request, Response, Status};
use tracing::{debug, info, error};

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

// "fake" agent type for comminicating with the message processing layer
pub const AGENT_TYPE: &str = "gateway";

#[derive(Debug, Clone)]
pub struct ControllerService {
    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// channel to send messages into the datapath
    tx_gw: OnceLock<mpsc::Sender<Result<PubsubMessage, Status>>>,

    /// map of connection IDs to their configuration
    connections: Arc<parking_lot::RwLock<HashMap<String, u64>>>,
}

impl ControllerService {
    pub fn new(message_processor: Arc<MessageProcessor>) -> Self {
        ControllerService {
            message_processor,
            tx_gw: OnceLock::new(),
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
                            let client_endpoint =
                                format!("{}:{}", conn.remote_address, conn.remote_port);

                            // connect to an endpoint if it's not already connected
                            if !self.connections.read().contains_key(&client_endpoint) {
                                let client_config = ClientConfig {
                                    endpoint: format!("http://{}", client_endpoint),
                                    tls_setting: TlsClientConfig::default().with_insecure(true),
                                    ..ClientConfig::default()
                                };

                                match client_config.to_channel() {
                                    Err(e) => {
                                        error!("error reading channel config {:?}", e);
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

                                        self.connections.write().insert(client_endpoint, conn_id);
                                    }
                                }
                            }
                        }

                        for route in &config.routes_to_set {
                            if !self.connections.read().contains_key(&route.connection_id) {
                                error!("connection {} not found", route.connection_id);
                                continue;
                            }

                            let conn = self
                                .connections
                                .read()
                                .get(&route.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                &route.agent_name.as_str(),
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                &route.agent_name.as_str(),
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

                            let conn = self
                                .connections
                                .read()
                                .get(&route.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                AGENT_TYPE,
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                route.company.as_str(),
                                route.namespace.as_str(),
                                AGENT_TYPE,
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
                            messages: vec![],
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
        let sender = self.tx_gw.get_or_init(|| {
            let (_, tx_gw, _) = self.message_processor.register_local_connection();
            tx_gw
        });

        sender.send(Ok(msg)).await.map_err(|e| {
            error!("error sending message into datapath: {}", e);
            ControllerError::DatapathError(e.to_string())
        })
    }

    async fn process_stream(
        &self,
        cancellation_token: CancellationToken,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> tokio::task::JoinHandle<()> {
        let svc = self.clone();
        let token = cancellation_token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(Ok(msg)) => {
                                if let Err(e) = svc.handle_new_message(msg, tx.clone()).await {
                                    error!("error processing incoming control message: {:?}", e);
                                }
                            }
                            Some(Err(e)) => {
                                if let Some(io_err) = ControllerService::match_for_io_error(&e) {
                                    if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                                        info!("connection closed by peer");
                                    }
                                } else {
                                    error!("error receiving control messages: {:?}", e);
                                }
                                break;
                            }
                            None => {
                                debug!("end of stream");
                                break;
                            }
                        }
                    }
                    _ = token.cancelled() => {
                        debug!("shutting down stream on cancellation token");
                        break;
                    }
                }
            }
        })
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
        //TODO(zkacsand): make this a constant or make it configurable?
        let max_retry = 10;

        let mut client: ControllerServiceClient<C> = ControllerServiceClient::new(channel);
        let mut i = 0;
        while i < max_retry {
            let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
            let out_stream = ReceiverStream::new(rx).map(|res| res.expect("mapping error"));

            match client.open_control_channel(Request::new(out_stream)).await {
                Ok(stream) => {
                    let ret = self
                        .process_stream(CancellationToken::new(), stream.into_inner(), tx)
                        .await;
                    return Ok(ret);
                }
                Err(e) => {
                    error!("connection error: {:?}.", e.to_string());
                }
            };

            i += 1;

            // sleep 1 sec between each connection retry
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        error!("unable to connect to the endpoint");
        Err(ControllerError::ConnectionError(
            "reached max connection retries".to_string(),
        ))
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
impl GrpcControllerService for ControllerService {
    type OpenControlChannelStream =
        Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send + 'static>>;

    async fn open_control_channel(
        &self,
        request: Request<tonic::Streaming<ControlMessage>>,
    ) -> Result<Response<Self::OpenControlChannelStream>, Status> {
        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);

        self.process_stream(CancellationToken::new(), stream, tx.clone())
            .await;

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenControlChannelStream
        ))
    }
}

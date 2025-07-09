// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::codegen::{Body, StdError};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::api::proto::api::v1::{
    Ack, ConnectionEntry, ControlMessage, SubscriptionEntry,
    controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::api::proto::api::v1::{
    ConnectionListResponse, ConnectionType, SubscriptionListResponse,
};
use crate::errors::ControllerError;
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::ProtoMessage as PubsubMessage;
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_datapath::messages::{Agent, AgentType};
use slim_datapath::tables::SubscriptionTable;

#[derive(Debug, Clone)]
pub struct ControllerService {
    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// channel to send messages into the datapath
    tx_slim: OnceLock<mpsc::Sender<Result<PubsubMessage, Status>>>,

    /// map of connection IDs to their configuration
    connections: Arc<parking_lot::RwLock<HashMap<String, u64>>>,
}

impl ControllerService {
    pub fn new(message_processor: Arc<MessageProcessor>) -> Self {
        ControllerService {
            message_processor,
            tx_slim: OnceLock::new(),
            connections: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    async fn handle_new_control_message(
        &self,
        msg: ControlMessage,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> Result<(), ControllerError> {
        match msg.payload {
            Some(ref payload) => {
                match payload {
                    crate::api::proto::api::v1::control_message::Payload::ConfigCommand(config) => {
                        for conn in &config.connections_to_create {
                            let client_config = serde_json::from_str::<ClientConfig>(&conn.config_data).map_err(
                                    |e| ControllerError::ConfigError(e.to_string())
                            )?;
                            let client_endpoint = &client_config.endpoint;

                            // connect to an endpoint if it's not already connected
                            if !self.connections.read().contains_key(client_endpoint) {
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

                                        self.connections.write().insert(client_endpoint.clone(), conn_id);
                                    }
                                }
                            }
                        }

                        for subscription in &config.subscriptions_to_set {
                            if !self.connections.read().contains_key(&subscription.connection_id) {
                                error!("connection {} not found", subscription.connection_id);
                                continue;
                            }

                            let conn = self
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                subscription.organization.as_str(),
                                subscription.namespace.as_str(),
                                subscription.agent_type.as_str(),
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                subscription.organization.as_str(),
                                subscription.namespace.as_str(),
                                subscription.agent_type.as_str(),
                            );

                            let msg = PubsubMessage::new_subscribe(
                                &source,
                                &agent_type,
                                subscription.agent_id,
                                Some(SlimHeaderFlags::default().with_recv_from(conn)),
                            );

                            if let Err(e) = self.send_control_message(msg).await {
                                error!("failed to subscribe: {}", e);
                            }
                        }

                        for subscription in &config.subscriptions_to_delete {
                            if !self.connections.read().contains_key(&subscription.connection_id) {
                                error!("connection {} not found", subscription.connection_id);
                                continue;
                            }

                            let conn = self
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Agent::from_strings(
                                subscription.organization.as_str(),
                                subscription.namespace.as_str(),
                                subscription.agent_type.as_str(),
                                0,
                            );
                            let agent_type = AgentType::from_strings(
                                subscription.organization.as_str(),
                                subscription.namespace.as_str(),
                                subscription.agent_type.as_str(),
                            );

                            let msg = PubsubMessage::new_unsubscribe(
                                &source,
                                &agent_type,
                                subscription.agent_id,
                                Some(SlimHeaderFlags::default().with_recv_from(conn)),
                            );

                            if let Err(e) = self.send_control_message(msg).await {
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
                    crate::api::proto::api::v1::control_message::Payload::SubscriptionListRequest(_) => {
                        const CHUNK_SIZE: usize = 100;

                        let conn_table = self.message_processor.connection_table();
                        let mut entries = Vec::new();

                        self
                            .message_processor
                            .subscription_table()
                            .for_each(|agent_type, agent_id, local, remote| {
                                let mut entry = SubscriptionEntry {
                                    organization: agent_type.organization_string().unwrap_or_else(|| agent_type.organization().to_string()),
                                    namespace: agent_type.namespace_string().unwrap_or_else(|| agent_type.organization().to_string()),
                                    agent_type: agent_type.agent_type_string().unwrap_or_else(|| agent_type.organization().to_string()),
                                    agent_id: Some(agent_id),
                                    ..Default::default()
                                };

                                for &cid in local {
                                    entry.local_connections.push(ConnectionEntry {
                                        id: cid,
                                        connection_type: ConnectionType::Local as i32,
                                        config_data: "{}".to_string(),
                                    });
                                }

                                for &cid in remote {
                                    if let Some(conn) = conn_table.get(cid as usize) {
                                            entry.remote_connections.push(ConnectionEntry {
                                                id: cid,
                                                connection_type: ConnectionType::Remote as i32,
                                                config_data: match conn.config_data() {
                                                    Some(data) => serde_json::to_string(data)
                                                        .unwrap_or_else(|_| "{}".to_string()),
                                                    None => "{}".to_string(),
                                                },
                                            });
                                    } else {
                                        error!("no connection entry for id {}", cid);
                                    }
                                }
                                entries.push(entry);
                            });

                        for chunk in entries.chunks(CHUNK_SIZE) {
                            let resp = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(
                                    crate::api::proto::api::v1::control_message::Payload::SubscriptionListResponse(
                                        SubscriptionListResponse {
                                            entries: chunk.to_vec(),
                                        }
                                    )
                                ),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!("failed to send subscription batch: {}", e);
                            }
                        }
                    }
                    crate::api::proto::api::v1::control_message::Payload::ConnectionListRequest(_) => {
                        let mut all_entries = Vec::new();
                        self.message_processor
                            .connection_table()
                            .for_each(|id, conn| {
                                all_entries.push(ConnectionEntry {
                                    id: id as u64,
                                    connection_type: ConnectionType::Remote as i32,
                                    config_data: match conn.config_data() {
                                        Some(data) => serde_json::to_string(data)
                                            .unwrap_or_else(|_| "{}".to_string()),
                                        None => "{}".to_string(),
                                    },
                                });
                            });

                        const CHUNK_SIZE: usize = 100;
                        for chunk in all_entries.chunks(CHUNK_SIZE) {
                            let resp = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(
                                    crate::api::proto::api::v1::control_message::Payload::ConnectionListResponse(
                                        ConnectionListResponse {
                                            entries: chunk.to_vec(),
                                        },
                                    ),
                                ),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!("failed to send connection list batch: {}", e);
                            }
                        }
                    }
                    crate::api::proto::api::v1::control_message::Payload::Ack(_ack) => {
                        // received an ack, do nothing - this should not happen
                    }
                    crate::api::proto::api::v1::control_message::Payload::SubscriptionListResponse(_) => {
                        // received a subscription list response, do nothing - this should not happen
                    }
                    crate::api::proto::api::v1::control_message::Payload::ConnectionListResponse(_) => {
                        // received a connection list response, do nothing - this should not happen
                    }
                    crate::api::proto::api::v1::control_message::Payload::RegisterNodeRequest(_) => {
                        panic!("this should not happen");
                    }
                    crate::api::proto::api::v1::control_message::Payload::RegisterNodeResponse(_) => {
                        // received a register node response, do nothing
                    }
                    crate::api::proto::api::v1::control_message::Payload::DeregisterNodeRequest(_) => {
                        panic!("this should not happen");
                    }                    
                    crate::api::proto::api::v1::control_message::Payload::DeregisterNodeResponse(_) => {
                        // received a deregister node response, do nothing
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

    async fn send_control_message(&self, msg: PubsubMessage) -> Result<(), ControllerError> {
        let sender = self.tx_slim.get_or_init(|| {
            let (_, tx_slim, _) = self.message_processor.register_local_connection();
            tx_slim
        });

        sender.send(Ok(msg)).await.map_err(|e| {
            error!("error sending message into datapath: {}", e);
            ControllerError::DatapathError(e.to_string())
        })
    }

    async fn process_control_message_stream(
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
                                if let Err(e) = svc.handle_new_control_message(msg, tx.clone()).await {
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
                        .process_control_message_stream(
                            CancellationToken::new(),
                            stream.into_inner(),
                            tx,
                        )
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

        self.process_control_message_stream(CancellationToken::new(), stream, tx.clone())
            .await;

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenControlChannelStream
        ))
    }
}

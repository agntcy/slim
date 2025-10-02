// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine;
use uuid::Uuid;

use slim_config::component::id::ID;
use slim_config::grpc::server::ServerConfig;
use slim_config::metadata::MetadataValue;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::api::proto::api::v1::control_message::Payload;
use crate::api::proto::api::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::api::v1::{
    self, ConnectionListResponse, ConnectionType, SubscriptionListResponse,
};
use crate::api::proto::api::v1::{
    Ack, ConnectionDetails, ConnectionEntry, ControlMessage, SubscriptionEntry,
    controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::errors::ControllerError;
use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::TokenProvider;
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::ProtoMessage as DataPlaneMessage;
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader};
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;
use slim_datapath::messages::encoder::calculate_hash;
use slim_datapath::messages::utils::{SLIM_IDENTITY, SlimHeaderFlags};
use slim_datapath::tables::SubscriptionTable;

use crate::api::proto::moderator::v1::{DeleteChannelRequest, ModeratorMessage};
use prost::Message;

type TxChannel = mpsc::Sender<Result<ControlMessage, Status>>;
type TxChannels = HashMap<String, TxChannel>;

/// The name used as the source for controller-originated messages.
pub static CONTROLLER_SOURCE_NAME: std::sync::LazyLock<slim_datapath::messages::Name> =
    std::sync::LazyLock::new(|| {
        slim_datapath::messages::Name::from_strings(["controller", "controller", "controller"])
            .with_id(0)
    });

/// Inner structure for the controller service
/// This structure holds the internal state of the controller service,
/// including the ID, message processor, connections, and channels.
/// It is normally wrapped in an Arc to allow shared ownership across multiple threads.
#[derive(Debug)]
struct ControllerServiceInternal {
    /// ID of this SLIM instance
    id: ID,

    /// optional group name
    group_name: Option<String>,

    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// map of connection IDs to their configuration
    connections: Arc<parking_lot::RwLock<HashMap<String, u64>>>,

    /// channel to send messages into the datapath
    tx_slim: mpsc::Sender<Result<DataPlaneMessage, Status>>,

    /// channels to send control messages
    tx_channels: parking_lot::RwLock<TxChannels>,

    /// cancellation token for graceful shutdown
    cancellation_tokens: parking_lot::RwLock<HashMap<String, CancellationToken>>,

    /// drain watch channel
    drain_rx: drain::Watch,

    /// array of connection details
    connection_details: Vec<ConnectionDetails>,
}

#[derive(Debug, Clone)]
struct ControllerService {
    /// internal service state
    inner: Arc<ControllerServiceInternal>,
}

/// The ControlPlane service is the main entry point for the controller service.
#[derive(Debug)]
pub struct ControlPlane {
    /// servers
    servers: Vec<ServerConfig>,

    /// clients
    clients: Vec<ClientConfig>,

    /// controller
    controller: ControllerService,

    /// channel to receive message from the datapath
    /// to be used in listen_from_data_plan
    rx_slim_option: Option<mpsc::Receiver<Result<DataPlaneMessage, Status>>>,
}

/// ControllerServiceInternal implements Drop trait to cancel all running listeners and
/// clean up resources.
impl Drop for ControlPlane {
    fn drop(&mut self) {
        // cancel all running listeners
        for (_endpoint, token) in self.controller.inner.cancellation_tokens.write().drain() {
            token.cancel();
        }
    }
}

fn from_server_config(server_config: &ServerConfig) -> ConnectionDetails {
    let group_name = server_config
        .metadata
        .as_ref()
        .and_then(|m| m.get("group_name"))
        .and_then(|v| match v {
            MetadataValue::String(s) => Some(s.clone()),
            _ => None,
        });
    let local_endpoint = server_config
        .metadata
        .as_ref()
        .and_then(|m| m.get("local_endpoint"))
        .and_then(|v| match v {
            MetadataValue::String(s) => Some(s.clone()),
            _ => None,
        });
    let external_endpoint = server_config
        .metadata
        .as_ref()
        .and_then(|m| m.get("external_endpoint"))
        .and_then(|v| match v {
            MetadataValue::String(s) => Some(s.clone()),
            _ => None,
        });
    ConnectionDetails {
        endpoint: server_config.endpoint.clone(),
        mtls_required: !server_config.tls_setting.insecure,
        group_name,
        local_endpoint,
        external_endpoint,
    }
}

/// ControlPlane implements the service trait for the controller service.
impl ControlPlane {
    /// Create a new ControlPlane service instance
    /// This function initializes the ControlPlane with the given ID, servers, clients, and message processor.
    /// It also sets up the internal state, including the connections and channels.
    /// # Arguments
    /// * `id` - The ID of the SLIM instance.
    /// * `servers` - A vector of server configurations.
    /// * `clients` - A vector of client configurations.
    /// * `drain_rx` - A drain watch channel for graceful shutdown.
    /// * `message_processor` - An Arc to the message processor instance.
    /// # Returns
    /// A new instance of ControlPlane.
    pub fn new(
        id: ID,
        group_name: Option<String>,
        servers: Vec<ServerConfig>,
        clients: Vec<ClientConfig>,
        drain_rx: drain::Watch,
        message_processor: Arc<MessageProcessor>,
        pubsub_servers: &[ServerConfig],
    ) -> Self {
        // create local connection with the message processor
        let (_, tx_slim, rx_slim) = message_processor.register_local_connection(true);

        let connection_details = pubsub_servers.iter().map(from_server_config).collect();

        ControlPlane {
            servers,
            clients,
            controller: ControllerService {
                inner: Arc::new(ControllerServiceInternal {
                    id,
                    group_name,
                    message_processor,
                    connections: Arc::new(parking_lot::RwLock::new(HashMap::new())),
                    tx_slim,
                    tx_channels: parking_lot::RwLock::new(HashMap::new()),
                    cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
                    drain_rx,
                    connection_details,
                }),
            },
            rx_slim_option: Some(rx_slim),
        }
    }

    /// Take an existing ControlPlane instance and return a new one with the provided clients.
    pub fn with_clients(mut self, clients: Vec<ClientConfig>) -> Self {
        self.clients = clients;
        self
    }

    /// Take an existing ControlPlane instance and return a new one with the provided servers.
    pub fn with_servers(mut self, servers: Vec<ServerConfig>) -> Self {
        self.servers = servers;
        self
    }

    /// Run the clients and servers of the ControlPlane service.
    /// This function starts all the servers and clients defined in the ControlPlane.
    /// # Returns
    /// A Result indicating success or failure of the operation.
    /// # Errors
    /// If there is an error starting any of the servers or clients, it will return a ControllerError.
    pub async fn run(&mut self) -> Result<(), ControllerError> {
        info!("starting controller service");

        // Collect servers to avoid borrowing self both mutably and immutably
        let servers = self.servers.clone();
        let clients = self.clients.clone();

        // run all servers
        for server in servers {
            self.run_server(server)?;
        }

        // run all clients
        for client in clients {
            self.run_client(client).await?;
        }

        let rx = self.rx_slim_option.take();
        self.listen_from_data_plane(rx.unwrap()).await;

        Ok(())
    }

    async fn listen_from_data_plane(
        &mut self,
        mut rx: mpsc::Receiver<Result<DataPlaneMessage, Status>>,
    ) {
        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();
        let drain = self.controller.inner.drain_rx.clone();

        self.controller
            .inner
            .cancellation_tokens
            .write()
            .insert("DATA_PLANE".to_string(), cancellation_token_clone);

        let clients = self.clients.clone();
        let inner = self.controller.inner.clone();

        // Send subscription to data-plane to receive messages for the controller source name
        let subscribe_msg =
            DataPlaneMessage::new_subscribe(&CONTROLLER_SOURCE_NAME, &CONTROLLER_SOURCE_NAME, None);

        // Send the subscribe message to the data plane
        if let Err(e) = inner.tx_slim.send(Ok(subscribe_msg)).await {
            error!("failed to send subscribe message to data plane: {}", e);
        }

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            Some(res) => {
                                match res {
                                    Ok(msg) => {
                                        debug!("Send sub/unsub to control plane for message: {:?}", msg);

                                        let mut sub_vec = vec![];
                                        let mut unsub_vec = vec![];

                                        let dst = msg.get_dst();
                                        let components = dst.components_strings().unwrap();
                                        let cmd = v1::Subscription {
                                            component_0: components[0].to_string(),
                                            component_1: components[1].to_string(),
                                            component_2: components[2].to_string(),
                                            id: Some(dst.id()),
                                            connection_id: "n/a".to_string(),
                                        };
                                        match msg.get_type() {
                                            slim_datapath::api::MessageType::Subscribe(_) => {
                                                sub_vec.push(cmd);
                                            },
                                            slim_datapath::api::MessageType::Unsubscribe(_) => {
                                                unsub_vec.push(cmd);
                                            }
                                            slim_datapath::api::MessageType::Publish(_) => {
                                                // drop publication messages
                                                continue;
                                            },
                                        }

                                        let ctrl = ControlMessage {
                                            message_id: uuid::Uuid::new_v4().to_string(),
                                            payload: Some(Payload::ConfigCommand(
                                                v1::ConfigurationCommand {
                                                    connections_to_create: vec![],
                                                    subscriptions_to_set: sub_vec,
                                                    subscriptions_to_delete: unsub_vec
                                                })),
                                        };

                                        for c in &clients {
                                            let tx = match inner.tx_channels.read().get(&c.endpoint) {
                                                Some(tx) => tx.clone(),
                                                None => continue,
                                            };
                                            if (tx.send(Ok(ctrl.clone())).await).is_err() {
                                                error!("error while notifiyng the control plane");
                                            };

                                        }
                                    }
                                    Err(e) => {
                                        error!("received error from the data plane {}", e.to_string());
                                        continue;
                                    }
                                }
                            }
                            None => {
                                debug!("Data plane receiver channel closed.");
                                break;
                            }
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        debug!("shutting down stream on cancellation token");
                        break;
                    }
                    _ = drain.clone().signaled() => {
                        debug!("shutting down stream on drain");
                        break;
                    }
                }
            }
        });
    }

    /// Stop the ControlPlane service.
    /// This function stops all running listeners and cancels any ongoing operations.
    /// It cleans up the internal state and ensures that all resources are released properly.
    pub fn stop(&mut self) {
        info!("stopping controller service");

        // cancel all running listeners
        for (endpoint, token) in self.controller.inner.cancellation_tokens.write().drain() {
            info!(%endpoint, "stopping");
            token.cancel();
        }
    }

    /// Run a client configuration.
    /// This function connects to the control plane using the provided client configuration.
    /// It checks if the client is already running and if not, it starts a new connection.
    async fn run_client(&mut self, client: ClientConfig) -> Result<(), ControllerError> {
        if self
            .controller
            .inner
            .cancellation_tokens
            .read()
            .contains_key(&client.endpoint)
        {
            return Err(ControllerError::ConfigError(format!(
                "client {} is already running",
                client.endpoint
            )));
        }

        let cancellation_token = CancellationToken::new();

        let tx = self
            .controller
            .connect(client.clone(), cancellation_token.clone())
            .await?;

        // Store the cancellation token in the controller service
        self.controller
            .inner
            .cancellation_tokens
            .write()
            .insert(client.endpoint.clone(), cancellation_token);

        // Store the sender in the tx_channels map
        self.controller
            .inner
            .tx_channels
            .write()
            .insert(client.endpoint.clone(), tx);

        // return the sender for control messages
        Ok(())
    }

    /// Run a server configuration.
    /// This function starts a server using the provided server configuration.
    /// It checks if the server is already running and if not, it starts a new server.
    pub fn run_server(&mut self, config: ServerConfig) -> Result<(), ControllerError> {
        info!(%config.endpoint, "starting control plane server");

        // Check if the server is already running
        if self
            .controller
            .inner
            .cancellation_tokens
            .read()
            .contains_key(&config.endpoint)
        {
            error!("server {} is already running", config.endpoint);
            return Err(ControllerError::ConfigError(format!(
                "server {} is already running",
                config.endpoint
            )));
        }

        let token = config
            .run_server(
                &[ControllerServiceServer::new(self.controller.clone())],
                self.controller.inner.drain_rx.clone(),
            )
            .map_err(|e| {
                error!("failed to run server {}: {}", config.endpoint, e);
                ControllerError::ConfigError(e.to_string())
            })?;

        // Store the cancellation token in the controller service
        self.controller
            .inner
            .cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), token.clone());

        info!(%config.endpoint, "control plane server started");

        Ok(())
    }
}

fn generate_session_id(moderator: &Name, channel: &Name) -> u32 {
    // get all the components of the two names
    // and hash them together to get the session id
    let mut all: [u64; 8] = [0; 8];
    let m = moderator.components();
    let c = channel.components();
    all[..4].copy_from_slice(m);
    all[4..].copy_from_slice(c);

    let hash = calculate_hash(&all);
    (hash ^ (hash >> 32)) as u32
}

fn get_name_from_string(string_name: &String) -> Result<Name, ControllerError> {
    let parts: Vec<&str> = string_name.split('/').collect();
    if parts.len() < 3 {
        return Err(ControllerError::ConfigError(format!(
            "invalid name format: {}",
            string_name
        )));
    }

    if parts.len() == 4 {
        let id = parts[3].parse::<u64>().map_err(|_| {
            ControllerError::ConfigError(format!("invalid moderator ID: {}", parts[3]))
        })?;
        Ok(Name::from_strings([parts[0], parts[1], parts[2]]).with_id(id))
    } else {
        Ok(Name::from_strings([parts[0], parts[1], parts[2]]))
    }
}

#[allow(clippy::too_many_arguments)]
fn create_channel_message(
    source: &Name,
    destination: &Name,
    // TODO(micpapal): this needs to be removed
    // use the protobuf file to define the payload
    // of the packets
    channel: Option<&Name>,
    request_type: ProtoSessionMessageType,
    session_id: u32,
    message_id: u32,
    mut metadata: HashMap<String, String>,
    payload: Vec<u8>,
) -> DataPlaneMessage {
    let slim_header = Some(SlimHeader::new(source, destination, None));
    let dest = channel.unwrap_or(destination);

    let session_header = Some(SessionHeader::new(
        ProtoSessionType::SessionMulticast.into(),
        request_type.into(),
        session_id,
        message_id,
        &None,
        &Some(dest.clone()),
    ));

    // TODO: fix the identity of the controller
    let controller_identity = SharedSecret::new("controller", "group");
    let identity_token = controller_identity
        .get_token()
        .map_err(|e| {
            error!("failed to generate identity token: {}", e);
            ControllerError::DatapathError(e.to_string())
        })
        .unwrap();

    metadata.insert(SLIM_IDENTITY.to_string(), identity_token);
    let mut msg =
        DataPlaneMessage::new_publish_with_headers(slim_header, session_header, "", payload);

    msg.set_metadata_map(metadata);

    msg
}

fn create_new_channel_message(
    controller: &Name,
    moderator: &Name,
    channel: &Name,
) -> DataPlaneMessage {
    let session_id = generate_session_id(moderator, channel);

    // Local copy of JoinMessagePayload (original defined in channel endpoint module for data plane service).
    // Duplicated here because controller does not depend on the service module.
    // TODO(micpapal): handle this using the protobuf
    #[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
    struct JoinMessagePayloadLocal {
        channel_name: Name,
        moderator_name: Name,
    }
    let p = JoinMessagePayloadLocal {
        channel_name: channel.clone(),
        moderator_name: moderator.clone(),
    };
    let invite_payload: Vec<u8> = bincode::encode_to_vec(p, bincode::config::standard())
        .expect("unable to encode channel join payload");

    let mut metadata = HashMap::new();

    metadata.insert("IS_MODERATOR".to_string(), "true".to_string());

    // by default MLS is always on
    // TODO(micpapal): define all these metadata constants somewhere
    // that is accessible everywhere
    metadata.insert("MLS_ENABLED".to_string(), "true".to_string());

    create_channel_message(
        controller,
        moderator,
        Some(channel),
        ProtoSessionMessageType::ChannelJoinRequest,
        session_id,
        rand::random::<u32>(),
        metadata,
        invite_payload,
    )
}

fn invite_participant_message(
    controller: &Name,
    moderator: &Name,
    participant: &Name,
    channel_name: &Name,
) -> DataPlaneMessage {
    let session_id = generate_session_id(moderator, channel_name);
    let mut metadata = HashMap::new();

    let encoded_participant: Vec<u8> =
        bincode::encode_to_vec(participant, bincode::config::standard())
            .expect("unable to encode channel join payload");
    let encoded_participant_str =
        base64::engine::general_purpose::STANDARD.encode(&encoded_participant);

    metadata.insert("PARTICIPANT_NAME".to_string(), encoded_participant_str);

    create_channel_message(
        controller,
        moderator,
        None,
        ProtoSessionMessageType::ChannelDiscoveryRequest,
        session_id,
        rand::random::<u32>(),
        metadata,
        vec![],
    )
}

fn remove_participant_message(
    controller: &Name,
    moderator: &Name,
    participant: &Name,
    channel_name: &Name,
) -> DataPlaneMessage {
    let session_id = generate_session_id(moderator, channel_name);

    let mut metadata = HashMap::new();
    let encoded_participant: Vec<u8> =
        bincode::encode_to_vec(participant, bincode::config::standard())
            .expect("unable to encode channel join payload");
    let encoded_participant_str =
        base64::engine::general_purpose::STANDARD.encode(&encoded_participant);
    metadata.insert("PARTICIPANT_NAME".to_string(), encoded_participant_str);

    create_channel_message(
        controller,
        moderator,
        None,
        ProtoSessionMessageType::ChannelLeaveRequest,
        session_id,
        rand::random::<u32>(),
        metadata,
        vec![],
    )
}

impl ControllerService {
    const MAX_RETRIES: i32 = 10;

    /// Handle new control messages.
    async fn handle_new_control_message(
        &self,
        msg: ControlMessage,
        tx: &mpsc::Sender<Result<ControlMessage, Status>>,
    ) -> Result<(), ControllerError> {
        match msg.payload {
            Some(ref payload) => {
                match payload {
                    Payload::ConfigCommand(config) => {
                        for conn in &config.connections_to_create {
                            info!("received a connection to create: {:?}", conn);
                            let client_config =
                                serde_json::from_str::<ClientConfig>(&conn.config_data)
                                    .map_err(|e| ControllerError::ConfigError(e.to_string()))?;
                            let client_endpoint = &client_config.endpoint;

                            // connect to an endpoint if it's not already connected
                            if !self.inner.connections.read().contains_key(client_endpoint) {
                                match client_config.to_channel() {
                                    Err(e) => {
                                        error!("error reading channel config {:?}", e);
                                    }
                                    Ok(channel) => {
                                        let ret = self
                                            .inner
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

                                        self.inner
                                            .connections
                                            .write()
                                            .insert(client_endpoint.clone(), conn_id);
                                    }
                                }
                            }
                        }

                        for subscription in &config.subscriptions_to_set {
                            if !self
                                .inner
                                .connections
                                .read()
                                .contains_key(&subscription.connection_id)
                            {
                                error!("connection {} not found", subscription.connection_id);
                                continue;
                            }

                            let conn = self
                                .inner
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Name::from_strings([
                                subscription.component_0.as_str(),
                                subscription.component_1.as_str(),
                                subscription.component_2.as_str(),
                            ])
                            .with_id(0);
                            let name = Name::from_strings([
                                subscription.component_0.as_str(),
                                subscription.component_1.as_str(),
                                subscription.component_2.as_str(),
                            ])
                            .with_id(subscription.id.unwrap_or(Name::NULL_COMPONENT));

                            let msg = DataPlaneMessage::new_subscribe(
                                &source,
                                &name,
                                Some(SlimHeaderFlags::default().with_recv_from(conn)),
                            );

                            if let Err(e) = self.send_control_message(msg).await {
                                error!("failed to subscribe: {}", e);
                            }
                        }

                        for subscription in &config.subscriptions_to_delete {
                            if !self
                                .inner
                                .connections
                                .read()
                                .contains_key(&subscription.connection_id)
                            {
                                error!("connection {} not found", subscription.connection_id);
                                continue;
                            }

                            let conn = self
                                .inner
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned()
                                .unwrap();
                            let source = Name::from_strings([
                                subscription.component_0.as_str(),
                                subscription.component_1.as_str(),
                                subscription.component_2.as_str(),
                            ])
                            .with_id(0);
                            let name = Name::from_strings([
                                subscription.component_0.as_str(),
                                subscription.component_1.as_str(),
                                subscription.component_2.as_str(),
                            ])
                            .with_id(subscription.id.unwrap_or(Name::NULL_COMPONENT));

                            let msg = DataPlaneMessage::new_unsubscribe(
                                &source,
                                &name,
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
                            payload: Some(Payload::Ack(ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!("failed to send ACK: {}", e);
                        }
                    }
                    Payload::SubscriptionListRequest(_) => {
                        const CHUNK_SIZE: usize = 100;

                        let conn_table = self.inner.message_processor.connection_table();
                        let mut entries = Vec::new();

                        self.inner.message_processor.subscription_table().for_each(
                            |name, id, local, remote| {
                                let mut entry = SubscriptionEntry {
                                    component_0: name.components_strings().unwrap()[0].to_string(),
                                    component_1: name.components_strings().unwrap()[1].to_string(),
                                    component_2: name.components_strings().unwrap()[2].to_string(),
                                    id: Some(id),
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
                            },
                        );

                        for chunk in entries.chunks(CHUNK_SIZE) {
                            let resp = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::SubscriptionListResponse(
                                    SubscriptionListResponse {
                                        entries: chunk.to_vec(),
                                    },
                                )),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!("failed to send subscription batch: {}", e);
                            }
                        }
                    }
                    Payload::ConnectionListRequest(_) => {
                        let mut all_entries = Vec::new();
                        self.inner
                            .message_processor
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
                                payload: Some(Payload::ConnectionListResponse(
                                    ConnectionListResponse {
                                        entries: chunk.to_vec(),
                                    },
                                )),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!("failed to send connection list batch: {}", e);
                            }
                        }
                    }
                    Payload::Ack(_ack) => {
                        // received an ack, do nothing - this should not happen
                    }
                    Payload::SubscriptionListResponse(_) => {
                        // received a subscription list response, do nothing - this should not happen
                    }
                    Payload::ConnectionListResponse(_) => {
                        // received a connection list response, do nothing - this should not happen
                    }
                    Payload::RegisterNodeRequest(_) => {
                        error!("received a register node request");
                    }
                    Payload::RegisterNodeResponse(_) => {
                        // received a register node response, do nothing
                    }
                    Payload::DeregisterNodeRequest(_) => {
                        error!("received a deregister node request");
                    }
                    Payload::DeregisterNodeResponse(_) => {
                        // received a deregister node response, do nothing
                    }
                    Payload::CreateChannelRequest(req) => {
                        info!("received a create channel request");

                        // Get the first moderator from the list, as we support only one for now
                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                return Err(ControllerError::ConfigError(
                                    "invalid moderator ID".to_owned(),
                                ));
                            }

                            let channel_name = get_name_from_string(&req.channel_name)?;

                            let source_name = CONTROLLER_SOURCE_NAME.clone();
                            let creation_msg = create_new_channel_message(
                                &source_name,
                                &moderator_name,
                                &channel_name,
                            );

                            debug!("Send session creation message: {:?}", creation_msg);
                            if let Err(e) = self.send_control_message(creation_msg).await {
                                error!("failed to send channel creation: {}", e);
                            }
                        } else {
                            return Err(ControllerError::ConfigError(
                                "no moderators specified in add participant request".to_string(),
                            ));
                        };

                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            messages: vec![msg.message_id.clone()],
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(Payload::Ack(ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!("failed to send Ack: {}", e);
                        }
                    }
                    Payload::DeleteChannelRequest(req) => {
                        info!("received a channel delete request");

                        let channel_id = req.channel_name.clone();

                        let moderator_name = if let Some(first_moderator) = req.moderators.first() {
                            let parts: Vec<&str> = first_moderator.split('/').collect();
                            if parts.len() != 4 {
                                return Err(ControllerError::ConfigError(format!(
                                    "invalid moderator name format: {}",
                                    first_moderator
                                )));
                            }
                            let id = parts[3].parse::<u64>().map_err(|_| {
                                ControllerError::ConfigError(format!(
                                    "invalid moderator ID: {}",
                                    parts[3]
                                ))
                            })?;
                            Name::from_strings([parts[0], parts[1], parts[2]]).with_id(id)
                        } else {
                            return Err(ControllerError::ConfigError(
                                "no moderators specified in delete channel request".to_string(),
                            ));
                        };
                        let source_name = CONTROLLER_SOURCE_NAME.clone();

                        let moderator_request = ModeratorMessage {
                            message_id: Uuid::new_v4().to_string(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64,
                            payload: Some(crate::api::proto::moderator::v1::moderator_message::Payload::DeleteChannel(
                                DeleteChannelRequest {
                                    channel_id: channel_id.clone(),
                                }
                            )),
                        };

                        let message_content = moderator_request.encode_to_vec();

                        let slim_header = Some(SlimHeader::new(
                            &source_name,
                            &moderator_name,
                            Some(SlimHeaderFlags::default()),
                        ));
                        let session_header = Some(SessionHeader::new(
                            ProtoSessionType::SessionPointToPoint.into(),
                            ProtoSessionMessageType::P2PMsg.into(),
                            0,
                            Uuid::new_v4().as_u128() as u32,
                            &None,
                            &None,
                        ));

                        let mut publish_msg = DataPlaneMessage::new_publish_with_headers(
                            slim_header,
                            session_header,
                            "application/x-moderator-protobuf",
                            message_content,
                        );

                        let controller_identity = SharedSecret::new("controller", "group");
                        let identity_token = controller_identity.get_token().map_err(|e| {
                            error!("failed to generate identity token: {}", e);
                            ControllerError::DatapathError(e.to_string())
                        })?;
                        publish_msg.insert_metadata(SLIM_IDENTITY.to_string(), identity_token);

                        if let Err(e) = self.send_control_message(publish_msg).await {
                            error!(
                                "failed to send message to moderator {}: {}",
                                req.moderators.first().unwrap_or(&"unknown".to_string()),
                                e
                            );
                        } else {
                            info!("successfully sent delete_channel message to moderator");
                        }
                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            messages: vec![msg.message_id.clone()],
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(Payload::Ack(ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!("failed to send Ack: {}", e);
                        }
                    }
                    Payload::AddParticipantRequest(req) => {
                        info!(
                            "received a participant add request for channel: {}, participant: {}",
                            req.channel_name, req.participant_name
                        );

                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                return Err(ControllerError::ConfigError(
                                    "invalid moderator ID".to_string(),
                                ));
                            }

                            let channel_name = get_name_from_string(&req.channel_name)?;
                            let participant_name = get_name_from_string(&req.participant_name)?;
                            let source_name = CONTROLLER_SOURCE_NAME.clone();

                            let invite_msg = invite_participant_message(
                                &source_name,
                                &moderator_name,
                                &participant_name,
                                &channel_name,
                            );

                            debug!("Send invite participant: {:?}", invite_msg);

                            if let Err(e) = self.send_control_message(invite_msg).await {
                                error!("failed to send channel creation: {}", e);
                            }
                        } else {
                            return Err(ControllerError::ConfigError(
                                "no moderators specified in add participant request".to_string(),
                            ));
                        };

                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            messages: vec![msg.message_id.clone()],
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(Payload::Ack(ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!("failed to send Ack: {}", e);
                        }
                    }
                    Payload::DeleteParticipantRequest(req) => {
                        info!("received a participant delete request");

                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                return Err(ControllerError::ConfigError(
                                    "invalid moderator ID".to_string(),
                                ));
                            }

                            let channel_name = get_name_from_string(&req.channel_name)?;
                            let participant_name = get_name_from_string(&req.participant_name)?;
                            let source_name = CONTROLLER_SOURCE_NAME.clone();

                            let remove_msg = remove_participant_message(
                                &source_name,
                                &moderator_name,
                                &participant_name,
                                &channel_name,
                            );

                            if let Err(e) = self.send_control_message(remove_msg).await {
                                error!("failed to send channel creation: {}", e);
                            }
                        } else {
                            return Err(ControllerError::ConfigError(
                                "no moderators specified in add participant request".to_string(),
                            ));
                        };

                        let ack = Ack {
                            original_message_id: msg.message_id.clone(),
                            success: true,
                            messages: vec![msg.message_id.clone()],
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(Payload::Ack(ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!("failed to send Ack: {}", e);
                        }
                    }
                    Payload::ListChannelRequest(_) => {}
                    Payload::ListChannelResponse(_) => {}
                    Payload::ListParticipantsRequest(_) => {}
                    Payload::ListParticipantsResponse(_) => {}
                }
            }
            None => {
                error!(
                    "received control message {} with no payload",
                    msg.message_id
                );
            }
        }

        Ok(())
    }

    /// Send a control message to SLIM.
    async fn send_control_message(&self, msg: DataPlaneMessage) -> Result<(), ControllerError> {
        self.inner.tx_slim.send(Ok(msg)).await.map_err(|e| {
            error!("error sending message into datapath: {}", e);
            ControllerError::DatapathError(e.to_string())
        })
    }

    /// Process the control message stream.
    fn process_control_message_stream(
        &self,
        config: Option<ClientConfig>,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
        cancellation_token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        let drain = this.inner.drain_rx.clone();
        tokio::spawn(async move {
            // Send a register message to the control plane
            let endpoint = config
                .as_ref()
                .map(|c| c.endpoint.clone())
                .unwrap_or_else(|| "unknown".to_string());
            info!(%endpoint, "connected to control plane");

            let mut retry_connect = false;

            let register_request = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(Payload::RegisterNodeRequest(v1::RegisterNodeRequest {
                    node_id: this.inner.id.to_string(),
                    group_name: this.inner.group_name.clone(),
                    connection_details: this.inner.connection_details.clone(),
                })),
            };

            // send register request if client
            if config.is_some()
                && let Err(e) = tx.send(Ok(register_request)).await
            {
                error!("failed to send register request: {}", e);
                return;
            }

            // TODO; here we should wait for an ack

            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(Ok(msg)) => {
                                if let Err(e) = this.handle_new_control_message(msg, &tx).await {
                                    error!("error processing incoming control message: {:?}", e);
                                }
                            }
                            Some(Err(e)) => {
                                if let Some(io_err) = Self::match_for_io_error(&e) {
                                    if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                                        info!("connection closed by peer");
                                        retry_connect = true;
                                    }
                                } else {
                                    error!(%e, "error receiving control messages");
                                }

                                break;
                            }
                            None => {
                                debug!("end of stream");
                                retry_connect = true;
                                break;
                            }
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        debug!("shutting down stream on cancellation token");
                        break;
                    }
                    _ = drain.clone().signaled() => {
                        debug!("shutting down stream on drain");
                        break;
                    }
                }
            }

            info!(%endpoint, "control plane stream closed");

            if retry_connect && let Some(config) = config {
                info!(%config.endpoint, "retrying connection to control plane");
                this.connect(config.clone(), cancellation_token)
                    .await
                    .map_or_else(
                        |e| {
                            error!("failed to reconnect to control plane: {}", e);
                        },
                        |tx| {
                            info!(%config.endpoint, "reconnected to control plane");

                            this.inner
                                .tx_channels
                                .write()
                                .insert(config.endpoint.clone(), tx);
                        },
                    )
            }
        })
    }

    /// Connect to the control plane using the provided client configuration.
    /// This function attempts to establish a connection to the control plane and returns a sender for control messages.
    /// It retries the connection a specified number of times if it fails.
    async fn connect(
        &self,
        config: ClientConfig,
        cancellation_token: CancellationToken,
    ) -> Result<mpsc::Sender<Result<ControlMessage, Status>>, ControllerError> {
        info!(%config.endpoint, "connecting to control plane");

        let channel = config.to_channel().map_err(|e| {
            error!("error reading channel config: {}", e);
            ControllerError::ConfigError(e.to_string())
        })?;

        let mut client = ControllerServiceClient::new(channel);
        for i in 0..Self::MAX_RETRIES {
            let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
            let out_stream = ReceiverStream::new(rx).map(|res| res.expect("mapping error"));
            match client.open_control_channel(Request::new(out_stream)).await {
                Ok(stream) => {
                    // process the control message stream
                    self.process_control_message_stream(
                        Some(config),
                        stream.into_inner(),
                        tx.clone(),
                        cancellation_token.clone(),
                    );

                    return Ok(tx);
                }
                Err(e) => {
                    error!(%e, "connection error, retrying {}/{}", i + 1, Self::MAX_RETRIES);
                }
            };

            // sleep 1 sec between each connection retry
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        Err(ControllerError::ConfigError(format!(
            "failed to connect to control plane after {} retries",
            Self::MAX_RETRIES
        )))
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
}

#[tonic::async_trait]
impl GrpcControllerService for ControllerService {
    type OpenControlChannelStream =
        Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send + 'static>>;

    async fn open_control_channel(
        &self,
        request: Request<tonic::Streaming<ControlMessage>>,
    ) -> Result<Response<Self::OpenControlChannelStream>, Status> {
        // Get the remote endpoint from the request metadata
        let remote_endpoint = request
            .remote_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);

        let cancellation_token = CancellationToken::new();

        self.process_control_message_stream(None, stream, tx.clone(), cancellation_token.clone());

        // store the sender in the tx_channels map
        self.inner
            .tx_channels
            .write()
            .insert(remote_endpoint.clone(), tx);

        // store the cancellation token in the controller service
        self.inner
            .cancellation_tokens
            .write()
            .insert(remote_endpoint.clone(), cancellation_token);

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::OpenControlChannelStream
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::component::id::Kind;
    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end() {
        // Create an ID for slim instance
        let id_server =
            ID::new_with_name(Kind::new("slim").unwrap(), "test-server-instance").unwrap();
        let id_client =
            ID::new_with_name(Kind::new("slim").unwrap(), "test-client-instance").unwrap();

        // Create a server configuration
        let server_config = ServerConfig::with_endpoint("127.0.0.1:50051")
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure());

        // create a client configuration
        let client_config = ClientConfig::with_endpoint("http://127.0.0.1:50051")
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure());

        // create drain channels
        let (signal_server, watch_server) = drain::channel();
        let (signal_client, watch_client) = drain::channel();

        // Create a message processor
        let message_processor_client = MessageProcessor::with_drain_channel(watch_client.clone());
        let message_processor_server = MessageProcessor::with_drain_channel(watch_server.clone());

        // Create a control plane instance for server
        let pubsub_servers = [server_config.clone()];
        let mut control_plane_server = ControlPlane::new(
            id_server,
            None,
            vec![server_config],
            vec![],
            watch_server,
            Arc::new(message_processor_server),
            &pubsub_servers,
        );

        let mut control_plane_client = ControlPlane::new(
            id_client,
            None,
            vec![],
            vec![client_config],
            watch_client,
            Arc::new(message_processor_client),
            &pubsub_servers,
        );

        // Start the server
        control_plane_server.run().await.unwrap();

        // Sleep for a short duration to ensure the server is ready
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Start the client
        control_plane_client.run().await.unwrap();

        // Sleep for a short duration to ensure the client is ready
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Check if the server received the connection
        assert!(logs_contain("received a register node request"));

        // drop the server and the client. This should also cancel the running listeners
        // and close the connections gracefully.
        drop(control_plane_server);
        drop(control_plane_client);

        // Make sure there is nothing left to drain (this should not block)
        signal_server.drain().await;
        signal_client.drain().await;
    }

    #[test]
    fn test_generate_session_id() {
        let moderator_a = Name::from_strings(["Org", "Ns", "Moderator"]).with_id(42);
        let moderator_b = Name::from_strings(["Org", "Ns", "Moderator"]).with_id(43); // different id
        let channel_x = Name::from_strings(["Org", "Ns", "ChannelX"]).with_id(7);
        let channel_y = Name::from_strings(["Org", "Ns", "ChannelY"]).with_id(7); // different last component

        let id1 = generate_session_id(&moderator_a, &channel_x);
        let id2 = generate_session_id(&moderator_a, &channel_x);
        assert_eq!(id1, id2, "hash must be deterministic for same inputs");

        let id3 = generate_session_id(&moderator_b, &channel_x);
        assert_ne!(id1, id3, "changing moderator id should change session id");

        let id4 = generate_session_id(&moderator_a, &channel_y);
        assert_ne!(id1, id4, "changing channel name should change session id");

        // Ensure moderate spread (not strictly required, but sanity check that values aren't zero)
        assert!(
            id1 != 0 && id3 != 0 && id4 != 0,
            "session ids should not be zero"
        );
    }
}

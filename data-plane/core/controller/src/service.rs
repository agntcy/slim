// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::vec;

use display_error_chain::ErrorChainExt;
use slim_config::component::id::ID;
use slim_config::grpc::server::ServerConfig;
use slim_session::SessionMessage;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::api::proto::api::v1::control_message::Payload;
use crate::api::proto::api::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::api::v1::{
    self, ConnectionDetails, ConnectionListResponse, ConnectionType, SubscriptionListResponse,
};
use crate::api::proto::api::v1::{
    Ack, ConnectionEntry, ControlMessage, SubscriptionEntry,
    controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::errors::ControllerError;
use prost_types::Struct;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::TokenProvider;
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::{
    CommandPayload, Content, MessageType::Publish, MessageType::Subscribe,
    MessageType::Unsubscribe, ProtoMessage as DataPlaneMessage,
};
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;
use slim_datapath::messages::encoder::calculate_hash;
use slim_datapath::messages::utils::{
    DELETE_GROUP, IS_MODERATOR, SUBSCRIPTION_ACK_ERROR, SUBSCRIPTION_ACK_ID,
    SUBSCRIPTION_ACK_SUCCESS, SlimHeaderFlags, TRUE_VAL,
};
use slim_datapath::tables::SubscriptionTable;

use slim_session::timer::{Timer, TimerType};
use slim_session::timer_factory::{TimerFactory, TimerSettings};

type TxChannel = mpsc::Sender<Result<ControlMessage, Status>>;
type TxChannels = HashMap<String, TxChannel>;

// Controller component
const CONTROLLER_COMPONENT: &str = "controller";
/// Maximum number of queued subscription notifications
const MAX_QUEUED_NOTIFICATIONS: usize = 1000; // Prevent unbounded growth

/// Settings struct for creating a ControlPlane instance
#[derive(Clone)]
pub struct ControlPlaneSettings {
    /// ID of this SLIM instance
    pub id: ID,
    /// Optional group name
    pub group_name: Option<String>,
    /// Server configurations
    pub servers: Vec<ServerConfig>,
    /// Client configurations
    pub clients: Vec<ClientConfig>,
    /// Message processor instance
    pub message_processor: Arc<MessageProcessor>,
    /// Optional authentication provider
    pub auth_provider: Option<AuthProvider>,
    /// Optional authentication verifier
    pub auth_verifier: Option<AuthVerifier>,
    /// array of connection details used by the control
    /// plane to store the connection settings (e.g., TLS settings).
    pub connection_details: Vec<ConnectionDetails>,
}

/// Inner structure for the controller service
/// This structure holds the internal state of the controller service,
/// including the ID, message processor, connections, and channels.
/// It is normally wrapped in an Arc to allow shared ownership across multiple threads.
struct ControllerServiceInternal {
    /// ID of this SLIM instance
    id: ID,

    /// controller name
    controller_name: slim_datapath::messages::Name,

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
    drain_watch: parking_lot::RwLock<Option<drain::Watch>>,

    /// authentication provider for adding authentication to outgoing messages to clients
    auth_provider: Option<AuthProvider>,

    /// authentication verifier for verifying incoming messages from clients
    _auth_verifier: Option<AuthVerifier>,

    /// queue for pending subscription notifications when connections are down
    pending_notifications: Arc<parking_lot::Mutex<Vec<ControlMessage>>>,

    /// pending subscription acknowledgments keyed by ack id
    pending_subscription_acks:
        parking_lot::Mutex<HashMap<String, oneshot::Sender<Result<(), String>>>>,

    /// counter used to generate subscription acknowledgment ids
    subscription_ack_counter: AtomicU64,

    /// map of generated u32 keys to original string message IDs and their associated timers
    message_id_map: Arc<parking_lot::RwLock<HashMap<u32, (String, Option<Timer>)>>>,

    /// timer factory for controller messages
    /// used to create timers for messages that require timeouts
    /// the lock is needed to set the timer factory after initialization
    /// because it requires a channel to send session messages
    timer_factory: parking_lot::RwLock<Option<TimerFactory>>,

    /// connection details used by control plane to store connection settings
    connection_details: Vec<ConnectionDetails>,
}

#[derive(Clone)]
struct ControllerService {
    /// internal service state
    inner: Arc<ControllerServiceInternal>,
}

/// The ControlPlane service is the main entry point for the controller service.
pub struct ControlPlane {
    /// servers
    servers: Vec<ServerConfig>,

    /// clients
    clients: Vec<ClientConfig>,

    /// drain signal channel
    drain_signal: parking_lot::RwLock<Option<drain::Signal>>,

    /// controller
    controller: ControllerService,

    /// channel to receive message from the datapath
    /// to be used in listen_from_data_plane
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

pub(crate) fn from_server_config(server_config: &ServerConfig) -> ConnectionDetails {
    // Convert metadata from MetadataMap to proto Struct
    let metadata = server_config.metadata.as_ref().map(|m| {
        let fields = m
            .inner
            .iter()
            .map(|(k, v)| (k.clone(), prost_types::Value::from(v)))
            .collect();
        Struct { fields }
    });

    // Serialize auth config to JSON string
    let auth = serde_json::to_string(&server_config.auth).ok();

    // Serialize tls config to JSON string
    let tls = serde_json::to_string(&server_config.tls_setting.config).ok();

    ConnectionDetails {
        endpoint: server_config.endpoint.clone(),
        mtls_required: !server_config.tls_setting.insecure,
        metadata,
        auth,
        tls,
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
    /// * `pubsub_servers` - A slice of server configurations for pub/sub connections.
    /// # Returns
    /// A new instance of ControlPlane.
    pub fn new(config: ControlPlaneSettings) -> Self {
        // create local connection with the message processor
        let (_, tx_slim, rx_slim) = config
            .message_processor
            .register_local_connection(true)
            .unwrap();

        let (signal, watch) = drain::channel();
        let controller_name = Name::from_strings([
            CONTROLLER_COMPONENT,
            CONTROLLER_COMPONENT,
            CONTROLLER_COMPONENT,
        ])
        .with_id(rand::random::<u64>());
        debug!("create controller with name: {}", controller_name);

        ControlPlane {
            servers: config.servers,
            clients: config.clients,
            controller: ControllerService {
                inner: Arc::new(ControllerServiceInternal {
                    id: config.id,
                    controller_name,
                    group_name: config.group_name,
                    message_processor: config.message_processor,
                    connections: Arc::new(parking_lot::RwLock::new(HashMap::new())),
                    tx_slim,
                    tx_channels: parking_lot::RwLock::new(HashMap::new()),
                    cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
                    drain_watch: parking_lot::RwLock::new(Some(watch)),
                    auth_provider: config.auth_provider,
                    _auth_verifier: config.auth_verifier,
                    pending_notifications: Arc::new(parking_lot::Mutex::new(Vec::new())),
                    pending_subscription_acks: parking_lot::Mutex::new(HashMap::new()),
                    subscription_ack_counter: AtomicU64::new(0),
                    message_id_map: Arc::new(parking_lot::RwLock::new(HashMap::new())),
                    timer_factory: parking_lot::RwLock::new(None),
                    connection_details: config.connection_details,
                }),
            },
            drain_signal: parking_lot::RwLock::new(Some(signal)),
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
        let rx = self
            .rx_slim_option
            .take()
            .ok_or(ControllerError::AlreadyStarted)?;

        // Collect servers to avoid borrowing self both mutably and immutably
        let servers = self.servers.clone();
        let clients = self.clients.clone();

        // run all servers
        for server in servers {
            self.run_server(server).await?;
        }

        // run all clients
        for client in clients {
            self.run_client(client).await?;
        }

        self.listen_from_data_plane(rx).await?;

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), ControllerError> {
        // Get signal drain
        let signal = self
            .drain_signal
            .write()
            .take()
            .ok_or(ControllerError::AlreadyStopped)?;

        // Stop everything using the cancellation tokens
        self.controller
            .inner
            .cancellation_tokens
            .write()
            .drain()
            .for_each(|(endpoint, token)| {
                info!(%endpoint, "stopping");
                token.cancel();
            });

        // Drop watch channel
        self.controller.inner.drain_watch.write().take();

        // Wait for drain to complete
        signal.drain().await;

        Ok(())
    }

    async fn listen_from_data_plane(
        &mut self,
        mut rx: mpsc::Receiver<Result<DataPlaneMessage, Status>>,
    ) -> Result<(), ControllerError> {
        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        self.controller
            .inner
            .cancellation_tokens
            .write()
            .insert("DATA_PLANE".to_string(), cancellation_token_clone);

        let clients = self.clients.clone();
        let controller = self.controller.clone();

        // Send subscription to data-plane to receive messages for the controller source name
        let controller_name = self.controller.inner.controller_name.clone();
        let subscribe_msg = DataPlaneMessage::builder()
            .source(controller_name.clone())
            .destination(controller_name.clone())
            .identity(controller_name.to_string())
            .build_subscribe()
            .unwrap();

        controller
            .inner
            .tx_slim
            .send(Ok(subscribe_msg))
            .await
            .map_err(|e| {
                error!(error = %e.chain(), "failed to send subscribe message to data plane");
                ControllerError::DatapathSendError(e.to_string())
            })?;

        // Get a drain watch clone
        let watch = self.controller.drain_watch()?;

        debug!("Starting data plane listener: {}", controller_name);
        tokio::spawn(async move {
            let mut drain_fut = std::pin::pin!(watch.signaled());
            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            Some(res) => {
                                match res {
                                    Ok(msg) => {
                                        debug!("Send sub/unsub/ack to control plane for message: {:?}", msg);
                                        if controller.handle_subscription_ack_message(&msg) {
                                            continue;
                                        }
                                        match msg.get_type() {
                                            Subscribe(_) => {
                                                controller.handle_subscribe_message(msg.get_dst(), &clients).await;
                                            }
                                            Unsubscribe(_) => {
                                                controller.handle_unsubscribe_message(msg.get_dst(), &clients).await;
                                            }
                                            Publish(_) => {
                                                if msg.get_session_message_type() == ProtoSessionMessageType::GroupAck {
                                                    controller.send_ack_message(msg.get_id(), true, &clients).await;
                                                } else {
                                                    debug!("Ignoring publish message with session type: {:?}", msg.get_session_message_type());
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(error = %e.chain(), "received error from the data plane");
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
                    _ = &mut drain_fut => {
                        debug!("shutting down stream on drain");
                        break;
                    }
                }
            }
        });
        Ok(())
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
            return Err(ControllerError::ClientAlreadyRunning(client.endpoint));
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
    pub async fn run_server(&mut self, config: ServerConfig) -> Result<(), ControllerError> {
        // Check if the server is already running
        if self
            .controller
            .inner
            .cancellation_tokens
            .read()
            .contains_key(&config.endpoint)
        {
            error!(endpoint = config.endpoint, "server is already running",);
            return Err(ControllerError::ServerAlreadyRunning(config.endpoint));
        }

        let token = config
            .run_server(
                &[ControllerServiceServer::new(self.controller.clone())],
                self.controller.drain_watch()?,
            )
            .await?;

        // Store the cancellation token in the controller service
        self.controller
            .inner
            .cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), token.clone());

        info!(%config.endpoint, "started controlplane server");

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

fn get_name_from_string(string_name: &str) -> Result<Name, ControllerError> {
    let parts: Vec<&str> = string_name.split('/').collect();
    if parts.len() < 3 {
        return Err(ControllerError::MalformedName(string_name.to_owned()));
    }

    if parts.len() == 4 {
        let id = parts[3]
            .parse::<u64>()
            .map_err(|_e| ControllerError::MalformedName(string_name.to_owned()))?;
        return Ok(Name::from_strings([parts[0], parts[1], parts[2]]).with_id(id));
    }

    Ok(Name::from_strings([parts[0], parts[1], parts[2]]))
}

#[allow(clippy::too_many_arguments)]
fn create_channel_message(
    source: &Name,
    destination: &Name,
    request_type: ProtoSessionMessageType,
    session_id: u32,
    message_id: u32,
    payload: Option<Content>,
    auth_provider: &Option<AuthProvider>,
) -> Result<DataPlaneMessage, ControllerError> {
    // if the auth_provider is set try to get an identity
    let identity_token = match auth_provider {
        Some(auth) => auth.get_token()?,
        None => String::new(),
    };

    let message = DataPlaneMessage::builder()
        .source(source.clone())
        .destination(destination.clone())
        .identity(&identity_token)
        .session_type(ProtoSessionType::Multicast)
        .session_message_type(request_type)
        .session_id(session_id)
        .message_id(message_id)
        .payload(payload.ok_or(ControllerError::PayloadMissing)?)
        .build_publish()?;

    Ok(message)
}

fn new_channel_message(
    controller: &Name,
    moderator: &Name,
    channel: &Name,
    message_id: u32,
    auth_provider: &Option<AuthProvider>,
) -> Result<DataPlaneMessage, ControllerError> {
    let session_id = generate_session_id(moderator, channel);

    let invite_payload = Some(
        CommandPayload::builder()
            .join_request(
                true,
                Some(10),
                Some(Duration::from_secs(1)),
                Some(channel.clone()),
            )
            .as_content(),
    );

    let mut msg = create_channel_message(
        controller,
        moderator,
        ProtoSessionMessageType::JoinRequest,
        session_id,
        message_id,
        invite_payload,
        auth_provider,
    )?;

    msg.insert_metadata(IS_MODERATOR.to_string(), TRUE_VAL.to_string());
    Ok(msg)
}

fn delete_channel_message(
    controller: &Name,
    moderator: &Name,
    channel_name: &Name,
    msg_id: u32,
    auth_provider: &Option<AuthProvider>,
) -> Result<DataPlaneMessage, ControllerError> {
    let session_id = generate_session_id(moderator, channel_name);

    let payload = Some(CommandPayload::builder().leave_request(None).as_content());

    let mut msg = create_channel_message(
        controller,
        moderator,
        ProtoSessionMessageType::LeaveRequest,
        session_id,
        msg_id,
        payload,
        auth_provider,
    )?;

    msg.insert_metadata(DELETE_GROUP.to_string(), TRUE_VAL.to_string());
    Ok(msg)
}

fn invite_participant_message(
    controller: &Name,
    moderator: &Name,
    participant: &Name,
    channel_name: &Name,
    msg_id: u32,
    auth_provider: &Option<AuthProvider>,
) -> Result<DataPlaneMessage, ControllerError> {
    let session_id = generate_session_id(moderator, channel_name);

    let payload = Some(
        CommandPayload::builder()
            .discovery_request(Some(participant.clone()))
            .as_content(),
    );

    let msg = create_channel_message(
        controller,
        moderator,
        ProtoSessionMessageType::DiscoveryRequest,
        session_id,
        msg_id,
        payload,
        auth_provider,
    )?;

    Ok(msg)
}

fn remove_participant_message(
    controller: &Name,
    moderator: &Name,
    participant: &Name,
    channel_name: &Name,
    msg_id: u32,
    auth_provider: &Option<AuthProvider>,
) -> Result<DataPlaneMessage, ControllerError> {
    let session_id = generate_session_id(moderator, channel_name);

    let payload = Some(
        CommandPayload::builder()
            .leave_request(Some(participant.clone()))
            .as_content(),
    );

    let msg = create_channel_message(
        controller,
        moderator,
        ProtoSessionMessageType::LeaveRequest,
        session_id,
        msg_id,
        payload,
        auth_provider,
    )?;

    Ok(msg)
}

impl ControllerService {
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
                        let mut connections_status = Vec::new();
                        let mut subscriptions_status = Vec::new();

                        // Process connections to create
                        for conn in &config.connections_to_create {
                            info!(?conn, "received a connection to create");
                            let mut connection_success = true;
                            let mut connection_error_msg = String::new();

                            match serde_json::from_str::<ClientConfig>(&conn.config_data) {
                                Err(e) => {
                                    connection_success = false;
                                    connection_error_msg = format!("Failed to parse config: {}", e);
                                }
                                Ok(client_config) => {
                                    let client_endpoint = &client_config.endpoint;

                                    // connect to an endpoint if it's not already connected
                                    if !self.inner.connections.read().contains_key(client_endpoint)
                                    {
                                        match self
                                            .inner
                                            .message_processor
                                            .connect(client_config.clone(), None, None)
                                            .await
                                        {
                                            Err(e) => {
                                                connection_success = false;
                                                connection_error_msg =
                                                    format!("Connection failed: {}", e);
                                            }
                                            Ok(conn_id) => {
                                                self.inner
                                                    .connections
                                                    .write()
                                                    .insert(client_endpoint.clone(), conn_id.1);
                                                info!(
                                                    endpoint = %client_endpoint, "Successfully created connection",

                                                );
                                            }
                                        }
                                    } else {
                                        info!(endpoint = %client_endpoint, "Connection already exists");
                                    }
                                }
                            }

                            // Add connection status
                            connections_status.push(v1::ConnectionAck {
                                connection_id: conn.connection_id.clone(),
                                success: connection_success,
                                error_msg: connection_error_msg,
                            });
                        }

                        // if the auth_provider is set try to get an identity
                        let identity_token = match &self.inner.auth_provider {
                            Some(auth) => auth.get_token()?,
                            None => String::new(),
                        };

                        // Process subscriptions to set
                        for subscription in &config.subscriptions_to_set {
                            let mut subscription_success = true;
                            let mut subscription_error_msg = String::new();

                            let conn = self
                                .inner
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned();

                            if let Some(conn) = conn {
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

                                let msg = DataPlaneMessage::builder()
                                    .source(source.clone())
                                    .destination(name.clone())
                                    .identity(&identity_token)
                                    .flags(SlimHeaderFlags::default().with_recv_from(conn))
                                    .build_subscribe()
                                    .unwrap();

                                let ack_id = self.next_subscription_ack_id();
                                if let Err(err) =
                                    self.send_subscription_message_with_ack(msg, ack_id).await
                                {
                                    subscription_success = false;
                                    subscription_error_msg =
                                        format!("Failed to subscribe: {}", err);
                                } else {
                                    info!(?subscription, "Successfully created subscription",);
                                }
                            } else {
                                subscription_success = false;
                                subscription_error_msg =
                                    format!("Connection {} not found", subscription.connection_id);
                            }

                            // Add subscription status
                            subscriptions_status.push(v1::SubscriptionAck {
                                subscription: Some(subscription.clone()),
                                success: subscription_success,
                                error_msg: subscription_error_msg,
                            });
                        }

                        // Process subscriptions to delete
                        for subscription in &config.subscriptions_to_delete {
                            let mut subscription_success = true;
                            let mut subscription_error_msg = String::new();

                            let conn = self
                                .inner
                                .connections
                                .read()
                                .get(&subscription.connection_id)
                                .cloned();

                            if let Some(conn) = conn {
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

                                let msg = DataPlaneMessage::builder()
                                    .source(source.clone())
                                    .destination(name.clone())
                                    .identity(&identity_token)
                                    .flags(SlimHeaderFlags::default().with_recv_from(conn))
                                    .build_unsubscribe()
                                    .unwrap();

                                let ack_id = self.next_subscription_ack_id();
                                if let Err(err) =
                                    self.send_subscription_message_with_ack(msg, ack_id).await
                                {
                                    subscription_success = false;
                                    subscription_error_msg =
                                        format!("Failed to unsubscribe: {}", err);
                                } else {
                                    info!(?subscription, "Successfully deleted subscription");
                                }
                            } else {
                                subscription_success = false;
                                subscription_error_msg =
                                    format!("Connection {} not found", subscription.connection_id);
                            }

                            // Add subscription status (for deletion)
                            subscriptions_status.push(v1::SubscriptionAck {
                                subscription: Some(subscription.clone()),
                                success: subscription_success,
                                error_msg: subscription_error_msg,
                            });
                        }

                        // Send ConfigurationCommandAck with detailed status information
                        let config_ack = v1::ConfigurationCommandAck {
                            original_message_id: msg.message_id.clone(),
                            connections_status,
                            subscriptions_status,
                        };

                        let reply = ControlMessage {
                            message_id: uuid::Uuid::new_v4().to_string(),
                            payload: Some(Payload::ConfigCommandAck(config_ack)),
                        };

                        if let Err(e) = tx.send(Ok(reply)).await {
                            error!(error = %e.chain(), "failed to send ConfigurationCommandAck");
                        }

                        info!(
                            connections = %config.connections_to_create.len(),
                            subscriptions_to_set = %config.subscriptions_to_set.len(),
                            subscriptions_to_del = %config.subscriptions_to_delete.len(),
                            "Processed ConfigurationCommand"
                        );
                    }
                    Payload::SubscriptionListRequest(_) => {
                        const CHUNK_SIZE: usize = 100;

                        let conn_table = self.inner.message_processor.connection_table();
                        let mut entries = Vec::new();

                        self.inner.message_processor.subscription_table().for_each(
                            |name, id, local, remote| {
                                let mut entry = SubscriptionEntry {
                                    component_0: name.components_strings()[0].to_string(),
                                    component_1: name.components_strings()[1].to_string(),
                                    component_2: name.components_strings()[2].to_string(),
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
                                        error!(%cid, "no connection entry for id");
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
                                        original_message_id: msg.message_id.clone(),
                                        entries: chunk.to_vec(),
                                    },
                                )),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!(error = %e.chain(), "failed to send subscription batch");
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
                                        original_message_id: msg.message_id.clone(),
                                        entries: chunk.to_vec(),
                                    },
                                )),
                            };

                            if let Err(e) = tx.try_send(Ok(resp)) {
                                error!(error = %e.chain(), "failed to send connection list batch");
                            }
                        }
                    }
                    Payload::Ack(_ack) => {
                        // received an ack, do nothing - this should not happen
                    }
                    Payload::ConfigCommandAck(_) => {
                        // received a config command ack, do nothing - this should not happen
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
                        info!("received a channel create request");

                        let mut success = true;
                        // Get the first moderator from the list, as we support only one for now
                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                error!("missing moderator ID");
                                success = false;
                            } else {
                                let channel_name = get_name_from_string(&req.channel_name)?;
                                let new_msg_id = rand::random::<u32>();
                                let controller_name = self.inner.controller_name.clone();
                                let creation_msg = new_channel_message(
                                    &controller_name,
                                    &moderator_name,
                                    &channel_name,
                                    new_msg_id,
                                    &self.inner.auth_provider,
                                )?;

                                debug!("send session creation message: {:?}", creation_msg);
                                if let Err(e) = self.send_control_message(creation_msg).await {
                                    error!(error = %e.chain(), "failed to send channel creation");
                                    success = false;
                                } else {
                                    // create timer for the message
                                    debug!(
                                        "create timer for message id: {} with type {:?}",
                                        new_msg_id,
                                        ProtoSessionMessageType::JoinRequest
                                    );
                                    let timer =
                                        self.inner.timer_factory.read().as_ref().map(|factory| {
                                            factory.create_and_start_timer(
                                                new_msg_id,
                                                ProtoSessionMessageType::JoinRequest,
                                                None,
                                            )
                                        });
                                    self.inner
                                        .message_id_map
                                        .write()
                                        .insert(new_msg_id, (msg.message_id.clone(), timer));
                                }
                            }
                        } else {
                            error!("no moderators specified in create channel request message");
                            success = false;
                        };

                        if !success {
                            let ack = Ack {
                                original_message_id: msg.message_id.clone(),
                                success,
                                messages: vec![msg.message_id.clone()],
                            };

                            let reply = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::Ack(ack)),
                            };

                            if let Err(e) = tx.send(Ok(reply)).await {
                                error!(error = %e.chain(), "failed to send ack");
                            }
                        }
                    }
                    Payload::DeleteChannelRequest(req) => {
                        info!("received a channel delete request");
                        let mut success = true;

                        // Get the first moderator from the list, as we support only one for now
                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                error!("missing moderator ID");
                                success = false;
                            } else {
                                let channel_name = get_name_from_string(&req.channel_name)?;
                                let new_msg_id = rand::random::<u32>();
                                let controller_name = self.inner.controller_name.clone();
                                let delete_msg = delete_channel_message(
                                    &controller_name,
                                    &moderator_name,
                                    &channel_name,
                                    new_msg_id,
                                    &self.inner.auth_provider,
                                )?;

                                debug!("Send delete session message: {:?}", delete_msg);
                                if let Err(e) = self.send_control_message(delete_msg).await {
                                    error!(error = %e.chain(), "failed to send delete channel");
                                    success = false;
                                } else {
                                    // create timer for the message
                                    debug!(
                                        "create timer for message id: {} with type {:?}",
                                        new_msg_id,
                                        ProtoSessionMessageType::LeaveRequest
                                    );
                                    let timer =
                                        self.inner.timer_factory.read().as_ref().map(|factory| {
                                            factory.create_and_start_timer(
                                                new_msg_id,
                                                ProtoSessionMessageType::LeaveRequest,
                                                None,
                                            )
                                        });

                                    self.inner
                                        .message_id_map
                                        .write()
                                        .insert(new_msg_id, (msg.message_id.clone(), timer));
                                }
                            }
                        } else {
                            error!("no moderators specified in delete channel request");
                            success = false;
                        };

                        if !success {
                            let ack = Ack {
                                original_message_id: msg.message_id.clone(),
                                success,
                                messages: vec![msg.message_id.clone()],
                            };

                            let reply = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::Ack(ack)),
                            };

                            if let Err(e) = tx.send(Ok(reply)).await {
                                error!(error = %e.chain(), "failed to send ack");
                            }
                        }
                    }
                    Payload::AddParticipantRequest(req) => {
                        info!(
                            channel_name = %req.channel_name,
                            participant_name = %req.participant_name,
                            "received a participant add request",
                        );

                        let mut success = true;

                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                error!("missing moderator ID");
                                success = false;
                            } else {
                                let channel_name = get_name_from_string(&req.channel_name)?;
                                let participant_name = get_name_from_string(&req.participant_name)?;
                                let new_msg_id = rand::random::<u32>();
                                let controller_name = self.inner.controller_name.clone();
                                let invite_msg = invite_participant_message(
                                    &controller_name,
                                    &moderator_name,
                                    &participant_name,
                                    &channel_name,
                                    new_msg_id,
                                    &self.inner.auth_provider,
                                )?;

                                debug!(?invite_msg, "Send invite participant");

                                if let Err(e) = self.send_control_message(invite_msg).await {
                                    error!(error = %e.chain(), "failed to send channel creation");
                                    success = false;
                                } else {
                                    // create timer for the message
                                    debug!(
                                        "create timer for message id: {} with type {:?}",
                                        new_msg_id,
                                        ProtoSessionMessageType::DiscoveryRequest
                                    );
                                    let timer =
                                        self.inner.timer_factory.read().as_ref().map(|factory| {
                                            factory.create_and_start_timer(
                                                new_msg_id,
                                                ProtoSessionMessageType::DiscoveryRequest,
                                                None,
                                            )
                                        });
                                    self.inner
                                        .message_id_map
                                        .write()
                                        .insert(new_msg_id, (msg.message_id.clone(), timer));
                                }
                            }
                        } else {
                            error!("no moderators specified in add participant request");
                        };

                        if !success {
                            let ack = Ack {
                                original_message_id: msg.message_id.clone(),
                                success,
                                messages: vec![msg.message_id.clone()],
                            };

                            let reply = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::Ack(ack)),
                            };

                            if let Err(e) = tx.send(Ok(reply)).await {
                                error!(error = %e.chain(), "failed to send ack");
                            }
                        }
                    }
                    Payload::DeleteParticipantRequest(req) => {
                        info!("received a participant delete request");

                        let mut success = true;

                        if let Some(first_moderator) = req.moderators.first() {
                            let moderator_name = get_name_from_string(first_moderator)?;
                            if !moderator_name.has_id() {
                                error!("missing moderator ID");
                                success = false;
                            } else {
                                let channel_name = get_name_from_string(&req.channel_name)?;
                                let participant_name = get_name_from_string(&req.participant_name)?;
                                let new_msg_id = rand::random::<u32>();
                                let controller_name = self.inner.controller_name.clone();
                                let remove_msg = remove_participant_message(
                                    &controller_name,
                                    &moderator_name,
                                    &participant_name,
                                    &channel_name,
                                    new_msg_id,
                                    &self.inner.auth_provider,
                                )?;

                                if let Err(e) = self.send_control_message(remove_msg).await {
                                    error!(error = %e.chain(), "failed to send delete participant request");
                                    success = false;
                                } else {
                                    // create timer for the message
                                    debug!(
                                        "create timer for message id: {} with type {:?}",
                                        new_msg_id,
                                        ProtoSessionMessageType::LeaveRequest
                                    );
                                    let timer =
                                        self.inner.timer_factory.read().as_ref().map(|factory| {
                                            factory.create_and_start_timer(
                                                new_msg_id,
                                                ProtoSessionMessageType::LeaveRequest,
                                                None,
                                            )
                                        });
                                    self.inner
                                        .message_id_map
                                        .write()
                                        .insert(new_msg_id, (msg.message_id.clone(), timer));
                                }
                            }
                        } else {
                            error!("no moderators specified in remove participant request");
                            success = false;
                        };

                        if !success {
                            let ack = Ack {
                                original_message_id: msg.message_id.clone(),
                                success,
                                messages: vec![msg.message_id.clone()],
                            };

                            let reply = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::Ack(ack)),
                            };

                            if let Err(e) = tx.send(Ok(reply)).await {
                                error!(error = %e.chain(), "failed to send ack");
                            }
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
                    message_id = %msg.message_id,
                    "received control message with no payload",
                );
            }
        }

        Ok(())
    }

    async fn handle_subscribe_message(&self, dst: Name, clients: &[ClientConfig]) {
        let mut sub_vec = vec![];

        let components = dst.components_strings();
        let cmd = v1::Subscription {
            component_0: components[0].to_string(),
            component_1: components[1].to_string(),
            component_2: components[2].to_string(),
            id: Some(dst.id()),
            connection_id: "n/a".to_string(),
            node_id: None,
        };

        sub_vec.push(cmd);

        let ctrl = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                subscriptions_to_set: sub_vec,
                subscriptions_to_delete: vec![],
            })),
        };

        return self.send_or_queue_notification(ctrl, clients).await;
    }

    async fn handle_unsubscribe_message(&self, dst: Name, clients: &[ClientConfig]) {
        let mut unsub_vec = vec![];

        let components = dst.components_strings();
        let cmd = v1::Subscription {
            component_0: components[0].to_string(),
            component_1: components[1].to_string(),
            component_2: components[2].to_string(),
            id: Some(dst.id()),
            connection_id: "n/a".to_string(),
            node_id: None,
        };

        unsub_vec.push(cmd);

        let ctrl = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: unsub_vec,
            })),
        };

        return self.send_or_queue_notification(ctrl, clients).await;
    }

    fn next_subscription_ack_id(&self) -> String {
        let next = self
            .inner
            .subscription_ack_counter
            .fetch_add(1, Ordering::Relaxed);
        format!("sub-{}", next)
    }

    fn register_subscription_ack(&self, ack_id: String) -> oneshot::Receiver<Result<(), String>> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.inner
            .pending_subscription_acks
            .lock()
            .insert(ack_id, ack_tx);
        ack_rx
    }

    fn remove_subscription_ack(&self, ack_id: &str) {
        self.inner.pending_subscription_acks.lock().remove(ack_id);
    }

    fn handle_subscription_ack_message(&self, msg: &DataPlaneMessage) -> bool {
        let Some(ack_id) = msg.get_metadata(SUBSCRIPTION_ACK_ID).cloned() else {
            return false;
        };

        let success = msg
            .get_metadata(SUBSCRIPTION_ACK_SUCCESS)
            .map(|val| val == TRUE_VAL)
            .unwrap_or(false);
        let error_msg = msg.get_metadata(SUBSCRIPTION_ACK_ERROR).cloned();

        let sender = self.inner.pending_subscription_acks.lock().remove(&ack_id);
        if let Some(sender) = sender {
            let _ = sender.send(if success {
                Ok(())
            } else {
                Err(error_msg.unwrap_or_else(|| "subscription ack failed".to_string()))
            });
        } else {
            debug!(
                ack_id = %ack_id,
                "received subscription ack with no pending waiter"
            );
        }

        true
    }

    async fn send_subscription_message_with_ack(
        &self,
        mut msg: DataPlaneMessage,
        ack_id: String,
    ) -> Result<(), String> {
        let ack_rx = self.register_subscription_ack(ack_id.clone());
        msg.insert_metadata(SUBSCRIPTION_ACK_ID.to_string(), ack_id.clone());

        if let Err(e) = self.send_control_message(msg).await {
            self.remove_subscription_ack(&ack_id);
            return Err(format!("datapath send error: {}", e.chain()));
        }

        match ack_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("subscription ack channel closed".to_string()),
        }
    }

    // send an ack back to the control plane. the success field indicates whether the original
    // operation was successfully delivered/processed or not.
    async fn send_ack_message(&self, msg_id: u32, success: bool, clients: &[ClientConfig]) {
        let original_message_id = self.inner.message_id_map.write().remove(&msg_id);
        match original_message_id {
            Some(entry) => {
                debug!("Received GroupAck for message ID: {}", entry.0);
                // stop timer and send ack
                if let Some(mut timer) = entry.1 {
                    timer.stop();
                }

                let ack = Ack {
                    original_message_id: entry.0,
                    success,
                    messages: vec![msg_id.to_string()],
                };

                let reply = ControlMessage {
                    message_id: uuid::Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(ack)),
                };

                self.send_or_queue_notification(reply, clients).await;
            }
            None => {
                debug!("Received GroupAck for unknown message ID: {}", msg_id);
            }
        }
    }

    /// Send a control message to SLIM.
    async fn send_control_message(&self, msg: DataPlaneMessage) -> Result<(), ControllerError> {
        self.inner.tx_slim.send(Ok(msg)).await.map_err(|e| {
            error!(error = %e.chain(), "error sending message into datapath");
            ControllerError::Datapath(slim_datapath::errors::DataPathError::ConnectionError)
        })
    }

    /// Send notification to control plane or queue it if no connection is available.
    async fn send_or_queue_notification(&self, ctrl_msg: ControlMessage, clients: &[ClientConfig]) {
        let mut has_active_connection = false;

        // Try to send to all active connections
        for c in clients {
            let tx = match self.inner.tx_channels.read().get(&c.endpoint) {
                Some(tx) => tx.clone(),
                None => continue,
            };

            if tx.send(Ok(ctrl_msg.clone())).await.is_ok() {
                has_active_connection = true;
            } else {
                debug!(
                    endpoint = %c.endpoint,
                    "failed to send notification to control plane"
                );
            }
        }

        // If no active connections, queue the notification
        if !has_active_connection {
            debug!("no active control plane connections, queuing subscription notification");

            let mut queue = self.inner.pending_notifications.lock();
            if queue.len() >= MAX_QUEUED_NOTIFICATIONS {
                // Remove oldest notification to make room for new one
                queue.remove(0);
                debug!("queue full, removed oldest notification");
            }
            queue.push(ctrl_msg);
        }
    }

    /// Get a drain watch clone to pass to a task
    fn drain_watch(&self) -> Result<drain::Watch, ControllerError> {
        self.inner
            .drain_watch
            .read()
            .clone()
            .ok_or(ControllerError::AlreadyStopped)
    }

    /// Send all queued subscription notifications when connection is restored.
    async fn send_queued_notifications(
        &self,
        tx: &mpsc::Sender<Result<ControlMessage, Status>>,
        endpoint: &str,
    ) {
        let notifications = {
            let mut queue = self.inner.pending_notifications.lock();
            if queue.is_empty() {
                return;
            }
            queue.drain(..).collect::<Vec<_>>()
        };

        if notifications.is_empty() {
            return;
        }

        debug!(
            "sending {} queued subscription notifications to {}",
            notifications.len(),
            endpoint
        );

        let mut failed_notifications = Vec::new();
        for notification in notifications {
            if let Err(e) = tx.send(Ok(notification)).await {
                error!(
                    error = %e.chain(),
                    %endpoint,
                    "failed to send queued notification to control plane",
                );

                // we can unwrap here because we know we sent a Ok(ControlMessage)
                failed_notifications.push(e.0.unwrap());
            }
        }

        // Re-queue any failed notifications
        if !failed_notifications.is_empty() {
            self.inner
                .pending_notifications
                .lock()
                .extend(failed_notifications);
        }
    }

    /// Process the control message stream.
    fn process_control_message_stream(
        &self,
        config: Option<ClientConfig>,
        mut stream: impl Stream<Item = Result<ControlMessage, Status>> + Unpin + Send + 'static,
        mut timer_rx: Option<mpsc::Receiver<SessionMessage>>,
        tx: mpsc::Sender<Result<ControlMessage, Status>>,
        cancellation_token: CancellationToken,
    ) -> Result<JoinHandle<()>, ControllerError> {
        let this = self.clone();
        let watch = self.drain_watch()?;
        let clients = config.clone();

        let handle = tokio::spawn(async move {
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
                error!(error = %e.chain(), "failed to send register request");
                return;
            }

            // TODO; here we should wait for an ack

            let mut drain_fut = std::pin::pin!(watch.clone().signaled());

            loop {
                tokio::select! {
                    next = stream.next() => {
                        match next {
                            Some(Ok(msg)) => {
                                if let Err(e) = this.handle_new_control_message(msg, &tx).await {
                                    error!(error = %e.chain(), "error processing incoming control message");
                                }
                            }
                            Some(Err(e)) => {
                                if let Some(io_err) = Self::match_for_io_error(&e) {
                                    if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                                        info!("connection closed by peer");
                                        retry_connect = true;
                                    }
                                } else {
                                    error!(error = %e.chain(), "error receiving control messages");
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
                    Some(session_msg) = async {
                        match &mut timer_rx {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match session_msg {
                            SessionMessage::TimerFailure { message_id, message_type: _, name: _, timeouts: _} => {
                                tracing::info!("got a failure for message id: {}", message_id);
                                // if there's a timer the clientconfig is always set
                                if let Some(clients) = &clients {
                                    this.send_ack_message(message_id, false, std::slice::from_ref(clients)).await;
                                }
                            }
                            _ => {
                                error!("unexpected session message received in controller");
                            }
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        debug!("shutting down stream on cancellation token");
                        break;
                    }
                    _ = &mut drain_fut => {
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
                            error!(error = %e.chain(), "failed to reconnect to control plane");
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
        });

        Ok(handle)
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

        let channel = config.to_channel().await?;

        let mut client = ControllerServiceClient::new(channel.clone());
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
        let out_stream = ReceiverStream::new(rx).map(|res| res.expect("mapping error"));
        let stream = client
            .open_control_channel(Request::new(out_stream))
            .await?;

        self.send_queued_notifications(&tx, &config.endpoint).await;

        let timer_settings = TimerSettings::new(
            Duration::from_millis(2000),
            None,
            Some(0),
            TimerType::Constant,
        );
        let (timer_tx, timer_rx) = mpsc::channel::<SessionMessage>(128);
        let timer_factory = TimerFactory::new(timer_settings, timer_tx.clone());
        self.inner.timer_factory.write().replace(timer_factory);

        // start processing the incoming stream
        self.process_control_message_stream(
            Some(config),
            stream.into_inner(),
            Some(timer_rx),
            tx.clone(),
            cancellation_token.clone(),
        )?;

        // return the sender
        Ok(tx)
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

        // Server-side connections don't initiate operations requiring acks, so no timer channel needed
        self.process_control_message_stream(
            None,
            stream,
            None,
            tx.clone(),
            cancellation_token.clone(),
        )
        .map_err(|e| {
            error!(error = %e.chain(), "error processing control message stream");
            Status::unavailable("failed to process control message stream")
        })?;

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
    use slim_auth::shared_secret::SharedSecret;
    use slim_config::component::id::Kind;
    use slim_testing::utils::TEST_VALID_SECRET;
    use tracing_test::traced_test;

    /// Helper to build a server and client control plane pair with shared-secret auth.
    async fn setup_control_planes(
        server_endpoint: &str,
        server_name: &str,
        client_name: &str,
    ) -> (ControlPlane, ControlPlane, ClientConfig) {
        let id_server = ID::new_with_name(Kind::new("slim").unwrap(), server_name).unwrap();
        let id_client = ID::new_with_name(Kind::new("slim").unwrap(), client_name).unwrap();

        let server_config = ServerConfig::with_endpoint(server_endpoint)
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure());
        let client_config = ClientConfig::with_endpoint(&format!("http://{}", server_endpoint))
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure());

        let message_processor_server = MessageProcessor::new();
        let message_processor_client = MessageProcessor::new();

        let control_plane_server = ControlPlane::new(ControlPlaneSettings {
            id: id_server,
            group_name: None,
            servers: vec![server_config.clone()],
            clients: vec![],
            message_processor: Arc::new(message_processor_server),
            auth_provider: Some(AuthProvider::SharedSecret(
                SharedSecret::new("server", TEST_VALID_SECRET).unwrap(),
            )),
            auth_verifier: Some(AuthVerifier::SharedSecret(
                SharedSecret::new("server", TEST_VALID_SECRET).unwrap(),
            )),
            connection_details: vec![from_server_config(&server_config)],
        });

        let control_plane_client = ControlPlane::new(ControlPlaneSettings {
            id: id_client,
            group_name: None,
            servers: vec![],
            clients: vec![client_config.clone()],
            message_processor: Arc::new(message_processor_client),
            auth_provider: Some(AuthProvider::SharedSecret(
                SharedSecret::new("client", TEST_VALID_SECRET).unwrap(),
            )),
            auth_verifier: Some(AuthVerifier::SharedSecret(
                SharedSecret::new("client", TEST_VALID_SECRET).unwrap(),
            )),
            connection_details: vec![],
        });

        (control_plane_server, control_plane_client, client_config)
    }

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end() {
        let (mut control_plane_server, mut control_plane_client, _client_cfg) =
            setup_control_planes(
                "127.0.0.1:50051",
                "test-server-instance",
                "test-client-instance",
            )
            .await;

        control_plane_server.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        control_plane_client.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(logs_contain("received a register node request"));
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

    #[tokio::test]
    #[traced_test]
    async fn test_subscription_notification_queue_drain() {
        // Reuse common setup with a different port to avoid clash with other tests.
        let (mut control_plane_server, mut control_plane_client, client_config) =
            setup_control_planes(
                "127.0.0.1:50061",
                "queue-drain-server",
                "queue-drain-client",
            )
            .await;

        let controller = control_plane_client.controller.clone();
        assert_eq!(controller.inner.pending_notifications.lock().len(), 0);

        const N: usize = 5;
        for i in 0..N {
            let ctrl_msg = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                    connections_to_create: vec![],
                    subscriptions_to_set: vec![v1::Subscription {
                        component_0: "queued".to_string(),
                        component_1: "sub".to_string(),
                        component_2: format!("name-{i}"),
                        id: Some(i as u64),
                        connection_id: "test-conn".to_string(),
                        node_id: None,
                    }],
                    subscriptions_to_delete: vec![],
                })),
            };
            controller
                .send_or_queue_notification(ctrl_msg, std::slice::from_ref(&client_config))
                .await;
        }
        assert_eq!(controller.inner.pending_notifications.lock().len(), N);

        control_plane_server.run().await.expect("server run failed");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        control_plane_client.run().await.expect("client run failed");
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        assert_eq!(controller.inner.pending_notifications.lock().len(), 0);
        assert!(
            logs_contain(&format!("sending {} queued subscription notifications", N)),
            "Expected log about sending queued subscription notifications"
        );

        drop(controller);
        drop(control_plane_server);
        drop(control_plane_client);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_drains_resources() {
        // Use a unique port to avoid conflicts with other tests.
        let (mut control_plane_server, mut control_plane_client, _client_cfg) =
            setup_control_planes(
                "127.0.0.1:50071",
                "shutdown-server-instance",
                "shutdown-client-instance",
            )
            .await;

        // Run both ends to populate cancellation tokens.
        control_plane_server
            .run()
            .await
            .expect("server should start");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        control_plane_client
            .run()
            .await
            .expect("client should start");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Ensure we have at least one cancellation token (server or client side tasks).
        let server_tokens_before = control_plane_server
            .controller
            .inner
            .cancellation_tokens
            .read()
            .len();
        assert!(
            server_tokens_before > 0,
            "expected server to have active cancellation tokens before shutdown"
        );

        let client_tokens_before = control_plane_client
            .controller
            .inner
            .cancellation_tokens
            .read()
            .len();
        assert!(
            client_tokens_before > 0,
            "expected client to have active cancellation tokens before shutdown"
        );

        // Perform shutdown on both.
        control_plane_client
            .shutdown()
            .await
            .expect("client shutdown ok");
        control_plane_server
            .shutdown()
            .await
            .expect("server shutdown ok");

        // After shutdown, all cancellation tokens should be drained.
        let server_tokens_after = control_plane_server
            .controller
            .inner
            .cancellation_tokens
            .read()
            .len();
        assert_eq!(
            server_tokens_after, 0,
            "expected server cancellation tokens to be drained after shutdown"
        );

        let client_tokens_after = control_plane_client
            .controller
            .inner
            .cancellation_tokens
            .read()
            .len();
        assert_eq!(
            client_tokens_after, 0,
            "expected client cancellation tokens to be drained after shutdown"
        );

        // Second shutdown should error because drain_signal has been taken.
        assert!(
            control_plane_server.shutdown().await.is_err(),
            "second shutdown on server should return an error"
        );
        assert!(
            control_plane_client.shutdown().await.is_err(),
            "second shutdown on client should return an error"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_shutdown_without_run() {
        // Build a control plane but do NOT call run()
        let (control_plane_server, mut _control_plane_client, _client_cfg) = setup_control_planes(
            "127.0.0.1:50072",
            "shutdown-no-run-server",
            "shutdown-no-run-client",
        )
        .await;

        // No tasks should be registered yet
        assert_eq!(
            control_plane_server
                .controller
                .inner
                .cancellation_tokens
                .read()
                .len(),
            0,
            "expected zero cancellation tokens before shutdown when not run"
        );

        // Shutdown should still succeed gracefully.
        control_plane_server
            .shutdown()
            .await
            .expect("shutdown without prior run should succeed");

        // Tokens remain zero.
        assert_eq!(
            control_plane_server
                .controller
                .inner
                .cancellation_tokens
                .read()
                .len(),
            0,
            "expected zero cancellation tokens after shutdown when not run"
        );

        // Second shutdown should fail.
        assert!(
            control_plane_server.shutdown().await.is_err(),
            "second shutdown should error due to missing drain signal"
        );
    }
}

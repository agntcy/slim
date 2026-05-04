// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;

use std::time::Duration;
use std::vec;

use display_error_chain::ErrorChainExt;
use slim_config::component::id::ID;
use slim_config::grpc::server::ServerConfig;
use slim_session::SessionMessage;
use slim_session::subscription_manager::SubscriptionManager;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::api::proto::api::v1::control_message::Payload;
use crate::api::proto::api::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::api::v1::{
    self, ConnectionDetails, ConnectionDirection, ConnectionListResponse, ConnectionType,
    SubscriptionListResponse,
};
use crate::api::proto::api::v1::{
    Ack, ConnectionEntry, ControlMessage, SubscriptionEntry,
    controller_service_client::ControllerServiceClient,
    controller_service_server::ControllerService as GrpcControllerService,
};
use crate::errors::ControllerError;
use prost_types::Struct;
use slim_config::grpc::client::ClientConfig;
use slim_datapath::api::ProtoSessionMessageType;
use slim_datapath::api::{
    MessageType::Link as LinkType, MessageType::Publish, MessageType::Subscribe,
    MessageType::SubscriptionAck as SubscriptionAckType, MessageType::Unsubscribe,
    ProtoMessage as DataPlaneMessage,
};
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_datapath::tables::SubscriptionTable;

use slim_session::timer::{Timer, TimerType};
use slim_session::timer_factory::{TimerFactory, TimerSettings};

type TxChannel = mpsc::Sender<Result<ControlMessage, Status>>;
type TxChannels = HashMap<String, TxChannel>;

// Controller component
const CONTROLLER_COMPONENT: &str = "controller";
/// Maximum number of queued subscription notifications
const MAX_QUEUED_NOTIFICATIONS: usize = 1000;

/// Timeout for waiting on a subscription ack from the datapath.
const SUBSCRIPTION_ACK_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// channel to send messages into the datapath
    tx_slim: mpsc::Sender<Result<DataPlaneMessage, Status>>,

    /// channels to send control messages
    tx_channels: parking_lot::RwLock<TxChannels>,

    /// cancellation token for graceful shutdown
    cancellation_tokens: parking_lot::RwLock<HashMap<String, CancellationToken>>,

    /// drain watch channel
    drain_watch: parking_lot::RwLock<Option<drain::Watch>>,

    /// queue for pending subscription notifications when connections are down
    pending_notifications: Arc<parking_lot::Mutex<VecDeque<ControlMessage>>>,

    /// Manages pending subscription ack tracking (id generation, registration, resolution).
    subscription_manager: SubscriptionManager,

    /// Maps generated u32 keys to original string message IDs and their associated timers.
    /// u32 keys have a ~0.01% collision probability at ~650 concurrent entries (birthday bound).
    /// Entries are short-lived (removed on ack or timeout), so practical risk is negligible.
    message_id_map: Arc<parking_lot::RwLock<HashMap<u32, (String, Option<Timer>)>>>,

    /// timer factory for controller messages
    /// used to create timers for messages that require timeouts
    /// the lock is needed to set the timer factory after initialization
    /// because it requires a channel to send session messages
    timer_factory: parking_lot::RwLock<Option<TimerFactory>>,

    /// connection details used by control plane to store connection settings
    connection_details: Vec<ConnectionDetails>,

    /// Maps (subscription_name, connection_id) → subscription_id for route tracking
    route_subscription_ids: parking_lot::Mutex<HashMap<(Name, u64), u64>>,

    /// Reverse index: link_id → connection_id for O(1) lookup.
    /// Populated lazily on first resolve and eagerly on connection create/delete.
    link_id_to_conn_id: parking_lot::RwLock<HashMap<String, u64>>,

    /// JoinHandles for control-plane stream processing tasks, keyed by endpoint.
    stream_handles: parking_lot::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
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
                    subscription_manager: SubscriptionManager::new(tx_slim.clone()),
                    tx_slim,
                    tx_channels: parking_lot::RwLock::new(HashMap::new()),
                    cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
                    drain_watch: parking_lot::RwLock::new(Some(watch)),
                    pending_notifications: Arc::new(parking_lot::Mutex::new(VecDeque::new())),
                    message_id_map: Arc::new(parking_lot::RwLock::new(HashMap::new())),
                    timer_factory: parking_lot::RwLock::new(None),
                    connection_details: config.connection_details,
                    route_subscription_ids: parking_lot::Mutex::new(HashMap::new()),
                    link_id_to_conn_id: parking_lot::RwLock::new(HashMap::new()),
                    stream_handles: parking_lot::Mutex::new(HashMap::new()),
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

    pub async fn deregister(&self) -> Result<(), ControllerError> {
        let node_id = self.controller.inner.id.to_string();
        let deregister_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::DeregisterNodeRequest(v1::DeregisterNodeRequest {
                node: Some(v1::Node { id: node_id }),
            })),
        };
        let channels: Vec<(String, TxChannel)> = self
            .controller
            .inner
            .tx_channels
            .read()
            .iter()
            .map(|(ep, tx)| (ep.clone(), tx.clone()))
            .collect();
        for (endpoint, tx) in channels {
            if let Err(e) = tx.send(Ok(deregister_msg.clone())).await {
                error!(%endpoint, error = %e, "failed to send deregister request");
            }
        }
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
                                            LinkType(_) => {
                                                debug!("received link message from dataplane - this should not happen");
                                            }
                                            SubscriptionAckType(_) => {
                                                controller.inner.subscription_manager.resolve_ack(msg.get_subscription_ack());
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

impl ControllerService {
    fn resolve_connection_by_link_id(&self, link_id: &str) -> Result<Option<u64>, String> {
        let cached = self.inner.link_id_to_conn_id.read().get(link_id).copied();
        if let Some(conn_id) = cached {
            if self
                .inner
                .message_processor
                .connection_table()
                .get(conn_id)
                .is_some_and(|conn| conn.link_id().as_deref() == Some(link_id))
            {
                return Ok(Some(conn_id));
            }
            self.inner.link_id_to_conn_id.write().remove(link_id);
        }

        let mut resolved: Option<u64> = None;
        self.inner
            .message_processor
            .connection_table()
            .for_each(|id, conn| {
                if conn.link_id().as_deref() == Some(link_id) && resolved.is_none() {
                    resolved = Some(id);
                }
            });

        if let Some(conn_id) = resolved {
            self.inner
                .link_id_to_conn_id
                .write()
                .insert(link_id.to_string(), conn_id);
        }

        Ok(resolved)
    }

    fn disconnect_connection_by_link_id(&self, link_id: &str) -> Result<(), String> {
        if link_id.trim().is_empty() {
            return Err("link_id cannot be empty".to_string());
        }

        let conn_id = match self.resolve_connection_by_link_id(link_id)? {
            Some(id) => id,
            None => {
                return Err(format!("Connection with link_id {} not found", link_id));
            }
        };

        if let Err(e) = self.inner.message_processor.disconnect(conn_id) {
            // Best-effort delete: local/control-plane connections can lack config_data.
            info!(
                link_id = %link_id,
                conn_id,
                error = %e,
                "Disconnect returned an error; continuing delete flow"
            );
        }

        self.inner.link_id_to_conn_id.write().remove(link_id);
        self.inner
            .route_subscription_ids
            .lock()
            .retain(|(_name, cid), _| *cid != conn_id);

        info!(link_id = %link_id, conn_id, "Successfully deleted connection by link_id");
        Ok(())
    }

    fn resolve_subscription_connection(
        &self,
        subscription: &v1::Subscription,
    ) -> Result<Option<u64>, String> {
        if let Some(link_id) = &subscription.link_id {
            let trimmed = link_id.trim();
            if !trimmed.is_empty() {
                return self.resolve_connection_by_link_id(trimmed);
            }
        }

        Ok(None)
    }

    async fn diff_connections(&self, desired: &v1::DesiredState) -> Vec<v1::ConnectionAck> {
        let mut connections_status = Vec::new();

        let desired_link_ids: std::collections::HashSet<String> = desired
            .desired_connections
            .iter()
            .map(|c| c.connection_id.clone())
            .collect();

        let mut live_outgoing_link_ids: Vec<String> = Vec::new();
        self.inner
            .message_processor
            .connection_table()
            .for_each(|_id, conn| {
                if conn.is_outgoing()
                    && let Some(lid) = conn.link_id()
                    && !lid.is_empty()
                {
                    live_outgoing_link_ids.push(lid);
                }
            });

        for link_id in &live_outgoing_link_ids {
            if desired_link_ids.contains(link_id) {
                continue;
            }
            info!(link_id = %link_id, "desired state: removing connection");
            let mut success = true;
            let mut error_msg = String::new();
            if let Err(err) = self.disconnect_connection_by_link_id(link_id) {
                success = false;
                error_msg = err;
            }
            connections_status.push(v1::ConnectionAck {
                connection_id: link_id.clone(),
                success,
                error_msg,
            });
        }

        for conn in &desired.desired_connections {
            let link_id = &conn.connection_id;
            if link_id.is_empty() {
                continue;
            }

            let already_exists = match self.resolve_connection_by_link_id(link_id) {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(err) => {
                    connections_status.push(v1::ConnectionAck {
                        connection_id: link_id.clone(),
                        success: false,
                        error_msg: err,
                    });
                    continue;
                }
            };

            if already_exists {
                connections_status.push(v1::ConnectionAck {
                    connection_id: link_id.clone(),
                    success: true,
                    error_msg: String::new(),
                });
                continue;
            }

            info!(?conn, "desired state: creating connection");
            let mut success = true;
            let mut error_msg = String::new();

            match serde_json::from_str::<ClientConfig>(&conn.config_data) {
                Err(e) => {
                    success = false;
                    error_msg = format!("Failed to parse config: {}", e);
                }
                Ok(client_config) => {
                    match self
                        .inner
                        .message_processor
                        .connect(client_config.clone(), None, None)
                        .await
                    {
                        Err(e) => {
                            success = false;
                            error_msg = format!("Connection failed: {}", e);
                        }
                        Ok(conn_id) => {
                            let requested_link_id = if client_config.link_id.trim().is_empty() {
                                String::new()
                            } else {
                                client_config.link_id.clone()
                            };
                            if !requested_link_id.is_empty() {
                                self.inner
                                    .link_id_to_conn_id
                                    .write()
                                    .insert(requested_link_id, conn_id.1);
                            }
                            info!(
                                link_id = %link_id,
                                "Successfully created connection"
                            );
                        }
                    }
                }
            }

            connections_status.push(v1::ConnectionAck {
                connection_id: link_id.clone(),
                success,
                error_msg,
            });
        }

        connections_status
    }

    fn resolve_desired_subscriptions<'a>(
        &self,
        desired_subscriptions: &'a [v1::Subscription],
    ) -> (
        HashMap<(Name, u64), &'a v1::Subscription>,
        Vec<v1::SubscriptionAck>,
    ) {
        type SubKey = (Name, u64);
        let mut desired_subs: HashMap<SubKey, &v1::Subscription> = HashMap::new();
        let mut failures: Vec<v1::SubscriptionAck> = Vec::new();

        for sub in desired_subscriptions {
            match self.resolve_subscription_connection(sub) {
                Ok(Some(conn_id)) => {
                    let name = Name::from_strings([
                        sub.component_0.as_str(),
                        sub.component_1.as_str(),
                        sub.component_2.as_str(),
                    ])
                    .with_id(sub.id.unwrap_or(Name::NULL_COMPONENT));
                    desired_subs.insert((name, conn_id), sub);
                }
                Ok(None) => {
                    failures.push(v1::SubscriptionAck {
                        subscription: Some(sub.clone()),
                        success: false,
                        error_msg: "connection not found".to_string(),
                    });
                }
                Err(err) => {
                    failures.push(v1::SubscriptionAck {
                        subscription: Some(sub.clone()),
                        success: false,
                        error_msg: err,
                    });
                }
            }
        }

        (desired_subs, failures)
    }

    async fn delete_stale_subscriptions(
        &self,
        desired_subs: &HashMap<(Name, u64), &v1::Subscription>,
    ) -> Vec<v1::SubscriptionAck> {
        let active_subs: Vec<((Name, u64), u64)> = self
            .inner
            .route_subscription_ids
            .lock()
            .iter()
            .map(|((name, conn_id), sub_id)| ((name.clone(), *conn_id), *sub_id))
            .collect();

        let stale: Vec<_> = active_subs
            .into_iter()
            .filter(|((name, conn_id), _)| !desired_subs.contains_key(&(name.clone(), *conn_id)))
            .collect();

        let futs = stale.iter().map(|((name, conn_id), sub_id)| {
            let name = name.clone();
            let conn_id = *conn_id;
            let sub_id = *sub_id;
            async move {
                let conn_alive = self
                    .inner
                    .message_processor
                    .connection_table()
                    .get(conn_id)
                    .is_some();

                let (success, error_msg) = if conn_alive {
                    let source = name.clone().with_id(0);
                    let unsub_msg = DataPlaneMessage::builder()
                        .source(source)
                        .destination(name.clone())
                        .identity("")
                        .flags(SlimHeaderFlags::default().with_recv_from(conn_id))
                        .build_unsubscribe()
                        .unwrap();

                    match self
                        .send_unsubscribe_message_with_ack(unsub_msg, sub_id)
                        .await
                    {
                        Ok(()) => (true, String::new()),
                        Err(err) => (false, format!("Failed to unsubscribe: {}", err)),
                    }
                } else {
                    (true, String::new())
                };

                (name, conn_id, success, error_msg)
            }
        });

        let results = futures::future::join_all(futs).await;

        let mut subscriptions_status = Vec::with_capacity(results.len());
        for (name, conn_id, success, error_msg) in results {
            if success {
                self.inner
                    .route_subscription_ids
                    .lock()
                    .remove(&(name.clone(), conn_id));
            }

            let components = name.components_strings();
            subscriptions_status.push(v1::SubscriptionAck {
                subscription: Some(v1::Subscription {
                    component_0: components[0].to_string(),
                    component_1: components[1].to_string(),
                    component_2: components[2].to_string(),
                    id: Some(name.id()),
                    connection_id: String::new(),
                    link_id: None,
                    node_id: None,
                    direction: None,
                }),
                success,
                error_msg,
            });
        }

        subscriptions_status
    }

    async fn create_new_subscriptions(
        &self,
        desired_subs: &HashMap<(Name, u64), &v1::Subscription>,
    ) -> Vec<v1::SubscriptionAck> {
        let mut subscriptions_status = Vec::new();

        let to_create: Vec<((Name, u64), v1::Subscription)> = desired_subs
            .iter()
            .filter(|((name, conn_id), _)| {
                !self
                    .inner
                    .route_subscription_ids
                    .lock()
                    .contains_key(&(name.clone(), *conn_id))
            })
            .map(|((name, conn_id), sub)| ((name.clone(), *conn_id), (*sub).clone()))
            .collect();

        // Report already-active subscriptions as success.
        for ((name, conn_id), sub) in desired_subs {
            let dominated = to_create
                .iter()
                .any(|((n, c), _)| n == name && *c == *conn_id);
            if !dominated {
                subscriptions_status.push(v1::SubscriptionAck {
                    subscription: Some((*sub).clone()),
                    success: true,
                    error_msg: String::new(),
                });
            }
        }

        let futs = to_create.iter().map(|((name, conn_id), sub)| {
            let name = name.clone();
            let conn_id = *conn_id;
            let sub = sub.clone();
            async move {
                let source = name.clone().with_id(0);
                let sub_msg = DataPlaneMessage::builder()
                    .source(source)
                    .destination(name.clone())
                    .identity("")
                    .flags(SlimHeaderFlags::default().with_recv_from(conn_id))
                    .build_subscribe()
                    .unwrap();

                let result = self.send_subscribe_message_with_ack(sub_msg).await;
                (name, conn_id, sub, result)
            }
        });

        let results = futures::future::join_all(futs).await;

        for (name, conn_id, sub, result) in results {
            let (success, error_msg) = match result {
                Ok(subscription_id) => {
                    self.inner
                        .route_subscription_ids
                        .lock()
                        .insert((name, conn_id), subscription_id);
                    info!(?sub, "desired state: created subscription");
                    (true, String::new())
                }
                Err(err) => (false, format!("Failed to subscribe: {}", err)),
            };

            subscriptions_status.push(v1::SubscriptionAck {
                subscription: Some(sub),
                success,
                error_msg,
            });
        }

        subscriptions_status
    }

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

                        // Process connections to delete by link_id.
                        for link_id in &config.connections_to_delete {
                            info!(link_id = %link_id, "received a connection to delete");
                            let mut connection_success = true;
                            let mut connection_error_msg = String::new();

                            if let Err(err) = self.disconnect_connection_by_link_id(link_id) {
                                connection_success = false;
                                connection_error_msg = err;
                            }

                            connections_status.push(v1::ConnectionAck {
                                connection_id: link_id.clone(),
                                success: connection_success,
                                error_msg: connection_error_msg,
                            });
                        }

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
                                    let requested_link_id =
                                        if client_config.link_id.trim().is_empty() {
                                            String::new()
                                        } else {
                                            client_config.link_id.clone()
                                        };
                                    let mut existing_conn_for_link_id = false;

                                    if !requested_link_id.is_empty() {
                                        match self.resolve_connection_by_link_id(&requested_link_id)
                                        {
                                            Err(err) => {
                                                connection_success = false;
                                                connection_error_msg = err;
                                            }
                                            Ok(Some(conn_id)) => {
                                                existing_conn_for_link_id = true;
                                                self.inner
                                                    .link_id_to_conn_id
                                                    .write()
                                                    .insert(requested_link_id.clone(), conn_id);
                                                info!(
                                                    link_id = %requested_link_id,
                                                    conn_id,
                                                    "Connection already exists for link_id"
                                                );
                                            }
                                            Ok(None) => {}
                                        }
                                    }

                                    if connection_success && !existing_conn_for_link_id {
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
                                                if !requested_link_id.is_empty() {
                                                    self.inner.link_id_to_conn_id.write().insert(
                                                        requested_link_id.clone(),
                                                        conn_id.1,
                                                    );
                                                }
                                                info!(
                                                    endpoint = %client_endpoint, "Successfully created connection",
                                                );
                                            }
                                        }
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

                        // Process subscriptions to set
                        for subscription in &config.subscriptions_to_set {
                            let mut subscription_success = true;
                            let mut subscription_error_msg = String::new();

                            let conn = self.resolve_subscription_connection(subscription);

                            if let Ok(Some(conn)) = conn {
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
                                    .identity("")
                                    .flags(SlimHeaderFlags::default().with_recv_from(conn))
                                    .build_subscribe()
                                    .unwrap();

                                match self.send_subscribe_message_with_ack(msg).await {
                                    Ok(subscription_id) => {
                                        // Store the subscription_id for later unsubscription
                                        self.inner
                                            .route_subscription_ids
                                            .lock()
                                            .insert((name.clone(), conn), subscription_id);
                                        info!(?subscription, "Successfully created subscription");
                                    }
                                    Err(err) => {
                                        subscription_success = false;
                                        subscription_error_msg =
                                            format!("Failed to subscribe: {}", err);
                                    }
                                }
                            } else {
                                subscription_success = false;
                                subscription_error_msg = match conn {
                                    Ok(None) => {
                                        if let Some(link_id) = &subscription.link_id {
                                            format!("Connection with link_id {} not found", link_id)
                                        } else {
                                            format!(
                                                "Connection {} not found",
                                                subscription.connection_id
                                            )
                                        }
                                    }
                                    Err(err) => err,
                                    _ => "unknown connection lookup error".to_string(),
                                };
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

                            let conn = self.resolve_subscription_connection(subscription);

                            if let Ok(Some(conn)) = conn {
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
                                    .identity("")
                                    .flags(SlimHeaderFlags::default().with_recv_from(conn))
                                    .build_unsubscribe()
                                    .unwrap();

                                let sub_id = self
                                    .inner
                                    .route_subscription_ids
                                    .lock()
                                    .remove(&(name.clone(), conn));
                                let unsubscribe_result = match sub_id {
                                    Some(subscription_id) => {
                                        self.send_unsubscribe_message_with_ack(msg, subscription_id)
                                            .await
                                    }
                                    None => {
                                        // No tracking ID in the map (e.g. after a CP restart
                                        // or for orphan cleanup from the reconciler).  Generate
                                        // a fresh ID — the datapath locates the subscription
                                        // by Name+connection, not by ID; the ID is only used
                                        // for ack correlation.
                                        let (ack_id, ack_rx) =
                                            self.inner.subscription_manager.register_ack();
                                        let mut fresh_msg = msg;
                                        fresh_msg.set_subscription_id(ack_id);
                                        if let Err(e) = self.send_control_message(fresh_msg).await {
                                            self.inner.subscription_manager.cancel_ack(ack_id);
                                            Err(format!("datapath send error: {}", e.chain()))
                                        } else {
                                            match tokio::time::timeout(
                                                SUBSCRIPTION_ACK_TIMEOUT,
                                                ack_rx,
                                            )
                                            .await
                                            {
                                                Ok(Ok(Ok(()))) => Ok(()),
                                                Ok(Ok(Err(err))) => Err(err.to_string()),
                                                Ok(Err(_)) => {
                                                    Err("subscription ack channel closed"
                                                        .to_string())
                                                }
                                                Err(_) => {
                                                    self.inner
                                                        .subscription_manager
                                                        .cancel_ack(ack_id);
                                                    Err("subscription ack timed out".to_string())
                                                }
                                            }
                                        }
                                    }
                                };
                                if let Err(err) = unsubscribe_result {
                                    subscription_success = false;
                                    subscription_error_msg =
                                        format!("Failed to unsubscribe: {}", err);
                                } else {
                                    info!(?subscription, "Successfully deleted subscription");
                                }
                            } else {
                                subscription_success = false;
                                subscription_error_msg = match conn {
                                    Ok(None) => {
                                        if let Some(link_id) = &subscription.link_id {
                                            format!("Connection with link_id {} not found", link_id)
                                        } else {
                                            format!(
                                                "Connection {} not found",
                                                subscription.connection_id
                                            )
                                        }
                                    }
                                    Err(err) => err,
                                    _ => "unknown connection lookup error".to_string(),
                                };
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
                            connections_to_delete = %config.connections_to_delete.len(),
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
                                        link_id: None,
                                        direction: ConnectionDirection::Outgoing as i32,
                                    });
                                }

                                for &cid in remote {
                                    if let Some(conn) = conn_table.get(cid) {
                                        entry.remote_connections.push(ConnectionEntry {
                                            id: cid,
                                            connection_type: ConnectionType::Remote as i32,
                                            config_data: match conn.config_data() {
                                                Some(data) => serde_json::to_string(data)
                                                    .unwrap_or_else(|_| "{}".to_string()),
                                                None => "{}".to_string(),
                                            },
                                            link_id: conn.link_id(),
                                            direction: if conn.is_outgoing() {
                                                ConnectionDirection::Outgoing as i32
                                            } else {
                                                ConnectionDirection::Incoming as i32
                                            },
                                        });
                                    } else {
                                        error!(%cid, "no connection entry for id");
                                    }
                                }
                                entries.push(entry);
                            },
                        );

                        let chunks: Vec<_> = entries.chunks(CHUNK_SIZE).collect();
                        if chunks.is_empty() {
                            let resp = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::SubscriptionListResponse(
                                    SubscriptionListResponse {
                                        original_message_id: msg.message_id.clone(),
                                        entries: vec![],
                                        done: true,
                                    },
                                )),
                            };
                            if let Err(e) = tx.send(Ok(resp)).await {
                                error!(error = %e.chain(), "failed to send subscription list response");
                            }
                        } else {
                            let n = chunks.len();
                            for (i, chunk) in chunks.into_iter().enumerate() {
                                let resp = ControlMessage {
                                    message_id: uuid::Uuid::new_v4().to_string(),
                                    payload: Some(Payload::SubscriptionListResponse(
                                        SubscriptionListResponse {
                                            original_message_id: msg.message_id.clone(),
                                            entries: chunk.to_vec(),
                                            done: i + 1 == n,
                                        },
                                    )),
                                };
                                if let Err(e) = tx.send(Ok(resp)).await {
                                    error!(error = %e.chain(), "failed to send subscription batch");
                                    break;
                                }
                            }
                        }
                    }
                    Payload::ConnectionListRequest(_) => {
                        let mut all_entries = Vec::new();
                        self.inner
                            .message_processor
                            .connection_table()
                            .for_each(|id, conn| {
                                debug!(
                                    conn_id = id,
                                    local_addr = ?conn.local_addr(),
                                    remote_addr = ?conn.remote_addr(),
                                    is_outgoing = conn.is_outgoing(),
                                    link_id = ?conn.link_id(),
                                    "connection entry",
                                );
                                all_entries.push(ConnectionEntry {
                                    id,
                                    connection_type: ConnectionType::Remote as i32,
                                    config_data: match conn.config_data() {
                                        Some(data) => serde_json::to_string(data)
                                            .unwrap_or_else(|_| "{}".to_string()),
                                        None => "{}".to_string(),
                                    },
                                    link_id: conn.link_id(),
                                    direction: if conn.is_outgoing() {
                                        ConnectionDirection::Outgoing as i32
                                    } else {
                                        ConnectionDirection::Incoming as i32
                                    },
                                });
                            });

                        const CHUNK_SIZE: usize = 100;
                        let chunks: Vec<_> = all_entries.chunks(CHUNK_SIZE).collect();
                        if chunks.is_empty() {
                            let resp = ControlMessage {
                                message_id: uuid::Uuid::new_v4().to_string(),
                                payload: Some(Payload::ConnectionListResponse(
                                    ConnectionListResponse {
                                        original_message_id: msg.message_id.clone(),
                                        entries: vec![],
                                        done: true,
                                    },
                                )),
                            };
                            if let Err(e) = tx.send(Ok(resp)).await {
                                error!(error = %e.chain(), "failed to send connection list response");
                            }
                        } else {
                            let n = chunks.len();
                            for (i, chunk) in chunks.into_iter().enumerate() {
                                let resp = ControlMessage {
                                    message_id: uuid::Uuid::new_v4().to_string(),
                                    payload: Some(Payload::ConnectionListResponse(
                                        ConnectionListResponse {
                                            original_message_id: msg.message_id.clone(),
                                            entries: chunk.to_vec(),
                                            done: i + 1 == n,
                                        },
                                    )),
                                };
                                if let Err(e) = tx.send(Ok(resp)).await {
                                    error!(error = %e.chain(), "failed to send connection list batch");
                                    break;
                                }
                            }
                        }
                    }
                    Payload::DesiredState(desired) => {
                        let connections_status = self.diff_connections(desired).await;

                        let (desired_subs, mut subscriptions_status) =
                            self.resolve_desired_subscriptions(&desired.desired_subscriptions);
                        subscriptions_status
                            .extend(self.delete_stale_subscriptions(&desired_subs).await);
                        subscriptions_status
                            .extend(self.create_new_subscriptions(&desired_subs).await);

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
                            error!(error = %e.chain(), "failed to send ConfigurationCommandAck for DesiredState");
                        }

                        info!(
                            connections = %desired.desired_connections.len(),
                            subscriptions = %desired.desired_subscriptions.len(),
                            "Processed DesiredState"
                        );
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
            link_id: None,
            direction: None,
        };

        sub_vec.push(cmd);

        let ctrl = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec![],
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
            link_id: None,
            direction: None,
        };

        unsub_vec.push(cmd);

        let ctrl = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: unsub_vec,
            })),
        };

        return self.send_or_queue_notification(ctrl, clients).await;
    }

    /// Send a subscribe message and await the ack. Returns the subscription_id.
    async fn send_subscribe_message_with_ack(
        &self,
        mut msg: DataPlaneMessage,
    ) -> Result<u64, String> {
        let (ack_id, ack_rx) = self.inner.subscription_manager.register_ack();
        msg.set_subscription_id(ack_id);

        if let Err(e) = self.send_control_message(msg).await {
            self.inner.subscription_manager.cancel_ack(ack_id);
            return Err(format!("datapath send error: {}", e.chain()));
        }

        match tokio::time::timeout(SUBSCRIPTION_ACK_TIMEOUT, ack_rx).await {
            Ok(Ok(Ok(()))) => Ok(ack_id),
            Ok(Ok(Err(err))) => Err(err.to_string()),
            Ok(Err(_)) => Err("subscription ack channel closed".to_string()),
            Err(_) => {
                self.inner.subscription_manager.cancel_ack(ack_id);
                Err("subscription ack timed out".to_string())
            }
        }
    }

    /// Send an unsubscribe message with a given subscription_id and await the ack.
    async fn send_unsubscribe_message_with_ack(
        &self,
        mut msg: DataPlaneMessage,
        subscription_id: u64,
    ) -> Result<(), String> {
        let ack_rx = self
            .inner
            .subscription_manager
            .register_ack_with_id(subscription_id);
        msg.set_subscription_id(subscription_id);

        if let Err(e) = self.send_control_message(msg).await {
            self.inner.subscription_manager.cancel_ack(subscription_id);
            return Err(format!("datapath send error: {}", e.chain()));
        }

        match tokio::time::timeout(SUBSCRIPTION_ACK_TIMEOUT, ack_rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(err))) => Err(err.to_string()),
            Ok(Err(_)) => Err("subscription ack channel closed".to_string()),
            Err(_) => {
                self.inner.subscription_manager.cancel_ack(subscription_id);
                Err("subscription ack timed out".to_string())
            }
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
    ///
    /// Uses `try_send` to avoid blocking the caller when the bounded channel is
    /// full (e.g. due to HTTP/2 back-pressure).  Blocking here would stall the
    /// data-plane listener task, preventing it from resolving subscription acks
    /// that the control-message handler is waiting on — a deadlock.
    async fn send_or_queue_notification(&self, ctrl_msg: ControlMessage, clients: &[ClientConfig]) {
        let mut sent = false;

        for c in clients {
            let tx = match self.inner.tx_channels.read().get(&c.endpoint) {
                Some(tx) => tx.clone(),
                None => continue,
            };

            match tx.try_send(Ok(ctrl_msg.clone())) {
                Ok(()) => {
                    sent = true;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    debug!(
                        endpoint = %c.endpoint,
                        "channel full, queuing notification instead of blocking"
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!(
                        endpoint = %c.endpoint,
                        "channel closed, queuing notification"
                    );
                }
            }
        }

        if !sent {
            let mut queue = self.inner.pending_notifications.lock();
            if queue.len() >= MAX_QUEUED_NOTIFICATIONS {
                queue.pop_front();
                debug!("queue full, removed oldest notification");
            }
            queue.push_back(ctrl_msg);
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

            let mut active_connections = Vec::new();
            this.inner
                .message_processor
                .connection_table()
                .for_each(|id, conn| {
                    active_connections.push(v1::ConnectionEntry {
                        id,
                        connection_type: v1::ConnectionType::Remote as i32,
                        config_data: match conn.config_data() {
                            Some(data) => {
                                serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string())
                            }
                            None => "{}".to_string(),
                        },
                        link_id: conn.link_id(),
                        direction: if conn.is_outgoing() {
                            v1::ConnectionDirection::Outgoing as i32
                        } else {
                            v1::ConnectionDirection::Incoming as i32
                        },
                    });
                });

            let register_request = ControlMessage {
                message_id: uuid::Uuid::new_v4().to_string(),
                payload: Some(Payload::RegisterNodeRequest(v1::RegisterNodeRequest {
                    node_id: this.inner.id.to_string(),
                    group_name: this.inner.group_name.clone(),
                    connection_details: this.inner.connection_details.clone(),
                    connections: active_connections,
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
                                    } else {
                                        // Handle other IO errors (ConnectionAborted, etc.)
                                        error!(
                                            error = %e.chain(),
                                            io_error_kind = ?io_err.kind(),
                                            "IO error receiving control messages"
                                        );
                                    }
                                } else {
                                    // Handle non-IO errors (e.g., gRPC Canceled, Unavailable, etc.)
                                    error!(error = %e.chain(), "error receiving control messages");
                                }

                                retry_connect = true;
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

        // On reconnect, drain message_id_map entries from the previous session.
        // Their timers reference the old channel, so stop each one and send a
        // failure ack to unblock waiters.
        {
            let prev_entries: Vec<(u32, (String, Option<Timer>))> =
                self.inner.message_id_map.write().drain().collect();
            for (msg_id, (original_id, timer)) in prev_entries {
                if let Some(mut t) = timer {
                    t.stop();
                }
                let ack = Ack {
                    original_message_id: original_id,
                    success: false,
                    messages: vec![msg_id.to_string()],
                };
                let reply = ControlMessage {
                    message_id: uuid::Uuid::new_v4().to_string(),
                    payload: Some(Payload::Ack(ack)),
                };
                self.send_or_queue_notification(reply, std::slice::from_ref(&config))
                    .await;
            }
        }

        // Reset connection state before establishing a new session. The CP
        // will send fresh ConfigCommands after re-registration to rebuild
        // these maps.
        self.inner.route_subscription_ids.lock().clear();
        self.inner.link_id_to_conn_id.write().clear();

        let channel = config.to_channel().await?;

        let mut client = ControllerServiceClient::new(channel.clone());
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(128);
        let out_stream = ReceiverStream::new(rx).filter_map(|res| match res {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!(error = %e, "dropping outbound control message due to error");
                None
            }
        });
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
        let endpoint_key = config.endpoint.clone();
        let handle = self.process_control_message_stream(
            Some(config),
            stream.into_inner(),
            Some(timer_rx),
            tx.clone(),
            cancellation_token.clone(),
        )?;
        self.inner
            .stream_handles
            .lock()
            .insert(endpoint_key, handle);

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
        let handle = self
            .process_control_message_stream(
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
        self.inner
            .stream_handles
            .lock()
            .insert(remote_endpoint.clone(), handle);

        self.inner
            .tx_channels
            .write()
            .insert(remote_endpoint.clone(), tx);

        if let Some(old_token) = self
            .inner
            .cancellation_tokens
            .write()
            .insert(remote_endpoint.clone(), cancellation_token)
        {
            old_token.cancel();
        }

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
            connection_details: vec![from_server_config(&server_config)],
        });

        let control_plane_client = ControlPlane::new(ControlPlaneSettings {
            id: id_client,
            group_name: None,
            servers: vec![],
            clients: vec![client_config.clone()],
            message_processor: Arc::new(message_processor_client),
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
                    connections_to_delete: vec![],
                    subscriptions_to_set: vec![v1::Subscription {
                        component_0: "queued".to_string(),
                        component_1: "sub".to_string(),
                        component_2: format!("name-{i}"),
                        id: Some(i as u64),
                        connection_id: "test-conn".to_string(),
                        node_id: None,
                        link_id: None,
                        direction: None,
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
    async fn test_delete_connection_by_link_id_success_ack() {
        let (mut control_plane_server, mut control_plane_client, _client_cfg) =
            setup_control_planes(
                "127.0.0.1:50081",
                "delete-linkid-server",
                "delete-linkid-client",
            )
            .await;

        control_plane_server.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        control_plane_client.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let controller = control_plane_client.controller.clone();
        let link_id = "test-delete-link-id".to_string();
        let mut assigned = false;
        for _ in 0..50 {
            controller
                .inner
                .message_processor
                .connection_table()
                .for_each(|_, conn| {
                    if !assigned {
                        conn.set_link_id(link_id.clone());
                        assigned = true;
                    }
                });
            if assigned {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        assert!(
            assigned,
            "expected at least one connection to assign link_id"
        );

        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec![link_id.clone()],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![],
            })),
        };
        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };
        assert_eq!(ack.connections_status.len(), 1);
        assert_eq!(ack.connections_status[0].connection_id, link_id);
        assert!(ack.connections_status[0].success);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_delete_connection_by_link_id_unknown_fails_ack() {
        let (control_plane_server, control_plane_client, _client_cfg) = setup_control_planes(
            "127.0.0.1:50082",
            "delete-linkid-server-unknown",
            "delete-linkid-client-unknown",
        )
        .await;

        let controller = control_plane_client.controller.clone();
        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec!["unknown-link-id".to_string()],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![],
            })),
        };
        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };
        assert_eq!(ack.connections_status.len(), 1);
        assert_eq!(ack.connections_status[0].connection_id, "unknown-link-id");
        assert!(!ack.connections_status[0].success);
        assert!(ack.connections_status[0].error_msg.contains("not found"));

        drop(control_plane_server);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_create_connection_with_existing_link_id_reuses_connection_ack() {
        let (mut control_plane_server, mut control_plane_client, _client_cfg) =
            setup_control_planes(
                "127.0.0.1:50083",
                "create-linkid-server",
                "create-linkid-client",
            )
            .await;

        control_plane_server.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        control_plane_client.run().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let controller = control_plane_client.controller.clone();
        let link_id = "test-create-link-id".to_string();
        let mut assigned = false;
        for _ in 0..50 {
            controller
                .inner
                .message_processor
                .connection_table()
                .for_each(|_, conn| {
                    if !assigned {
                        conn.set_link_id(link_id.clone());
                        assigned = true;
                    }
                });
            if assigned {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        assert!(
            assigned,
            "expected at least one connection to assign link_id"
        );

        let endpoint = "http://127.0.0.1:59999";
        let connection_config = serde_json::json!({
            "endpoint": endpoint,
            "link_id": link_id
        })
        .to_string();

        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![v1::Connection {
                    connection_id: "reuse-existing-link".to_string(),
                    config_data: connection_config,
                }],
                connections_to_delete: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![],
            })),
        };

        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };
        assert_eq!(ack.connections_status.len(), 1);
        assert_eq!(
            ack.connections_status[0].connection_id,
            "reuse-existing-link"
        );
        assert!(ack.connections_status[0].success);

        assert!(
            controller
                .inner
                .link_id_to_conn_id
                .read()
                .contains_key(&link_id),
            "expected link_id to be mapped to reused connection id"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_subscription_set_unknown_link_id_fails_ack() {
        let (control_plane_server, control_plane_client, _client_cfg) = setup_control_planes(
            "127.0.0.1:50084",
            "sub-linkid-server-unknown",
            "sub-linkid-client-unknown",
        )
        .await;

        let controller = control_plane_client.controller.clone();
        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec![],
                subscriptions_to_set: vec![v1::Subscription {
                    component_0: "org".to_string(),
                    component_1: "ns".to_string(),
                    component_2: "agent".to_string(),
                    id: Some(1),
                    connection_id: String::new(),
                    node_id: None,
                    link_id: Some("missing-link-id".to_string()),
                    direction: None,
                }],
                subscriptions_to_delete: vec![],
            })),
        };
        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };

        assert_eq!(ack.subscriptions_status.len(), 1);
        assert!(!ack.subscriptions_status[0].success);
        assert!(
            ack.subscriptions_status[0]
                .error_msg
                .contains("Connection with link_id missing-link-id not found")
        );

        drop(control_plane_server);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_create_connection_invalid_config_fails_ack() {
        let (_control_plane_server, control_plane_client, _client_cfg) = setup_control_planes(
            "127.0.0.1:50085",
            "create-invalid-config-server",
            "create-invalid-config-client",
        )
        .await;

        let controller = control_plane_client.controller.clone();
        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![v1::Connection {
                    connection_id: "invalid-config-conn".to_string(),
                    config_data: "{invalid-json".to_string(),
                }],
                connections_to_delete: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![],
            })),
        };
        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };
        assert_eq!(ack.connections_status.len(), 1);
        assert!(!ack.connections_status[0].success);
        assert!(
            ack.connections_status[0]
                .error_msg
                .contains("Failed to parse config")
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_subscription_delete_unknown_link_id_fails_ack() {
        let (control_plane_server, control_plane_client, _client_cfg) = setup_control_planes(
            "127.0.0.1:50086",
            "sub-del-linkid-server-unknown",
            "sub-del-linkid-client-unknown",
        )
        .await;

        let controller = control_plane_client.controller.clone();
        let ctrl_msg = ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommand(v1::ConfigurationCommand {
                connections_to_create: vec![],
                connections_to_delete: vec![],
                subscriptions_to_set: vec![],
                subscriptions_to_delete: vec![v1::Subscription {
                    component_0: "org".to_string(),
                    component_1: "ns".to_string(),
                    component_2: "agent".to_string(),
                    id: Some(1),
                    connection_id: String::new(),
                    node_id: None,
                    link_id: Some("missing-link-id-delete".to_string()),
                    direction: None,
                }],
            })),
        };
        let (tx, mut rx) = mpsc::channel(1);
        controller
            .handle_new_control_message(ctrl_msg, &tx)
            .await
            .expect("config command must be handled");

        let ack_msg = rx
            .recv()
            .await
            .expect("expected ack message")
            .expect("ack should be ok");
        let ack = match ack_msg.payload {
            Some(Payload::ConfigCommandAck(ack)) => ack,
            _ => panic!("expected ConfigCommandAck payload"),
        };

        assert_eq!(ack.subscriptions_status.len(), 1);
        assert!(!ack.subscriptions_status[0].success);
        assert!(
            ack.subscriptions_status[0]
                .error_msg
                .contains("Connection with link_id missing-link-id-delete not found")
        );

        drop(control_plane_server);
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

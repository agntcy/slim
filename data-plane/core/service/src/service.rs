// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;

use display_error_chain::ErrorChainExt;
// Third-party crates
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::component::configuration::Configuration;
use slim_config::component::id::{ID, Kind};
use slim_config::component::{Component, ComponentBuilder};
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_controller::config::Config as ControllerConfig;
use slim_controller::config::Config as DataplaneConfig;
use slim_controller::service::ControlPlane;
// removed unused DataPlaneServiceServer import
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;

// Local crate
#[cfg(feature = "session")]
use crate::app::App;
use crate::errors::ServiceError;
#[cfg(feature = "session")]
use slim_session::notification::Notification;
#[cfg(feature = "session")]
use slim_session::{Direction, SessionError};

// Define the kind of the component as static string
pub const KIND: &str = "slim";

/// Information about a connection
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Connection ID
    pub id: u64,

    /// Remote address and port (if available)
    pub remote_addr: Option<SocketAddr>,

    /// Local address and port (if available)
    pub local_addr: Option<SocketAddr>,

    /// Endpoint from client configuration (if available)
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceConfiguration {
    /// Optional node ID for the service. If not set, the name of the component will be used.
    #[serde(default)]
    pub node_id: Option<String>,

    /// Optional name of the group for the service.
    #[serde(default)]
    pub group_name: Option<String>,

    /// DataPlane API configuration
    #[serde(default)]
    pub dataplane: DataplaneConfig,

    /// Controller API configuration
    #[serde(default)]
    pub controller: ControllerConfig,
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        ServiceConfiguration::default()
    }

    pub fn with_dataplane_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.dataplane.servers = server;
        self
    }

    pub fn with_dataplane_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.dataplane.clients = clients;
        self
    }

    pub fn dataplane_servers(&self) -> &[ServerConfig] {
        self.dataplane.servers.as_ref()
    }

    pub fn dataplane_clients(&self) -> &[ClientConfig] {
        &self.dataplane.clients
    }

    pub fn with_controlplane_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.controller.servers = server;
        self
    }

    pub fn with_controlplane_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.controller.clients = clients;
        self
    }

    pub fn controlplane_servers(&self) -> &[ServerConfig] {
        self.controller.servers.as_ref()
    }

    pub fn controlplane_clients(&self) -> &[ClientConfig] {
        &self.controller.clients
    }

    pub fn build_server(&self, id: ID) -> Result<Service, ServiceError> {
        let service = Service::new_with_config(id, self.clone());
        Ok(service)
    }
}

impl Configuration for ServiceConfiguration {
    type Error = ServiceError;

    fn validate(&self) -> Result<(), Self::Error> {
        // Validate client and server configurations
        for server in self.dataplane.servers.iter() {
            server.validate()?;
        }
        for client in &self.dataplane.clients {
            client.validate()?;
        }

        // Validate the controller
        self.controller.validate()?;

        Ok(())
    }
}

pub struct Service {
    /// id of the service
    id: ID,

    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// controller service
    controller: tokio::sync::RwLock<Option<ControlPlane>>,

    /// the configuration of the service
    config: ServiceConfiguration,

    /// cancellation tokens to stop the servers main loop
    cancellation_tokens: parking_lot::RwLock<HashMap<String, CancellationToken>>,

    /// clients created by the service
    clients: parking_lot::RwLock<HashMap<String, u64>>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("id", &self.id)
            .field("dataplane_servers", &self.config.dataplane_servers())
            .field("dataplane_clients", &self.config.dataplane_clients())
            .field("group_name", &self.config.group_name)
            .field("controller", &self.config.controller)
            .finish()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // Trigger all cancellation tokens to stop servers
        for (endpoint, token) in self.cancellation_tokens.write().drain() {
            debug!(%endpoint, "cancelling server on drop");
            token.cancel();
        }

        // Disconnect all clients
        for (endpoint, conn_id) in self.clients.write().drain() {
            debug!(%endpoint, conn_id = %conn_id, "disconnecting client on drop");
            if let Err(e) = self.message_processor.disconnect(conn_id) {
                tracing::error!(%endpoint, error = %e.chain(), "disconnect error");
            }
        }
    }
}

impl Service {
    /// Create a new Service
    pub fn new(id: ID) -> Self {
        Service::new_with_config(id, ServiceConfiguration::new())
    }

    /// Create a new Service with configuration
    pub fn new_with_config(id: ID, config: ServiceConfiguration) -> Self {
        let message_processor = Arc::new(MessageProcessor::new());

        Service {
            id,
            message_processor,
            controller: tokio::sync::RwLock::new(None),
            config,
            cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
            clients: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// get the service configuration
    pub fn config(&self) -> &ServiceConfiguration {
        &self.config
    }

    /// Create a new ServiceBuilder
    pub fn builder() -> ServiceBuilder {
        ServiceBuilder::new()
    }

    /// Run the service
    pub async fn run(&self) -> Result<(), ServiceError> {
        // Check that at least one client or server is configured

        if self.config.dataplane_servers().is_empty() && self.config.dataplane_clients().is_empty()
        {
            return Err(ServiceError::NoServerOrClientConfigured);
        }

        // Run servers
        for server in self.config.dataplane_servers().iter() {
            self.run_server(server).await?;
        }

        // Run clients
        for client in self.config.dataplane_clients() {
            _ = self.connect(client).await?;
        }

        // Controller service
        if self.config.controller.is_default() {
            info!("no controller configuration provided, skipping controller startup");
            return Ok(());
        }

        // run the controller
        debug!("starting controller service");
        let mut controller = self.config.controller.into_service(
            self.id.clone(),
            self.config.group_name.clone(),
            self.message_processor.clone(),
            self.config.dataplane_servers(),
        );

        // run controller service
        controller.run().await?;

        // save controller service
        *self.controller.write().await = Some(controller);

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), ServiceError> {
        debug!("shutting down service");

        // Cancel and drain all server cancellation tokens
        for (endpoint, token) in self.cancellation_tokens.write().drain() {
            info!(%endpoint, "stopping server");
            token.cancel();
        }

        // Disconnect all clients (ignore individual disconnect errors, just log)
        for (endpoint, conn_id) in self.clients.write().drain() {
            info!(%endpoint, conn_id = %conn_id, "disconnecting client");
            if let Err(e) = self.message_processor.disconnect(conn_id) {
                tracing::error!(%endpoint, error = %e.chain(), "disconnect error");
            }
        }

        // Call the shutdown method of message processor to make sure all
        // tasks ended gracefully
        self.message_processor.shutdown().await?;

        // Shutdown controller if present
        if let Some(ref controller) = *self.controller.read().await {
            controller.shutdown().await?;
        }

        Ok(())
    }

    // APP APIs
    #[cfg(feature = "session")]
    pub fn create_app<P, V>(
        &self,
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<
        (
            App<P, V>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    >
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        self.create_app_with_direction(
            app_name,
            identity_provider,
            identity_verifier,
            Direction::Bidirectional,
        )
    }

    #[cfg(feature = "session")]
    pub fn create_app_with_direction<P, V>(
        &self,
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        direction: Direction,
    ) -> Result<
        (
            App<P, V>,
            mpsc::Receiver<Result<Notification, SessionError>>,
        ),
        ServiceError,
    >
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        debug!(%app_name, "creating app");

        // Create storage path for the app
        let mut hasher = DefaultHasher::new();
        app_name.to_string().hash(&mut hasher);
        let hashed_name = hasher.finish();

        let home_dir = dirs::home_dir().ok_or(ServiceError::HomeDirUnavailable)?;
        let storage_path = home_dir.join(".slim").join(hashed_name.to_string());
        std::fs::create_dir_all(&storage_path)?;

        // Channels to communicate with SLIM
        let (conn_id, tx_slim, rx_slim) =
            self.message_processor.register_local_connection(false)?;

        // Channels to communicate with the local app. This will be mainly used to receive notifications about new
        // sessions opened

        // TODO(msardara): make the buffer size configurable
        let (tx_app, rx_app) = mpsc::channel(128);

        // create app
        let app = App::new_with_direction(
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            storage_path,
            direction,
        );

        // start message processing using the rx channel
        app.process_messages(rx_slim);

        // return the app instance and the rx channel
        Ok((app, rx_app))
    }

    pub async fn run_server(&self, config: &ServerConfig) -> Result<(), ServiceError> {
        let cancellation_token = self.message_processor.run_server(config).await?;
        self.cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), cancellation_token);

        info!(endpoint = %config.endpoint, "dataplane server started");

        Ok(())
    }

    pub fn stop_server(&self, endpoint: &str) -> Result<(), ServiceError> {
        // stop the server
        if let Some(token) = self.cancellation_tokens.write().remove(endpoint) {
            token.cancel();
            Ok(())
        } else {
            Err(ServiceError::ServerNotFound(endpoint.to_string()))
        }
    }

    pub async fn connect(&self, config: &ClientConfig) -> Result<u64, ServiceError> {
        // ensure there is no other client connected to the same endpoint
        if self.clients.read().contains_key(&config.endpoint) {
            return Err(ServiceError::ClientAlreadyConnected(
                config.endpoint.clone(),
            ));
        }

        let (_handle, conn_id) = self
            .message_processor
            .connect(config.clone(), None, None)
            .await?;

        // register the client
        self.clients
            .write()
            .insert(config.endpoint.clone(), conn_id);

        tracing::info!(endpoint = %config.endpoint, conn_id = %conn_id, "client connected");

        // return the connection id
        Ok(conn_id)
    }

    pub fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        let client_config = self.message_processor.disconnect(conn)?;
        let endpoint = client_config.endpoint.clone();
        let mut clients = self.clients.write();

        let stored_conn =
            clients
                .get(&endpoint)
                .ok_or(ServiceError::ConnectionNotFoundForEndpoint(
                    endpoint.clone(),
                ))?;

        if *stored_conn == conn {
            clients.remove(&endpoint);
            debug!(%endpoint, "removed client mapping");
        } else {
            return Err(ServiceError::DifferentIdForConnection {
                endpoint: endpoint.clone(),
                expected: *stored_conn,
                found: conn,
            });
        }

        Ok(())
    }

    pub fn get_connection_id(&self, endpoint: &str) -> Option<u64> {
        self.clients.read().get(endpoint).cloned()
    }

    /// Get a list of all connections ordered by connection ID
    ///
    /// This method iterates through the client connections tracked by the service
    /// and returns information about all active connections, sorted by their connection ID.
    ///
    /// # Returns
    /// A vector of `ConnectionInfo` structs, ordered by connection ID (ascending)
    pub fn get_all_connections(&self) -> Vec<ConnectionInfo> {
        let clients = self.clients.read();
        let mut connections: Vec<ConnectionInfo> = clients
            .iter()
            .filter_map(|(endpoint, &conn_id)| {
                // Get connection details from the connection table
                self.message_processor
                    .connection_table()
                    .get(conn_id as usize)
                    .map(|conn| ConnectionInfo {
                        id: conn_id,
                        remote_addr: conn.remote_addr().copied(),
                        local_addr: conn.local_addr().copied(),
                        endpoint: Some(endpoint.clone()),
                    })
            })
            .collect();

        // Sort by connection ID
        connections.sort_by_key(|c| c.id);

        connections
    }

    #[cfg(test)]
    pub(crate) fn message_processor(&self) -> &Arc<MessageProcessor> {
        &self.message_processor
    }
}

#[async_trait::async_trait]
impl Component for Service {
    type Error = ServiceError;

    fn identifier(&self) -> &ID {
        &self.id
    }

    async fn start(&mut self) -> Result<(), Self::Error> {
        debug!("starting service");
        let res = self.run().await?;

        Ok(res)
    }
}

#[derive(PartialEq, Eq, Hash, Default)]
pub struct ServiceBuilder;

impl ServiceBuilder {
    // Create a new ServiceBuilder
    pub fn new() -> Self {
        ServiceBuilder {}
    }

    pub fn kind() -> Kind {
        Kind::new(KIND).unwrap()
    }
}

impl ComponentBuilder for ServiceBuilder {
    type Config = ServiceConfiguration;
    type Component = Service;

    // Kind of the component
    fn kind(&self) -> Kind {
        ServiceBuilder::kind()
    }

    // Build the component
    fn build(&self, name: String) -> Result<Self::Component, ServiceError> {
        let id = ID::new_with_name(ServiceBuilder::kind(), name.as_ref())?;

        Ok(Service::new(id))
    }

    // Build the component
    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, ServiceError> {
        let node_name = config.node_id.clone().unwrap_or(name.to_string());
        let id = ID::new_with_name(ServiceBuilder::kind(), &node_name)?;

        config.build_server(id)
    }
}

// tests
#[cfg(test)]
mod tests {

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_config::grpc::server::ServerConfig;
    use slim_config::tls::server::TlsServerConfig;
    use slim_datapath::api::MessageType;
    use slim_datapath::messages::Name;
    use slim_session::SessionConfig;
    use slim_testing::utils::TEST_VALID_SECRET;
    use std::time::Duration;
    use tokio::time;
    use tracing_test::traced_test;

    #[tokio::test]
    async fn test_service_configuration() {
        let config = ServiceConfiguration::new();
        assert_eq!(config.dataplane_servers(), &[]);
        assert_eq!(config.dataplane_clients(), &[]);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_skips_controller_when_config_is_default() {
        // Create a service with only dataplane server config, no controller config
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12347").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(
                ID::new_with_name(Kind::new(KIND).unwrap(), "test-no-controller").unwrap(),
            )
            .unwrap();

        // Run the service - should start dataplane but skip controller
        service.run().await.expect("failed to run service");

        // Wait a bit for logs to be generated
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify controller was skipped
        assert!(logs_contain(
            "no controller configuration provided, skipping controller startup"
        ));
        // Verify dataplane still started
        assert!(logs_contain("dataplane server started"));

        // Graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_starts_controller_when_config_is_provided() {
        // Create a service with both dataplane and controller configurations
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let dataplane_server_config =
            ServerConfig::with_endpoint("0.0.0.0:12348").with_tls_settings(tls_config.clone());
        let controller_server_config =
            ServerConfig::with_endpoint("0.0.0.0:12349").with_tls_settings(tls_config);

        let config = ServiceConfiguration::new()
            .with_dataplane_server(vec![dataplane_server_config])
            .with_controlplane_server(vec![controller_server_config]);

        let service = config
            .build_server(
                ID::new_with_name(Kind::new(KIND).unwrap(), "test-with-controller").unwrap(),
            )
            .unwrap();

        // Run the service - should start both dataplane and controller
        service.run().await.expect("failed to run service");

        // Wait a bit for logs to be generated
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify controller was started (not skipped)
        assert!(!logs_contain(
            "no controller configuration provided, skipping controller startup"
        ));
        assert!(logs_contain("starting controller service"));
        // Verify dataplane also started
        assert!(logs_contain("dataplane server started"));

        // Graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_build_server() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        service.run().await.expect("failed to run service");

        // wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // assert that the service is running
        assert!(logs_contain("dataplane server started"));

        // graceful shutdown
        service
            .shutdown()
            .await
            .expect("failed to shutdown service");

        assert!(logs_contain("shutting down service"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_disconnection() {
        // create the service (server + one client we will disconnect)
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12346").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test-disconnect").unwrap())
            .unwrap();

        service.run().await.expect("failed to run service");

        // wait a bit for server loop to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // build client configuration and connect
        let mut client_conf =
            slim_config::grpc::client::ClientConfig::with_endpoint("http://0.0.0.0:12346");
        client_conf.tls_setting.insecure = true;
        let conn_id = service
            .connect(&client_conf)
            .await
            .expect("failed to connect client");

        assert!(service.get_connection_id(&client_conf.endpoint).is_some());

        // disconnect
        service
            .disconnect(conn_id)
            .expect("disconnect should succeed");

        // allow cancellation token to propagate and stream to terminate
        tokio::time::sleep(Duration::from_millis(200)).await;

        // verify connection is removed from internal client mapping
        assert!(
            service.get_connection_id(&client_conf.endpoint).is_none(),
            "client mapping should be removed after disconnect"
        );

        // verify connection is removed from connection table
        assert!(
            service
                .message_processor
                .connection_table()
                .get(conn_id as usize)
                .is_none(),
            "connection should be removed after disconnect"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_publish_subscribe() {
        // in this test, we create a publisher and a subscriber and test the
        // communication between them

        info!("starting test_service_publish_subscribe");

        // create the service
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // create a subscriber
        let subscriber_name = Name::from_strings(["cisco", "default", "subscriber"]).with_id(0);
        let (sub_app, mut sub_rx) = service
            .create_app(
                &subscriber_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .expect("failed to create app");

        // create a publisher
        let publisher_name = Name::from_strings(["cisco", "default", "publisher"]).with_id(0);
        let (pub_app, _rx) = service
            .create_app(
                &publisher_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .expect("failed to create app");

        // sleep to allow the subscription to be processed
        time::sleep(Duration::from_millis(100)).await;

        // NOTE: here we don't call any subscribe as the publisher and the subscriber
        // are in the same service (so they share one single slim instance) and the
        // subscription is done automatically.

        // create a point to point session
        let mut config = SessionConfig::default()
            .with_session_type(slim_datapath::api::ProtoSessionType::PointToPoint);
        config.initiator = true;
        let (send_session, completion_handle) = pub_app
            .create_session(config, subscriber_name.clone(), None)
            .await
            .unwrap();

        completion_handle.await.expect("session creation failed");

        // publish a message
        let message_blob = "very complicated message".as_bytes().to_vec();
        send_session
            .session_arc()
            .unwrap()
            .publish(&subscriber_name, message_blob.clone(), None, None)
            .await
            .unwrap();

        // wait for the new session to arrive in the subscriber app
        // and check the message is correct
        let session = sub_rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let mut recv_session = match session {
            Notification::NewSession(s) => s,
            _ => panic!("expected a point to point session"),
        };

        // Let's receive now the message from the session
        let msg = recv_session
            .rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // make sure message is a publication
        assert!(msg.message_type.is_some());

        // make sure the session ids correspond
        assert_eq!(
            send_session.session_arc().unwrap().id(),
            msg.get_session_header().get_session_id()
        );

        let publ = match msg.message_type.unwrap() {
            MessageType::Publish(p) => p,
            _ => panic!("expected a publication"),
        };

        // make sure message is correct
        assert_eq!(
            publ.get_payload().as_application_payload().unwrap().blob,
            message_blob
        );

        // Now remove the session from the 2 apps
        pub_app
            .delete_session(&send_session.session_arc().unwrap())
            .unwrap();
        sub_app
            .delete_session(&recv_session.session_arc().unwrap())
            .unwrap();

        // And drop the 2 apps
        drop(pub_app);
        drop(sub_app);

        // sleep to allow the deletion to be processed
        time::sleep(Duration::from_millis(100)).await;

        // This should also trigger a stop of the message processing loop.
        // Make sure the loop stopped by checking the logs
        assert!(logs_contain("message processing loop cancelled"));
    }

    #[tokio::test]
    async fn test_session_configuration() {
        // create the service
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_dataplane_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // register local app
        let name = Name::from_strings(["cisco", "default", "session"]).with_id(0);
        let (app, _) = service
            .create_app(
                &name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .expect("failed to create app");

        //////////////////////////// p2p session ////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(Duration::from_millis(500)),
            mls_enabled: false,
            initiator: true,
            metadata: HashMap::new(),
        };
        let dst = Name::from_strings(["org", "ns", "dst"]);
        let (session_info, _completion_handle) = app
            .create_session(session_config.clone(), dst, None)
            .await
            .expect("Failed to create session");

        // check the configuration we get is the one we used to create the session
        let session_config_ret = session_info.session().upgrade().unwrap().session_config();

        assert_eq!(session_config_ret, session_config);

        ////////////// multicast session //////////////////////////////////////////////////////////////////////////////////

        let stream = Name::from_strings(["agntcy", "ns", "stream"]);

        let session_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::Multicast,
            max_retries: Some(5),
            interval: Some(Duration::from_millis(1000)),
            mls_enabled: true,
            initiator: true,
            metadata: HashMap::new(),
        };
        let (session_info, _completion_handle) = app
            .create_session(session_config.clone(), stream.clone(), None)
            .await
            .expect("Failed to create session");

        // The multicast session was created successfully

        let session_config_ret = session_info.session().upgrade().unwrap().session_config();

        assert_eq!(session_config_ret, session_config);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_error_routing_with_session_context() {
        // This test verifies that errors from the datapath that include session context
        // are properly routed to the correct session by sending a message to a non-existent
        // destination and verifying the error contains session context

        use slim_datapath::api::ProtoSessionType;

        info!("starting test_error_routing_with_session_context");

        // Create the service
        let service = Service::new(
            ID::new_with_name(Kind::new(KIND).unwrap(), "test-error-routing").unwrap(),
        );

        // Create an app
        let app_name = Name::from_strings(["cisco", "default", "testapp"]).with_id(0);
        let (app, _app_rx) = service
            .create_app(
                &app_name,
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            )
            .expect("failed to create app");

        // Create a point to point session to a non-existent destination
        // This will trigger an error from the datapath
        let non_existent_dst = Name::from_strings(["cisco", "default", "nonexistent"]).with_id(999);
        let mut session_config =
            SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
        session_config.initiator = true;
        session_config.max_retries = Some(10);
        session_config.interval = Some(Duration::from_secs(2));

        let (session, completion_handle) = app
            .create_session(session_config, non_existent_dst.clone(), None)
            .await
            .unwrap();

        let session_id = session.session_arc().unwrap().id();
        info!("Created session with ID: {}", session_id);

        // Wait session creation in completion handle. It should fail quickly as the
        // destination does not exist
        let result = tokio::time::timeout(std::time::Duration::from_millis(300), completion_handle)
            .await
            .expect("timeout waiting for session creation");

        assert!(
            result.is_err_and(|e| {
                println!("--> {}", e.chain());
                true
            }),
            "Session creation should fail for non-existent destination"
        );
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

// Third-party crates
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::component::configuration::{Configuration, ConfigurationError};
use slim_config::component::id::{ID, Kind};
use slim_config::component::{Component, ComponentBuilder, ComponentError};
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_controller::config::Config as ControllerConfig;
use slim_controller::config::Config as DataplaneConfig;
use slim_controller::service::ControlPlane;
use slim_datapath::api::DataPlaneServiceServer;
use slim_datapath::message_processing::MessageProcessor;
use slim_datapath::messages::Name;

// Local crate
use crate::app::App;
use crate::errors::ServiceError;
use slim_session::SessionError;
use slim_session::notification::Notification;

// Define the kind of the component as static string
pub const KIND: &str = "slim";

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

    pub fn with_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.dataplane.servers = server;
        self
    }

    pub fn with_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.dataplane.clients = clients;
        self
    }

    pub fn servers(&self) -> &[ServerConfig] {
        self.dataplane.servers.as_ref()
    }

    pub fn clients(&self) -> &[ClientConfig] {
        &self.dataplane.clients
    }

    pub fn build_server(&self, id: ID) -> Result<Service, ServiceError> {
        let service = Service::new(id).with_config(self.clone());
        Ok(service)
    }
}

impl Configuration for ServiceConfiguration {
    fn validate(&self) -> Result<(), ConfigurationError> {
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
    controller: Option<ControlPlane>,

    /// the configuration of the service
    config: ServiceConfiguration,

    /// drain watch to shutdown the service
    watch: drain::Watch,

    /// signal to shutdown the service
    signal: drain::Signal,

    /// cancellation tokens to stop the servers main loop
    cancellation_tokens: parking_lot::RwLock<HashMap<String, CancellationToken>>,

    /// clients created by the service
    clients: parking_lot::RwLock<HashMap<String, u64>>,
}

impl Service {
    /// Create a new Service
    pub fn new(id: ID) -> Self {
        let (signal, watch) = drain::channel();

        let message_processor = Arc::new(MessageProcessor::with_drain_channel(watch.clone()));

        Service {
            id,
            message_processor,
            controller: None,
            config: ServiceConfiguration::new(),
            watch,
            signal,
            cancellation_tokens: parking_lot::RwLock::new(HashMap::new()),
            clients: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Set the configuration of the service
    pub fn with_config(self, config: ServiceConfiguration) -> Self {
        Service { config, ..self }
    }

    /// Set the message processor of the service
    pub fn with_message_processor(self, message_processor: Arc<MessageProcessor>) -> Self {
        Service {
            message_processor,
            ..self
        }
    }

    /// get the service configuration
    pub fn config(&self) -> &ServiceConfiguration {
        &self.config
    }

    /// get signal used to shutdown the service
    /// NOTE: this method consumes the service!
    pub fn signal(self) -> drain::Signal {
        self.signal
    }

    /// Create a new ServiceBuilder
    pub fn builder() -> ServiceBuilder {
        ServiceBuilder::new()
    }

    /// Run the service
    pub async fn run(&mut self) -> Result<(), ServiceError> {
        // Check that at least one client or server is configured

        if self.config.servers().is_empty() && self.config.clients().is_empty() {
            return Err(ServiceError::ConfigError(
                "no dataplane server or clients configured".to_string(),
            ));
        }

        // Run servers
        for server in self.config.servers().iter() {
            info!("starting server {}", server.endpoint);
            self.run_server(server).await?;
        }

        // Run clients
        for client in self.config.clients().iter() {
            _ = self.connect(client).await?;
            info!("client connected to {}", client.endpoint);
        }

        // Controller service
        let mut controller = self.config.controller.into_service(
            self.id.clone(),
            self.config.group_name.clone(),
            self.watch.clone(),
            self.message_processor.clone(),
            self.config.servers(),
        );

        // run controller service
        controller.run().await.map_err(|e| {
            ServiceError::ControllerError(format!("failed to run controller service: {}", e))
        })?;

        // save controller service
        self.controller = Some(controller);

        Ok(())
    }

    // APP APIs
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
        debug!(%app_name, "creating app");

        // Create storage path for the app
        let mut hasher = DefaultHasher::new();
        app_name.to_string().hash(&mut hasher);
        let hashed_name = hasher.finish();

        let home_dir = dirs::home_dir().ok_or_else(|| {
            ServiceError::StorageError("Unable to determine home directory".to_string())
        })?;
        let storage_path = home_dir.join(".slim").join(hashed_name.to_string());
        std::fs::create_dir_all(&storage_path).map_err(|e| {
            ServiceError::StorageError(format!("Failed to create storage directory: {}", e))
        })?;

        // Channels to communicate with SLIM
        let (conn_id, tx_slim, rx_slim) = self.message_processor.register_local_connection(false);

        // Channels to communicate with the local app. This will be mainly used to receive notifications about new
        // sessions opened

        // TODO(msardara): make the buffer size configurable
        let (tx_app, rx_app) = mpsc::channel(128);

        // create app
        let app = App::new(
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            storage_path,
        );

        // start message processing using the rx channel
        app.process_messages(rx_slim);

        // return the app instance and the rx channel
        Ok((app, rx_app))
    }

    pub async fn run_server(&self, config: &ServerConfig) -> Result<(), ServiceError> {
        info!(%config, "server configured: setting it up");

        let cancellation_token = config
            .run_server(
                &[DataPlaneServiceServer::from_arc(
                    self.message_processor.clone(),
                )],
                self.watch.clone(),
            )
            .await
            .map_err(|e| ServiceError::ConfigError(format!("failed to run server: {}", e)))?;

        self.cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), cancellation_token);

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
        // make sure there is no other client connected to the same endpoint
        // TODO(msardara): we might want to allow multiple clients to connect to the same endpoint,
        // but we need to introduce an identifier in the configuration for it
        if self.clients.read().contains_key(&config.endpoint) {
            return Err(ServiceError::ClientAlreadyConnected(
                config.endpoint.clone(),
            ));
        }

        match config.to_channel().await {
            Err(e) => {
                error!("error reading channel config {:?}", e);
                Err(ServiceError::ConfigError(e.to_string()))
            }
            Ok(channel) => {
                //let client_config = config.clone();
                let ret = self
                    .message_processor
                    .connect(channel, Some(config.clone()), None, None)
                    .await
                    .map_err(|e| ServiceError::ConnectionError(e.to_string()));

                let conn_id = match ret {
                    Err(e) => {
                        error!("connection error: {:?}", e);
                        return Err(ServiceError::ConnectionError(e.to_string()));
                    }
                    Ok(conn_id) => conn_id.1,
                };

                // register the client
                self.clients
                    .write()
                    .insert(config.endpoint.clone(), conn_id);

                // return the connection id
                Ok(conn_id)
            }
        }
    }

    pub fn disconnect(&self, conn: u64) -> Result<(), ServiceError> {
        info!("disconnect from conn {}", conn);

        match self.message_processor.disconnect(conn) {
            Ok(cfg) => {
                let endpoint = cfg.endpoint.clone();
                let mut clients = self.clients.write();
                if let Some(stored_conn) = clients.get(&endpoint) {
                    if *stored_conn == conn {
                        clients.remove(&endpoint);
                        info!("removed client mapping for endpoint {}", endpoint);
                    } else {
                        debug!(
                            "client mapping endpoint {} has different conn_id {} != {}",
                            endpoint, stored_conn, conn
                        );
                    }
                } else {
                    debug!("no client mapping found for endpoint {}", endpoint);
                }
                Ok(())
            }
            Err(e) => Err(ServiceError::DisconnectError(e.to_string())),
        }
    }

    pub fn get_connection_id(&self, endpoint: &str) -> Option<u64> {
        self.clients.read().get(endpoint).cloned()
    }
}

impl Component for Service {
    fn identifier(&self) -> &ID {
        &self.id
    }

    async fn start(&mut self) -> Result<(), ComponentError> {
        info!("starting service");
        self.run()
            .await
            .map_err(|e| ComponentError::RuntimeError(e.to_string()))
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
    fn build(&self, name: String) -> Result<Self::Component, ComponentError> {
        let id = ID::new_with_name(ServiceBuilder::kind(), name.as_ref())
            .map_err(|e| ComponentError::ConfigError(e.to_string()))?;

        Ok(Service::new(id))
    }

    // Build the component
    fn build_with_config(
        &self,
        name: &str,
        config: &Self::Config,
    ) -> Result<Self::Component, ComponentError> {
        let node_name = config.node_id.clone().unwrap_or(name.to_string());
        let id = ID::new_with_name(ServiceBuilder::kind(), &node_name)
            .map_err(|e| ComponentError::ConfigError(e.to_string()))?;

        let service = config
            .build_server(id)
            .map_err(|e| ComponentError::ConfigError(e.to_string()))?;

        Ok(service)
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
        assert_eq!(config.servers(), &[]);
        assert_eq!(config.clients(), &[]);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_build_server() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_server([server_config].to_vec());
        let mut service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        service.run().await.expect("failed to run service");

        // wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // assert that the service is running
        assert!(logs_contain("starting server main loop"));

        // send the drain signal and wait for graceful shutdown
        match time::timeout(time::Duration::from_secs(10), service.signal().drain()).await {
            Ok(_) => {}
            Err(_) => panic!("timeout waiting for drain"),
        }

        // wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(logs_contain("shutting down server"));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_disconnection() {
        // create the service (server + one client we will disconnect)
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12346").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_server([server_config].to_vec());
        let mut service = config
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

        // verify disconnect log
        assert!(logs_contain("disconnect from conn"));
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
        let config = ServiceConfiguration::new().with_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // create a subscriber
        let subscriber_name = Name::from_strings(["cisco", "default", "subscriber"]).with_id(0);
        let (sub_app, mut sub_rx) = service
            .create_app(
                &subscriber_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .expect("failed to create app");

        // create a publisher
        let publisher_name = Name::from_strings(["cisco", "default", "publisher"]).with_id(0);
        let (pub_app, _rx) = service
            .create_app(
                &publisher_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
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
        let config = ServiceConfiguration::new().with_server([server_config].to_vec());
        let service = config
            .build_server(ID::new_with_name(Kind::new(KIND).unwrap(), "test").unwrap())
            .unwrap();

        // register local app
        let name = Name::from_strings(["cisco", "default", "session"]).with_id(0);
        let (app, _) = service
            .create_app(
                &name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
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
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod authzen_integration;
pub mod errors;
pub mod producer_buffer;
pub mod receiver_buffer;
#[macro_use]
pub mod session;

pub mod app;
pub mod interceptor;
pub mod interceptor_mls;
pub mod streaming;
pub mod timer;

mod fire_and_forget;
mod testutils;
mod transmitter;

mod channel_endpoint;

pub use fire_and_forget::FireAndForgetConfiguration;
pub use session::SessionMessage;
pub use slim_datapath::messages::utils::SlimHeaderFlags;
pub use streaming::StreamingConfiguration;

use serde::Deserialize;
use session::{AppChannelReceiver, MessageDirection};
use slim_datapath::messages::Agent;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

pub use errors::ServiceError;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_config::component::configuration::{Configuration, ConfigurationError};
use slim_config::component::id::{ID, Kind};
use slim_config::component::{Component, ComponentBuilder, ComponentError};
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_controller::api::proto::api::v1::controller_service_server::ControllerServiceServer;
use slim_controller::service::ControllerService;
use slim_datapath::api::PubSubServiceServer;
use slim_datapath::message_processing::MessageProcessor;

use crate::app::App;

// Define the kind of the component as static string
pub const KIND: &str = "slim";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PubsubConfig {
    /// Pubsub GRPC server settings
    #[serde(default)]
    servers: Vec<ServerConfig>,

    /// Pubsub client config to connect to other services
    #[serde(default)]
    clients: Vec<ClientConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ControllerConfig {
    /// Controller GRPC server settings
    #[serde(default)]
    server: Option<ServerConfig>,

    /// Controller client config to connect to control plane
    #[serde(default)]
    client: Option<ClientConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceConfiguration {
    /// Pubsub API configuration
    #[serde(default)]
    pub pubsub: PubsubConfig,

    /// Controller API configuration
    #[serde(default)]
    pub controller: ControllerConfig,
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        ServiceConfiguration::default()
    }

    pub fn with_server(mut self, server: Vec<ServerConfig>) -> Self {
        self.pubsub.servers = server;
        self
    }

    pub fn with_client(mut self, clients: Vec<ClientConfig>) -> Self {
        self.pubsub.clients = clients;
        self
    }

    pub fn servers(&self) -> &[ServerConfig] {
        self.pubsub.servers.as_ref()
    }

    pub fn clients(&self) -> &[ClientConfig] {
        &self.pubsub.clients
    }

    pub fn controller_server(&self) -> Option<&ServerConfig> {
        self.controller.server.as_ref()
    }

    pub fn controller_client(&self) -> Option<&ClientConfig> {
        self.controller.client.as_ref()
    }

    pub fn build_server(&self, id: ID) -> Result<Service, ServiceError> {
        let service = Service::new(id).with_config(self.clone());
        Ok(service)
    }
}

impl Configuration for ServiceConfiguration {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate client and server configurations
        for server in self.pubsub.servers.iter() {
            server.validate()?;
        }
        for client in &self.pubsub.clients {
            client.validate()?;
        }

        // Validate the control server and client
        if let Some(server) = self.controller.server.as_ref() {
            server.validate()?;
        }
        if let Some(client) = self.controller.client.as_ref() {
            client.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Service {
    /// id of the service
    id: ID,

    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// controller service
    controller: Arc<ControllerService>,

    /// cancellation tokens to stop the servers main loop
    controller_cancellation_token: CancellationToken,

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

        // create the controller service
        let controller = Arc::new(ControllerService::new(message_processor.clone()));

        Service {
            id,
            message_processor,
            controller,
            controller_cancellation_token: CancellationToken::new(),
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

    /// Run the service
    pub async fn run(&mut self) -> Result<(), ServiceError> {
        // Check that at least one client or server is configured
        if self.config.servers().is_empty() && self.config.pubsub.clients.is_empty() {
            return Err(ServiceError::ConfigError(
                "no pubsub server or clients configured".to_string(),
            ));
        }

        for server in self.config.pubsub.servers.iter() {
            info!("starting server {}", server.endpoint);
            self.run_server(server)?;
        }

        for client in self.config.pubsub.clients.iter() {
            info!("connecting client to {}", client.endpoint);
            _ = self.connect(client).await?;
        }

        // Controller service
        if self.config.controller_server().is_some() {
            info!("starting controller server");
            self.serve_controller()?;
        }

        if let Some(controller_client) = self.config.controller_client() {
            info!(
                "connecting controller client {}",
                controller_client.endpoint
            );
            let channel = controller_client
                .to_channel()
                .map_err(|e| ServiceError::ConfigError(e.to_string()))?;

            self.controller
                .connect(channel)
                .await
                .expect("error connecting controller client");
        }

        Ok(())
    }

    // APP APIs
    pub async fn create_app<P, V>(
        &self,
        app_name: &Agent,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<(App<P, V>, AppChannelReceiver), ServiceError>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        debug!(%app_name, "creating app");

        // Channels to communicate with SLIM
        let (conn_id, tx_slim, rx_slim) = self.message_processor.register_local_connection();

        // Channels to communicate with the local app
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
        );

        // start message processing using the rx channel
        app.process_messages(rx_slim);

        // return the app instance and the rx channel
        Ok((app, rx_app))
    }

    pub fn run_server(&self, config: &ServerConfig) -> Result<(), ServiceError> {
        info!(%config, "server configured: setting it up");
        let server_future = config
            .to_server_future(&[PubSubServiceServer::from_arc(
                self.message_processor.clone(),
            )])
            .map_err(|e| ServiceError::ConfigError(e.to_string()))?;

        // clone the watcher to be notified when the service is shutting down
        let drain_rx = self.watch.clone();

        // create a new cancellation token
        let token = CancellationToken::new();
        self.cancellation_tokens
            .write()
            .insert(config.endpoint.clone(), token.clone());

        // spawn server acceptor in a new task
        tokio::spawn(async move {
            debug!("starting server main loop");
            let shutdown = drain_rx.signaled();

            info!("running service");
            tokio::select! {
                res = server_future => {
                    match res {
                        Ok(_) => {
                            info!("server shutdown");
                        }
                        Err(e) => {
                            info!("server error: {:?}", e);
                        }
                    }
                }
                _ = shutdown => {
                    info!("shutting down server");
                }
                _ = token.cancelled() => {
                    info!("cancellation token triggered: shutting down server");
                }
            }
        });

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

        match config.to_channel() {
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

        self.message_processor
            .disconnect(conn)
            .map_err(|e| ServiceError::DisconnectError(e.to_string()))
    }

    pub fn get_connection_id(&self, endpoint: &str) -> Option<u64> {
        self.clients.read().get(endpoint).cloned()
    }

    fn serve_controller(&self) -> Result<(), ServiceError> {
        let controller_server_config = match self.config.controller_server() {
            Some(s) => s.clone(),
            None => {
                error!("no controller server configured");
                return Err(ServiceError::ConfigError(
                    "no controller server configured".into(),
                ));
            }
        };

        info!("controller server configured: setting it up");

        let server_future = controller_server_config
            .to_server_future(&[ControllerServiceServer::from_arc(self.controller.clone())])
            .map_err(|e| ServiceError::ConfigError(e.to_string()))?;

        // clone the watcher to be notified when the service is shutting down
        let drain_rx = self.watch.clone();

        let token = self.controller_cancellation_token.clone();

        tokio::spawn(async move {
            info!("controller server running");
            let shutdown = drain_rx.signaled();

            tokio::select! {
                res = server_future => {
                    match res {
                        Ok(_) => info!("controller server shutdown"),
                        Err(e) => error!("controller server error: {:?}", e),
                    }
                }
                _ = shutdown => {
                    info!("shutting down controller server");
                }
                _ = token.cancelled() => {
                    info!("Shutting down controller server (cancellation token triggered)");
                }
            }
        });

        Ok(())
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
    // Create a new NopComponentBuilder
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
        let id = ID::new_with_name(ServiceBuilder::kind(), name)
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
    use crate::session::SessionConfig;

    use super::*;
    use slim_auth::simple::SimpleGroup;
    use slim_config::grpc::server::ServerConfig;
    use slim_config::tls::server::TlsServerConfig;
    use slim_datapath::api::MessageType;
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
        let subscriber_agent = Agent::from_strings("cisco", "default", "subscriber_agent", 0);
        let (sub_app, mut sub_rx) = service
            .create_app(
                &subscriber_agent,
                SimpleGroup::new("a", "group"),
                SimpleGroup::new("a", "group"),
            )
            .await
            .expect("failed to create agent");

        // create a publisher
        let publisher_agent = Agent::from_strings("cisco", "default", "publisher_agent", 0);
        let (pub_app, _rx) = service
            .create_app(
                &publisher_agent,
                SimpleGroup::new("a", "group"),
                SimpleGroup::new("a", "group"),
            )
            .await
            .expect("failed to create agent");

        // sleep to allow the subscription to be processed
        time::sleep(Duration::from_millis(100)).await;

        // NOTE: here we don't call any subscribe as the publisher and the subscriber
        // are in the same service (so they share one single slim instance) and the
        // subscription is done automatically.

        // create a fire and forget session
        let session_info = pub_app
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                None,
            )
            .await
            .unwrap();

        // publish a message
        let message_blob = "very complicated message".as_bytes().to_vec();
        pub_app
            .publish(
                session_info.clone(),
                subscriber_agent.agent_type(),
                Some(subscriber_agent.agent_id()),
                message_blob.clone(),
            )
            .await
            .unwrap();

        // wait for the message to arrive
        let msg = sub_rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // make sure message is a publication
        assert!(msg.message.message_type.is_some());
        let publ = match msg.message.message_type.unwrap() {
            MessageType::Publish(p) => p,
            _ => panic!("expected a publication"),
        };

        // make sure message is correct
        assert_eq!(publ.get_payload().blob, message_blob);

        // make also sure the session ids correspond
        assert_eq!(session_info.id, msg.info.id);

        // Now remove the session from the 2 agents
        pub_app.delete_session(session_info.id).await.unwrap();
        sub_app.delete_session(session_info.id).await.unwrap();

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

        // register local agent
        let agent = Agent::from_strings("cisco", "default", "session_agent", 0);
        let (app, _) = service
            .create_app(
                &agent,
                SimpleGroup::new("a", "group"),
                SimpleGroup::new("a", "group"),
            )
            .await
            .expect("failed to create agent");

        //////////////////////////// ff session ////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());
        let session_info = app
            .create_session(session_config.clone(), None)
            .await
            .expect("failed to create session");

        // check the configuration we get is the one we used to create the session
        let session_config_ret = app
            .get_session_config(session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set config for the session
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());

        app.set_session_config(&session_config, Some(session_info.id))
            .await
            .expect("failed to set session config");

        // get session config
        let session_config_ret = app
            .get_session_config(session_info.id)
            .await
            .expect("failed to get session config");
        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set default session config
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());
        app.set_session_config(&session_config, None)
            .await
            .expect("failed to set default session config");

        // get default session config
        let session_config_ret = app
            .get_default_session_config(session::SessionType::FireAndForget)
            .await
            .expect("failed to get default session config");

        assert_eq!(session_config, session_config_ret);
        ////////////////////////////////////////////////////////////////////////////////////////////////////////////////

        ////////////// stream session //////////////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            false,
            Some(1000),
            Some(time::Duration::from_secs(123)),
            false,
        ));
        let session_info = app
            .create_session(session_config.clone(), None)
            .await
            .expect("failed to create session");
        // get session config
        let session_config_ret = app
            .get_session_config(session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Sender,
            None,
            false,
            Some(2000),
            Some(time::Duration::from_secs(1234)),
            false,
        ));

        app.set_session_config(&session_config, Some(session_info.id))
            .await
            .expect_err("we should not be allowed to set a different direction");

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            false,
            Some(2000),
            Some(time::Duration::from_secs(1234)),
            false,
        ));

        app.set_session_config(&session_config, Some(session_info.id))
            .await
            .expect("failed to set session config");

        // get session config
        let session_config_ret = app
            .get_session_config(session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set default session config
        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Sender,
            None,
            false,
            Some(20000),
            Some(time::Duration::from_secs(12345)),
            false,
        ));

        app.set_session_config(&session_config, None)
            .await
            .expect_err("we should not be allowed to set a sender direction as default");

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            false,
            Some(20000),
            Some(time::Duration::from_secs(123456)),
            false,
        ));

        app.set_session_config(&session_config, None)
            .await
            .expect("failed to set default session config");

        // get default session config
        let session_config_ret = app
            .get_default_session_config(session::SessionType::Streaming)
            .await
            .expect("failed to get default session config");

        assert_eq!(session_config, session_config_ret);
        ////////////////////////////////////////////////////////////////////////////////////////////////////////////////
    }
}

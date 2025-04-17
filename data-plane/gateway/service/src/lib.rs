// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod errors;
pub mod producer_buffer;
pub mod receiver_buffer;
#[macro_use]
pub mod session;
pub mod streaming;
pub mod timer;

mod fire_and_forget;
mod request_response;
mod session_layer;

pub use agp_datapath::messages::utils::AgpHeaderFlags;
pub use fire_and_forget::FireAndForgetConfiguration;
pub use request_response::RequestResponseConfiguration;
pub use session::SessionMessage;
pub use streaming::StreamingConfiguration;

use agp_datapath::messages::{Agent, AgentType};
use agp_datapath::pubsub::MessageType;
use serde::Deserialize;
use session::{AppChannelReceiver, MessageDirection};
use session_layer::SessionLayer;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::Status;
use tracing::{debug, error, info};

use agp_config::component::configuration::{Configuration, ConfigurationError};
use agp_config::component::id::{ID, Kind};
use agp_config::component::{Component, ComponentBuilder, ComponentError};
use agp_config::grpc::client::ClientConfig;
use agp_config::grpc::server::ServerConfig;
use agp_datapath::message_processing::MessageProcessor;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::pub_sub_service_server::PubSubServiceServer;
pub use errors::ServiceError;

// Define the kind of the component as static string
pub const KIND: &str = "gateway";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceConfiguration {
    /// The GRPC server settings
    #[serde(default)]
    servers: Vec<ServerConfig>,

    /// Client config to connect to other services
    #[serde(default)]
    clients: Vec<ClientConfig>,
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        ServiceConfiguration::default()
    }

    pub fn with_server(self, server: Vec<ServerConfig>) -> Self {
        ServiceConfiguration {
            servers: server,
            ..self
        }
    }

    pub fn with_client(self, clients: Vec<ClientConfig>) -> Self {
        ServiceConfiguration { clients, ..self }
    }

    pub fn servers(&self) -> &[ServerConfig] {
        self.servers.as_ref()
    }

    pub fn clients(&self) -> &[ClientConfig] {
        &self.clients
    }

    pub fn build_server(&self, id: ID) -> Result<Service, ServiceError> {
        let service = Service::new(id).with_config(self.clone());
        Ok(service)
    }
}

impl Configuration for ServiceConfiguration {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate client and server configurations
        for server in self.servers.iter() {
            server.validate()?;
        }

        for client in self.clients.iter() {
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

    /// the configuration of the service
    config: ServiceConfiguration,

    /// pool of sessions for the service
    session_layers: RwLock<HashMap<Agent, Arc<SessionLayer>>>,

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

        Service {
            id,
            message_processor: Arc::new(MessageProcessor::with_drain_channel(watch.clone())),
            config: ServiceConfiguration::new(),
            session_layers: RwLock::new(HashMap::new()),
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
        if self.config.servers().is_empty() && self.config.clients.is_empty() {
            return Err(ServiceError::ConfigError(
                "no server or clients configured".to_string(),
            ));
        }

        for server in self.config.servers.iter() {
            info!("starting server {}", server.endpoint);
            self.run_server(server)?;
        }

        for client in self.config.clients.iter() {
            info!("connecting client to {}", client.endpoint);
            _ = self.connect(client).await?;
        }

        Ok(())
    }

    // APP APIs
    pub async fn create_agent(
        &self,
        agent_name: &Agent,
    ) -> Result<AppChannelReceiver, ServiceError> {
        // get a write lock on the session layers
        let mut session_layers = self.session_layers.write().await;

        // make sure the agent is not already registered
        if session_layers.contains_key(agent_name) {
            error!(%agent_name, "agent already exists");
            return Err(ServiceError::AgentAlreadyRegistered);
        }

        debug!(%agent_name, "creating agent");

        // Channels to communicate with the gateway
        let (conn_id, tx_gw, rx_gw) = self.message_processor.register_local_connection();

        // Channels to communicate with the local app
        // TODO(msardara): make the buffer size configurable
        let (tx_app, rx_app) = mpsc::channel(128);

        // create session layer
        let session_layer = Arc::new(SessionLayer::new(agent_name, conn_id, tx_gw, tx_app));

        // register agent within session layers
        session_layers.insert(agent_name.clone(), session_layer.clone());

        // start message processing using the rx channel
        self.process_messages(agent_name.clone(), session_layer, rx_gw);

        // return the rx channel
        Ok(rx_app)
    }

    pub async fn delete_agent(&self, agent_name: &Agent) -> Result<(), ServiceError> {
        // get a write lock on the session layers
        let mut session_layers = self.session_layers.write().await;

        match session_layers.remove(agent_name) {
            None => {
                error!("agent {:?} not found", agent_name);
                Err(ServiceError::AgentNotFound(agent_name.to_string()))
            }
            Some(layer) => {
                info!("deleting agent {}", agent_name);

                // disconnect local connection (this should also end the processing loop)
                self.message_processor
                    .disconnect(layer.conn_id())
                    .map_err(|e| {
                        error!("error disconnecting agent: {}", e);
                        ServiceError::DisconnectError(e.to_string())
                    })
            }
        }
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

    async fn send_message(
        &self,
        agent: &Agent,
        msg: Message,
        info: Option<session::Info>,
    ) -> Result<(), ServiceError> {
        self.with_session_layer(agent, async move |session: &Arc<SessionLayer>| {
            // save session id for later use
            match info {
                Some(info) => {
                    let id = info.id;
                    session
                        .handle_message(SessionMessage::from((msg, info)), MessageDirection::South)
                        .await
                        .map_err(|e| {
                            error!("error sending the message to session {}: {}", id, e);
                            ServiceError::SessionError(e.to_string())
                        })
                }
                None => session.tx_gw().send(Ok(msg)).await.map_err(|e| {
                    error!("error sending message {}", e);
                    ServiceError::MessageSendingError(e.to_string())
                }),
            }
        })
        .await
    }

    pub async fn subscribe(
        &self,
        local_agent: &Agent,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: Option<u64>,
    ) -> Result<(), ServiceError> {
        debug!("subscribe to {}/{:?}", agent_type, agent_id);

        let header = if let Some(c) = conn {
            Some(AgpHeaderFlags::default().with_forward_to(c))
        } else {
            Some(AgpHeaderFlags::default())
        };
        let msg = Message::new_subscribe(local_agent, agent_type, agent_id, header);
        self.send_message(local_agent, msg, None).await
    }

    pub async fn unsubscribe(
        &self,
        local_agent: &Agent,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: Option<u64>,
    ) -> Result<(), ServiceError> {
        debug!("unsubscribe from {}/{:?}", agent_type, agent_id);

        let header = if let Some(c) = conn {
            Some(AgpHeaderFlags::default().with_forward_to(c))
        } else {
            Some(AgpHeaderFlags::default())
        };
        let msg = Message::new_subscribe(local_agent, agent_type, agent_id, header);
        self.send_message(local_agent, msg, None).await
    }

    pub async fn set_route(
        &self,
        local_agent: &Agent,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        debug!("set route to {}/{:?}", agent_type, agent_id);

        // send a message with subscription from
        let msg = Message::new_subscribe(
            local_agent,
            agent_type,
            agent_id,
            Some(AgpHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(local_agent, msg, None).await
    }

    pub async fn remove_route(
        &self,
        local_agent: &Agent,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        debug!("unset route to {}/{:?}", agent_type, agent_id);

        //  send a message with unsubscription from
        let msg = Message::new_subscribe(
            local_agent,
            agent_type,
            agent_id,
            Some(AgpHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(local_agent, msg, None).await
    }

    pub async fn publish_to(
        &self,
        source: &Agent,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        forward_to: u64,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            source,
            session_info,
            agent_type,
            agent_id,
            AgpHeaderFlags::default().with_forward_to(forward_to),
            blob,
        )
        .await
    }

    pub async fn publish(
        &self,
        source: &Agent,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            source,
            session_info,
            agent_type,
            agent_id,
            AgpHeaderFlags::default(),
            blob,
        )
        .await
    }

    pub async fn publish_with_flags(
        &self,
        source: &Agent,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        flags: AgpHeaderFlags,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        debug!(
            "sending publication to {}/{:?}. Flags: {}",
            agent_type, agent_id, flags
        );

        let msg = Message::new_publish(source, agent_type, agent_id, Some(flags), "msg", blob);

        self.send_message(source, msg, Some(session_info)).await
    }

    /// Receive messages from gateway and forward them to the appropriate session
    fn process_messages(
        &self,
        agent: Agent,
        session_layer: Arc<SessionLayer>,
        mut rx: mpsc::Receiver<Result<Message, Status>>,
    ) {
        // clone drain watch
        let watch = self.watch.clone();

        tokio::spawn(async move {
            debug!("starting message processing loop for agent {}", agent);

            // subscribe for local agent running this loop
            let subscribe_msg =
                Message::new_subscribe(&agent, agent.agent_type(), Some(agent.agent_id()), None);
            let tx = session_layer.tx_gw();
            tx.send(Ok(subscribe_msg))
                .await
                .expect("error sending subscription");

            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                debug!("no more messages to process");
                                break;
                            }
                            Some(msg) => {
                                match msg {
                                    Ok(msg) => {
                                        debug!("received message in service processing: {:?}", msg);

                                        // filter only the messages of type publish
                                        match msg.message_type.as_ref() {
                                            Some(MessageType::Publish(_)) => {},
                                            None => {
                                                continue;
                                            }
                                            _ => {
                                                continue;
                                            }
                                        }

                                        // Handle the message
                                        let res = session_layer
                                            .handle_message(SessionMessage::from(msg), MessageDirection::North)
                                            .await;

                                        if let Err(e) = res {
                                            error!("error handling message: {}", e);
                                        }
                                    }
                                    Err(e) => {
                                        error!("error: {}", e);

                                        // if internal error, forward it to application
                                        let tx_app = session_layer.tx_app();
                                        tx_app.send(Err(errors::SessionError::Forward(e.to_string())))
                                            .await
                                            .expect("error sending error to application");
                                    }
                                }
                            }
                        }
                    }
                    _ = watch.clone().signaled() => {
                        debug!("shutting down processing on drain for agent: {}", agent);
                        break;
                    }
                }
            }
        });
    }

    async fn with_session_layer<F, T>(&self, agent: &Agent, f: F) -> Result<T, ServiceError>
    where
        F: AsyncFnOnce(&Arc<SessionLayer>) -> Result<T, ServiceError>,
    {
        // get a read lock on the session layers
        let session_layers = self.session_layers.read().await;

        // check if agent was registered
        let layer = session_layers.get(agent);

        if layer.is_none() {
            error!("agent {} not found", agent);
            return Err(ServiceError::AgentNotFound(agent.to_string()));
        }

        let layer = layer.unwrap();

        f(layer).await
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        agent: &Agent,
        session_config: session::SessionConfig,
    ) -> Result<session::Info, ServiceError> {
        self.with_session_layer(agent, async move |layer: &Arc<SessionLayer>| {
            layer
                .create_session(session_config, None)
                .await
                .map_err(|e| {
                    error!("error creating session: {}", e);
                    ServiceError::SessionError(e.to_string())
                })
        })
        .await
    }

    /// Set config for a session
    pub async fn set_session_config(
        &self,
        agent: &Agent,
        session_config: &session::SessionConfig,
        session_id: Option<session::Id>,
    ) -> Result<(), ServiceError> {
        self.with_session_layer(agent, async move |layer: &Arc<SessionLayer>| {
            // set the session config
            layer
                .set_session_config(session_config, session_id)
                .await
                .map_err(|e| {
                    error!("error setting session config: {}", e);
                    ServiceError::SessionError(e.to_string())
                })
        })
        .await
    }

    /// Get config for a session
    pub async fn get_session_config(
        &self,
        agent: &Agent,
        session_id: session::Id,
    ) -> Result<session::SessionConfig, ServiceError> {
        self.with_session_layer(agent, async move |layer: &Arc<SessionLayer>| {
            // get the session config
            layer.get_session_config(session_id).await.map_err(|e| {
                error!("error getting session config: {}", e);
                ServiceError::SessionError(e.to_string())
            })
        })
        .await
    }

    /// Get default session config
    pub async fn get_default_session_config(
        &self,
        agent: &Agent,
        session_type: session::SessionType,
    ) -> Result<session::SessionConfig, ServiceError> {
        self.with_session_layer(agent, async move |layer: &Arc<SessionLayer>| {
            // get the default session config
            layer
                .get_default_session_config(session_type)
                .await
                .map_err(|e| {
                    error!("error getting default session config: {}", e);
                    ServiceError::SessionError(e.to_string())
                })
        })
        .await
    }

    /// delete a session
    pub async fn delete_session(
        &self,
        agent: &Agent,
        session_id: session::Id,
    ) -> Result<(), ServiceError> {
        self.with_session_layer(agent, async move |layer: &Arc<SessionLayer>| {
            // delete the session
            match layer.remove_session(session_id).await {
                true => Ok(()),
                false => {
                    error!("error deleting session");
                    Err(ServiceError::SessionError("session not found".to_string()))
                }
            }
        })
        .await
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
    use agp_config::grpc::server::ServerConfig;
    use agp_config::tls::server::TlsServerConfig;
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
        let mut sub_rx = service
            .create_agent(&subscriber_agent)
            .await
            .expect("failed to create agent");

        // create a publisher
        let publisher_agent = Agent::from_strings("cisco", "default", "publisher_agent", 0);
        let _pub_rx = service
            .create_agent(&publisher_agent)
            .await
            .expect("failed to create agent");

        // sleep to allow the subscription to be processed
        time::sleep(Duration::from_millis(100)).await;

        // NOTE: here we don't call any subscribe as the publisher and the subscriber
        // are in the same service (so they share one single gateway) and the
        // subscription is done automatically.

        // create a fire and forget session
        let session_info = service
            .create_session(
                &publisher_agent,
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
            )
            .await
            .unwrap();

        // publish a message
        let message_blob = "very complicated message".as_bytes().to_vec();
        service
            .publish(
                &publisher_agent,
                session_info.clone(),
                &subscriber_agent.agent_type(),
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
        service
            .delete_session(&publisher_agent, session_info.id)
            .await
            .unwrap();
        service
            .delete_session(&subscriber_agent, session_info.id)
            .await
            .unwrap();

        // And remove the agents
        service.delete_agent(&publisher_agent).await.unwrap();
        service.delete_agent(&subscriber_agent).await.unwrap();

        // sleep to allow the deletion to be processed
        time::sleep(Duration::from_millis(100)).await;

        // This should also trigger a stop of the message processing loop.
        // Make sure the loop stopped by checking the logs
        assert!(logs_contain("no more messages to process"));
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
        let _ = service
            .create_agent(&agent)
            .await
            .expect("failed to create agent");

        //////////////////////////// ff session ////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());
        let session_info = service
            .create_session(&agent, session_config.clone())
            .await
            .expect("failed to create session");

        // check the configuration we get is the one we used to create the session
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set config for the session
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());

        service
            .set_session_config(&agent, &session_config, Some(session_info.id))
            .await
            .expect("failed to set session config");

        // get session config
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
            .await
            .expect("failed to get session config");
        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set default session config
        let session_config = SessionConfig::FireAndForget(FireAndForgetConfiguration::default());
        service
            .set_session_config(&agent, &session_config, None)
            .await
            .expect("failed to set default session config");

        // get default session config
        let session_config_ret = service
            .get_default_session_config(&agent, session::SessionType::FireAndForget)
            .await
            .expect("failed to get default session config");

        assert_eq!(session_config, session_config_ret);
        ////////////////////////////////////////////////////////////////////////////////////////////////////////////////

        /////////////request response session //////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig::RequestResponse(RequestResponseConfiguration {
            timeout: Duration::from_secs(20000),
        });
        let session_info = service
            .create_session(&agent, session_config.clone())
            .await
            .expect("failed to create session");

        // get session config
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        let session_config = SessionConfig::RequestResponse(RequestResponseConfiguration {
            timeout: Duration::from_secs(21345),
        });

        service
            .set_session_config(&agent, &session_config, Some(session_info.id))
            .await
            .expect("failed to set session config");

        // get session config
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        // set default session config
        let session_config = SessionConfig::RequestResponse(RequestResponseConfiguration {
            timeout: Duration::from_secs(213456),
        });
        service
            .set_session_config(&agent, &session_config, None)
            .await
            .expect("failed to set default session config");

        // get default session config
        let session_config_ret = service
            .get_default_session_config(&agent, session::SessionType::RequestResponse)
            .await
            .expect("failed to get default session config");

        assert_eq!(session_config, session_config_ret);
        ////////////////////////////////////////////////////////////////////////////////////////////////////////////////

        ////////////// stream session //////////////////////////////////////////////////////////////////////////////////
        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            Some(1000),
            Some(time::Duration::from_secs(123)),
        ));
        let session_info = service
            .create_session(&agent, session_config.clone())
            .await
            .expect("failed to create session");
        // get session config
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
            .await
            .expect("failed to get session config");

        assert_eq!(
            session_config, session_config_ret,
            "session config mismatch"
        );

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Sender,
            None,
            Some(2000),
            Some(time::Duration::from_secs(1234)),
        ));

        service
            .set_session_config(&agent, &session_config, Some(session_info.id))
            .await
            .expect_err("we should not be allowed to set a different direction");

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            Some(2000),
            Some(time::Duration::from_secs(1234)),
        ));

        service
            .set_session_config(&agent, &session_config, Some(session_info.id))
            .await
            .expect("failed to set session config");

        // get session config
        let session_config_ret = service
            .get_session_config(&agent, session_info.id)
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
            Some(20000),
            Some(time::Duration::from_secs(12345)),
        ));

        service
            .set_session_config(&agent, &session_config, None)
            .await
            .expect_err("we should not be allowed to set a sender direction as default");

        let session_config = SessionConfig::Streaming(StreamingConfiguration::new(
            session::SessionDirection::Receiver,
            None,
            Some(20000),
            Some(time::Duration::from_secs(123456)),
        ));

        service
            .set_session_config(&agent, &session_config, None)
            .await
            .expect("failed to set default session config");

        // get default session config
        let session_config_ret = service
            .get_default_session_config(&agent, session::SessionType::Streaming)
            .await
            .expect("failed to get default session config");

        assert_eq!(session_config, session_config_ret);
        ////////////////////////////////////////////////////////////////////////////////////////////////////////////////
    }
}

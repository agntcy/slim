// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub mod errors;

use agp_datapath::messages::utils::{
    create_agp_header, create_default_service_header, create_publication, create_subscription_from,
    create_subscription_to_forward, create_unsubscription_from, create_unsubscription_to_forward,
};
use agp_datapath::messages::{Agent, AgentType};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::Status;
use tracing::{debug, error, info};

use agp_config::component::configuration::{Configuration, ConfigurationError};
use agp_config::component::id::{Kind, ID};
use agp_config::component::{Component, ComponentBuilder, ComponentError};
use agp_config::grpc::client::ClientConfig;
use agp_config::grpc::server::ServerConfig;
use agp_datapath::message_processing::MessageProcessor;
use agp_datapath::pubsub::proto::pubsub::v1::pub_sub_service_server::PubSubServiceServer;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
pub use errors::ServiceError;

// Define the kind of the component as static string
pub const KIND: &str = "gateway";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceConfiguration {
    /// The GRPC server settings
    #[serde(default)]
    server: Option<ServerConfig>,

    /// Client config to connect to other services
    #[serde(default)]
    clients: Vec<ClientConfig>,
}

impl ServiceConfiguration {
    pub fn new() -> Self {
        ServiceConfiguration::default()
    }

    pub fn with_server(self, server: Option<ServerConfig>) -> Self {
        ServiceConfiguration { server, ..self }
    }

    pub fn with_client(self, clients: Vec<ClientConfig>) -> Self {
        ServiceConfiguration { clients, ..self }
    }

    pub fn server(&self) -> Option<&ServerConfig> {
        self.server.as_ref()
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
        if let Some(server) = self.server.as_ref() {
            server.validate()?;
        }

        for client in self.clients.iter() {
            client.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct LocalAgent {
    /// name of the agent
    name: Agent,

    /// channels used to send messages to the message processor
    tx_channel: tokio::sync::mpsc::Sender<Result<Message, Status>>,
}

#[derive(Debug)]
pub struct Service {
    /// id of the service
    id: ID,

    /// underlying message processor
    message_processor: Arc<MessageProcessor>,

    /// local agent status. this is optional
    agent: Option<LocalAgent>,

    /// the configuration of the service
    config: ServiceConfiguration,

    /// drain watch to shutdown the service
    watch: drain::Watch,

    /// signal to shutdown the service
    signal: drain::Signal,
}

impl Service {
    /// Create a new Service
    pub fn new(id: ID) -> Self {
        let (signal, watch) = drain::channel();

        Service {
            id,
            agent: None,
            message_processor: Arc::new(MessageProcessor::with_drain_channel(watch.clone())),
            config: ServiceConfiguration::new(),
            watch,
            signal,
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

    /// get signal used to shutdown the service
    /// NOTE: this method consumes the service!
    pub fn signal(self) -> drain::Signal {
        self.signal
    }

    /// Run the service
    pub async fn run(&self) -> Result<(), ServiceError> {
        // Check that at least one client or server is configured
        if self.config.server().is_none() && self.config.clients.is_empty() {
            return Err(ServiceError::ConfigError(
                "no server or clients configured".to_string(),
            ));
        }

        if self.config.server().is_some() {
            info!("starting server");
            self.serve(None)?;
        }

        for (i, client) in self.config.clients.iter().enumerate() {
            info!("connecting client {} to {}", i, client.endpoint);

            let channel = client
                .to_channel()
                .map_err(|e| ServiceError::ConfigError(e.to_string()))?;

            self.message_processor
                .connect(channel, None, None, None)
                .await
                .expect("error connecting client");
        }

        Ok(())
    }

    // APP APIs
    // TODO(msardara): unit tests the APIs
    pub fn create_agent(&mut self, agent_name: Agent) -> mpsc::Receiver<Result<Message, Status>> {
        let (tx, rx) = self.message_processor.register_local_connection();
        self.agent = Some(LocalAgent {
            name: agent_name,
            tx_channel: tx,
        });

        rx
    }

    pub fn serve(&self, new_config: Option<ServerConfig>) -> Result<(), ServiceError> {
        // if no new config is provided, try to get it from local configuration
        let config = match &new_config {
            Some(c) => c,
            None => {
                // make sure at least one client is configured
                if self.config.server().is_none() {
                    error!("no server configured");
                    return Err(ServiceError::ConfigError(
                        "no server configured".to_string(),
                    ));
                }

                // get the server config
                self.config.server().unwrap()
            }
        };

        info!("server configured: setting it up");
        let server_future = config
            .to_server_future(&[PubSubServiceServer::from_arc(
                self.message_processor.clone(),
            )])
            .map_err(|e| ServiceError::ConfigError(e.to_string()))?;

        // clone the watcher to be notified when the service is shutting down
        let drain_rx = self.watch.clone();

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
            }
        });

        Ok(())
    }

    pub async fn connect(&mut self, new_config: Option<ClientConfig>) -> Result<u64, ServiceError> {
        // here the agent must be configured
        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }

        // if no new config is provided, try to get it from local configuration
        let config = match &new_config {
            Some(c) => c,
            None => {
                // make sure at least one client is configured
                if self.config.clients.is_empty() {
                    error!("no client configured");
                    return Err(ServiceError::ConfigError(
                        "no client configured".to_string(),
                    ));
                }

                // get the first client
                &self.config.clients[0]
            }
        };

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

                match ret {
                    Err(e) => {
                        error!("connection error: {:?}", e);
                        Err(ServiceError::ConnectionError(e.to_string()))
                    }
                    Ok(conn_id) => Ok(conn_id.1),
                }
            }
        }
    }

    pub fn disconnect(&mut self, conn: u64) -> Result<(), ServiceError> {
        info!("disconnect from conn {}", conn);
        if self.message_processor.disconnect(conn).is_err() {
            return Err(ServiceError::DisconnectError);
        }
        Ok(())
    }

    pub async fn subscribe(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }
        let agent = self.agent.as_ref().unwrap();
        let msg = create_subscription_to_forward(&agent.name, agent_type, agent_id, conn);
        match agent.tx_channel.send(Ok(msg)).await {
            Err(e) => {
                error!("error sending the subscription {:?}", e);
                Err(ServiceError::SubscriptionError(e.to_string()))
            }
            Ok(_) => Ok(()),
        }
    }

    pub async fn unsubscribe(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }
        let agent = self.agent.as_ref().unwrap();
        let msg = create_unsubscription_to_forward(&agent.name, agent_type, agent_id, conn);
        match agent.tx_channel.send(Ok(msg)).await {
            Err(e) => {
                error!("error sending the unsubscription {:?}", e);
                Err(ServiceError::UnsubscriptionError(e.to_string()))
            }
            Ok(_) => Ok(()),
        }
    }

    pub async fn set_route(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        debug!("set route to {:?}/{:?}", agent_type, agent_id);

        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }
        // send a message with subscription from
        let msg = create_subscription_from(agent_type, agent_id, conn);
        if let Err(e) = self.agent.as_ref().unwrap().tx_channel.send(Ok(msg)).await {
            error!("error on set route to {:?}", e);
            return Err(ServiceError::SetRouteError(e.to_string()));
        }
        Ok(())
    }

    pub async fn remove_route(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }
        //  send a message with unsubscription from
        let msg = create_unsubscription_from(agent_type, agent_id, conn);
        if let Err(e) = self.agent.as_ref().unwrap().tx_channel.send(Ok(msg)).await {
            error!("error on remove route {:?}", e);
            return Err(ServiceError::RemoveRouteError(e.to_string()));
        }
        Ok(())
    }

    pub async fn publish(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        fanout: u32,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        self.publish_to(agent_type, agent_id, fanout, blob, None)
            .await
    }

    pub async fn publish_to(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        fanout: u32,
        blob: Vec<u8>,
        out_conn: Option<u64>,
    ) -> Result<(), ServiceError> {
        if self.agent.is_none() {
            error!("the local agent is not configured");
            return Err(ServiceError::MissingAgentError);
        }

        let agent = self.agent.as_ref().unwrap();
        let header = create_agp_header(
            &agent.name,
            agent_type,
            agent_id,
            None,
            out_conn,
            None,
            None,
        );

        let msg = create_publication(
            header,
            create_default_service_header(),
            HashMap::new(),
            fanout,
            "msg",
            blob,
        );

        debug!("sending publication {:?}", msg);

        if let Err(e) = self.agent.as_ref().unwrap().tx_channel.send(Ok(msg)).await {
            error!("error sending the publication {:?}", e);
            return Err(ServiceError::PublishError(e.to_string()));
        }
        Ok(())
    }
}

impl Component for Service {
    fn identifier(&self) -> &ID {
        &self.id
    }

    async fn start(&self) -> Result<(), ComponentError> {
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
    use super::*;
    use agp_config::grpc::server::ServerConfig;
    use agp_config::tls::server::TlsServerConfig;
    use std::time::Duration;
    use tokio::time;
    use tracing_test::traced_test;

    #[tokio::test]
    async fn test_service_configuration() {
        let config = ServiceConfiguration::new();
        assert_eq!(config.server(), None);
        assert_eq!(config.clients(), &[]);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_service_build_server() {
        let tls_config = TlsServerConfig::new().with_insecure(true);
        let server_config =
            ServerConfig::with_endpoint("0.0.0.0:12345").with_tls_settings(tls_config);
        let config = ServiceConfiguration::new().with_server(Some(server_config));
        let service = config
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
}

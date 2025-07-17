// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use serde::Deserialize;

use slim_config::component::configuration::{Configuration, ConfigurationError};
use slim_config::component::id::ID;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_datapath::message_processing::MessageProcessor;

use crate::service::ControlPlane;

/// Configuration for the Control Plane component
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    /// Controller GRPC server settings
    #[serde(default)]
    servers: Vec<ServerConfig>,

    /// Controller client config to connect to control plane
    #[serde(default)]
    clients: Vec<ClientConfig>,
}

impl Config {
    /// Create a new Config instance with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new Config instance with the given servers
    pub fn with_servers(self, servers: Vec<ServerConfig>) -> Self {
        Self { servers, ..self }
    }

    /// Create a new Config instance with the given clients
    pub fn with_clients(self, clients: Vec<ClientConfig>) -> Self {
        Self { clients, ..self }
    }

    /// Get the list of server configurations
    pub fn servers(&self) -> &[ServerConfig] {
        &self.servers
    }
    /// Get the list of client configurations
    pub fn clients(&self) -> &[ClientConfig] {
        &self.clients
    }

    /// Create a ControlPlane service instance from this configuration
    pub fn into_service(
        &self,
        id: ID,
        rx_drain: drain::Watch,
        message_processor: Arc<MessageProcessor>,
    ) -> ControlPlane {
        ControlPlane::new(
            id,
            self.servers.clone(),
            self.clients.clone(),
            rx_drain,
            message_processor,
        )
    }
}

impl Configuration for Config {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate client and server configurations
        for server in self.servers.iter() {
            server.validate()?;
        }

        for client in &self.clients {
            client.validate()?;
        }

        Ok(())
    }
}

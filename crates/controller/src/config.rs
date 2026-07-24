// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

use slim_config::client::ClientConfig;
use slim_config::component::configuration::Configuration;
use slim_config::pqc::EnforcePqcPolicy;
use slim_config::server::ServerConfig;
use slim_datapath::message_processing::MessageProcessor;

use crate::errors::ControllerError;
use crate::service::{ControlPlane, ControlPlaneSettings, from_server_config};

/// Configuration for the Control-Plane / Data-Plane component
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Applies to TLS, link negotiation, and MLS for this dataplane.
    #[serde(default)]
    pub enforce_pqc: EnforcePqcPolicy,

    /// Controller GRPC server settings
    #[serde(default)]
    pub servers: Vec<ServerConfig>,

    /// Controller client config to connect to control plane
    #[serde(default)]
    pub clients: Vec<ClientConfig>,

    /// Controller client config to connect to data-plane server nodes
    #[serde(default)]
    pub outbound_clients: Vec<ClientConfig>,
}

impl Config {
    /// Create a new Config instance with default values
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Resolved PQC policy for this dataplane (also written to TLS runtime config on normalize).
    pub fn enforce_pqc(&self) -> EnforcePqcPolicy {
        self.enforce_pqc
    }

    /// Write the dataplane policy to every TLS endpoint runtime config.
    pub fn normalize_pqc(&mut self) -> Result<(), ControllerError> {
        self.apply_pqc_policy(self.enforce_pqc)?;
        Ok(())
    }

    fn check_pqc_policy(&self) -> Result<(), ControllerError> {
        self.validate_pqc_for_endpoints(self.enforce_pqc)
    }

    fn apply_pqc_policy(&mut self, policy: EnforcePqcPolicy) -> Result<(), ControllerError> {
        for server in &mut self.servers {
            server.tls_setting.config.enforce_pqc = policy.is_enforced();
            policy
                .validate_tls_version(&server.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        for client in &mut self.clients {
            client.tls_setting.config.enforce_pqc = policy.is_enforced();
            policy
                .validate_tls_version(&client.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        for client in &mut self.outbound_clients {
            client.tls_setting.config.enforce_pqc = policy.is_enforced();
            policy
                .validate_tls_version(&client.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        Ok(())
    }

    fn validate_pqc_for_endpoints(&self, policy: EnforcePqcPolicy) -> Result<(), ControllerError> {
        for server in &self.servers {
            policy
                .validate_tls_version(&server.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        for client in &self.clients {
            policy
                .validate_tls_version(&client.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        for client in &self.outbound_clients {
            policy
                .validate_tls_version(&client.tls_setting.config.tls_version)
                .map_err(|e| ControllerError::ConfigError(e.into()))?;
        }
        Ok(())
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
        node_id: String,
        domain_name: Option<String>,
        message_processor: MessageProcessor,
        // List of server configurations for the dataplane services.
        // Used to extract connection type information required to connect to the node
        // (e.g., TLS settings). This information is used by the control plane.
        dataplane_servers: &[ServerConfig],
        auth_provider: Option<slim_auth::auth_provider::AuthProvider>,
    ) -> ControlPlane {
        let connection_details = dataplane_servers.iter().map(from_server_config).collect();

        ControlPlane::new(ControlPlaneSettings {
            id: node_id,
            domain_name,
            servers: self.servers.clone(),
            clients: self.clients.clone(),
            outbound_clients: self.outbound_clients.clone(),
            message_processor,
            connection_details,
            auth_provider,
        })
    }
}

impl Configuration for Config {
    type Error = ControllerError;

    fn validate(&self) -> Result<(), Self::Error> {
        self.check_pqc_policy()?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::server::ServerConfig;
    use slim_datapath::message_processing::MessageProcessor;

    fn create_test_server_config() -> ServerConfig {
        ServerConfig::with_endpoint("127.0.0.1:50051")
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure())
    }

    fn create_test_client_config() -> ClientConfig {
        ClientConfig::with_endpoint("http://127.0.0.1:50051")
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure())
    }

    fn server_with_pqc(enforced: bool) -> ServerConfig {
        let mut server = create_test_server_config();
        server.tls_setting.config.enforce_pqc = enforced;
        server
    }

    #[test]
    fn test_normalize_pqc_propagates_to_tls_runtime_config() {
        let mut config = Config {
            enforce_pqc: EnforcePqcPolicy::enforced(),
            servers: vec![server_with_pqc(false)],
            ..Config::default()
        };

        config.normalize_pqc().unwrap();

        assert!(config.enforce_pqc().is_enforced());
        assert!(config.servers[0].tls_setting.config.enforce_pqc);
    }

    #[test]
    fn test_config_new() {
        let config = Config::new();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
    }

    #[test]
    fn test_config_with_servers() {
        let server_config = create_test_server_config();
        let config = Config::new().with_servers(vec![server_config.clone()]);

        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0], server_config);
        assert!(config.clients.is_empty());
    }

    #[test]
    fn test_config_with_clients() {
        let client_config = create_test_client_config();
        let config = Config::new().with_clients(vec![client_config.clone()]);

        assert_eq!(config.clients.len(), 1);
        assert_eq!(config.clients[0], client_config);
        assert!(config.servers.is_empty());
    }

    #[test]
    fn test_config_servers_getter() {
        let server_config = create_test_server_config();
        let config = Config::new().with_servers(vec![server_config.clone()]);

        let servers = config.servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0], server_config);
    }

    #[test]
    fn test_config_clients_getter() {
        let client_config = create_test_client_config();
        let config = Config::new().with_clients(vec![client_config.clone()]);

        let clients = config.clients();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0], client_config);
    }

    #[test]
    fn test_config_chaining() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();

        let config = Config::new()
            .with_servers(vec![server_config.clone()])
            .with_clients(vec![client_config.clone()]);

        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.clients.len(), 1);
    }

    #[test]
    fn test_config_validate_empty() {
        let config = Config::new();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validate_with_valid_servers_and_clients() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();
        let config = Config::new()
            .with_servers(vec![server_config])
            .with_clients(vec![client_config]);

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_clone() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();

        let config1 = Config::new()
            .with_servers(vec![server_config])
            .with_clients(vec![client_config]);

        let config2 = config1.clone();

        assert_eq!(config1.servers, config2.servers);
        assert_eq!(config1.clients, config2.clients);
    }

    #[tokio::test]
    async fn test_config_into_service() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();

        let config = Config::new()
            .with_servers(vec![server_config.clone()])
            .with_clients(vec![client_config]);

        let domain_name = Some("test-domain".to_string());
        let message_processor = MessageProcessor::new();

        let _control_plane = config.into_service(
            "test-instance".to_string(),
            domain_name,
            message_processor,
            &[server_config],
            None,
        );
    }

    #[test]
    fn test_config_debug_trait() {
        let config = Config::new();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("servers"));
        assert!(debug_str.contains("clients"));
    }

    #[test]
    fn test_config_validate_with_multiple_servers() {
        let server1 = create_test_server_config();
        let server2 = ServerConfig::with_endpoint("127.0.0.1:50052")
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure());

        let config = Config::new().with_servers(vec![server1, server2]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validate_with_multiple_clients() {
        let client1 = create_test_client_config();
        let client2 = ClientConfig::with_endpoint("http://127.0.0.1:50052")
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure());

        let config = Config::new().with_clients(vec![client1, client2]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_partial_eq() {
        let config1 = Config::new();
        let config2 = Config::new();

        assert_eq!(config1, config2);

        let server_config = create_test_server_config();
        let config3 = config1.clone().with_servers(vec![server_config]);

        assert_ne!(config1, config3);
    }

    #[test]
    fn test_config_builder_pattern_reuse() {
        let base_config = Config::new();

        let config1 = base_config
            .clone()
            .with_servers(vec![create_test_server_config()]);
        let config2 = base_config
            .clone()
            .with_clients(vec![create_test_client_config()]);

        assert!(base_config.servers.is_empty());
        assert!(base_config.clients.is_empty());

        assert_eq!(config1.servers.len(), 1);
        assert!(config1.clients.is_empty());

        assert!(config2.servers.is_empty());
        assert_eq!(config2.clients.len(), 1);
    }

    #[test]
    fn test_config_overwrite_behavior() {
        let server1 = create_test_server_config();
        let server2 = ServerConfig::with_endpoint("127.0.0.1:50052")
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure());

        let config = Config::new()
            .with_servers(vec![server1])
            .with_servers(vec![server2.clone()]);

        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0], server2);
    }
}

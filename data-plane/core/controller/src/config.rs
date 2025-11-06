// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use serde::Deserialize;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_config::auth::jwt::Config as JwtConfig;
use slim_config::auth::static_jwt::Config as StaticJwtConfig;
use slim_config::component::configuration::{Configuration, ConfigurationError};
use slim_config::component::id::ID;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use slim_datapath::message_processing::MessageProcessor;

use crate::service::{ControlPlane, ControlPlaneSettings};

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TokenProviderAuthConfig {
    SharedSecret {
        data: String,
    },
    StaticJwt(StaticJwtConfig),
    Jwt(JwtConfig),
    #[default]
    None,
}

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TokenVerifierAuthConfig {
    SharedSecret {
        data: String,
    },
    Jwt(JwtConfig),
    #[default]
    None,
}

/// Configuration for the Control-Plane / Data-Plane component
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct Config {
    /// Controller GRPC server settings
    #[serde(default)]
    pub servers: Vec<ServerConfig>,

    /// Controller client config to connect to control plane
    #[serde(default)]
    pub clients: Vec<ClientConfig>,

    /// Token provider authentication configuration
    #[serde(default)]
    pub token_provider: TokenProviderAuthConfig,

    /// Token verifier authentication configuration
    #[serde(default)]
    pub token_verifier: TokenVerifierAuthConfig,
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

    /// Set the token provider authentication configuration
    pub fn with_token_provider_auth(self, auth: TokenProviderAuthConfig) -> Self {
        Self {
            token_provider: auth,
            ..self
        }
    }

    /// Set the token verifier authentication configuration
    pub fn with_token_verifier_auth(self, auth: TokenVerifierAuthConfig) -> Self {
        Self {
            token_verifier: auth,
            ..self
        }
    }

    /// Get the list of server configurations
    pub fn servers(&self) -> &[ServerConfig] {
        &self.servers
    }

    /// Get the list of client configurations
    pub fn clients(&self) -> &[ClientConfig] {
        &self.clients
    }

    fn get_token_provider_auth(&self) -> Option<AuthProvider> {
        match &self.token_provider {
            TokenProviderAuthConfig::SharedSecret { data } => {
                Some(AuthProvider::shared_secret_from_str("control-plane", data))
            }
            TokenProviderAuthConfig::StaticJwt(static_jwt_config) => {
                let provider = static_jwt_config
                    .build_static_token_provider()
                    .expect("Failed to build StaticTokenProvider");
                Some(AuthProvider::static_token(provider))
            }
            TokenProviderAuthConfig::Jwt(jwt_config) => {
                let provider = jwt_config
                    .get_provider()
                    .expect("Failed to build JwtTokenProvider");
                Some(AuthProvider::jwt_signer(provider))
            }
            TokenProviderAuthConfig::None => None,
        }
    }

    fn get_token_verifier_auth(&self) -> Option<AuthVerifier> {
        match &self.token_verifier {
            TokenVerifierAuthConfig::SharedSecret { data } => {
                Some(AuthVerifier::shared_secret_from_str("control-plane", data))
            }
            TokenVerifierAuthConfig::Jwt(jwt_config) => {
                let verifier = jwt_config
                    .get_verifier()
                    .expect("Failed to build JwtTokenVerifier");
                Some(AuthVerifier::jwt_verifier(verifier))
            }
            TokenVerifierAuthConfig::None => None,
        }
    }

    /// Create a ControlPlane service instance from this configuration
    pub fn into_service(
        &self,
        id: ID,
        group_name: Option<String>,
        rx_drain: drain::Watch,
        message_processor: Arc<MessageProcessor>,
        servers: &[ServerConfig],
    ) -> ControlPlane {
        let auth_provider = self.get_token_provider_auth();
        let auth_verifier = self.get_token_verifier_auth();

        ControlPlane::new(ControlPlaneSettings {
            id,
            group_name,
            servers: self.servers.clone(),
            clients: self.clients.clone(),
            drain_rx: rx_drain,
            message_processor,
            pubsub_servers: servers.to_vec(),
            auth_provider,
            auth_verifier,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::auth::jwt::Config as JwtConfig;
    use slim_config::auth::static_jwt::Config as StaticJwtConfig;
    use slim_config::component::id::{ID, Kind};
    use slim_config::grpc::client::ClientConfig;
    use slim_config::grpc::server::ServerConfig;
    use slim_datapath::message_processing::MessageProcessor;
    use slim_testing::utils::TEST_VALID_SECRET;
    use std::sync::Arc;

    fn create_test_server_config() -> ServerConfig {
        ServerConfig::with_endpoint("127.0.0.1:50051")
            .with_tls_settings(slim_config::tls::server::TlsServerConfig::insecure())
    }

    fn create_test_client_config() -> ClientConfig {
        ClientConfig::with_endpoint("http://127.0.0.1:50051")
            .with_tls_setting(slim_config::tls::client::TlsClientConfig::insecure())
    }

    #[test]
    fn test_config_new() {
        let config = Config::new();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
        assert_eq!(config.token_provider, TokenProviderAuthConfig::None);
        assert_eq!(config.token_verifier, TokenVerifierAuthConfig::None);
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.servers.is_empty());
        assert!(config.clients.is_empty());
        assert_eq!(config.token_provider, TokenProviderAuthConfig::None);
        assert_eq!(config.token_verifier, TokenVerifierAuthConfig::None);
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
    fn test_config_with_token_provider_auth_shared_secret() {
        let auth = TokenProviderAuthConfig::SharedSecret {
            data: "test-secret".to_string(),
        };
        let config = Config::new().with_token_provider_auth(auth.clone());

        assert_eq!(config.token_provider, auth);
        assert_eq!(config.token_verifier, TokenVerifierAuthConfig::None);
    }

    #[test]
    fn test_config_with_token_provider_auth_static_jwt() {
        let static_jwt_config = StaticJwtConfig::with_file("test-key".to_string());
        let auth = TokenProviderAuthConfig::StaticJwt(static_jwt_config);
        let config = Config::new().with_token_provider_auth(auth.clone());

        assert_eq!(config.token_provider, auth);
    }

    #[test]
    fn test_config_with_token_provider_auth_jwt() {
        use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
        use slim_config::auth::jwt::{Claims, JwtKey};
        use std::time::Duration;

        let claims = Claims::default();
        let duration = Duration::from_secs(3600);
        let key = JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("test-secret".to_string()),
        });
        let jwt_config = JwtConfig::new(claims, duration, key);
        let auth = TokenProviderAuthConfig::Jwt(jwt_config);
        let config = Config::new().with_token_provider_auth(auth.clone());

        assert_eq!(config.token_provider, auth);
    }

    #[test]
    fn test_config_with_token_verifier_auth_shared_secret() {
        let auth = TokenVerifierAuthConfig::SharedSecret {
            data: "test-secret".to_string(),
        };
        let config = Config::new().with_token_verifier_auth(auth.clone());

        assert_eq!(config.token_verifier, auth);
        assert_eq!(config.token_provider, TokenProviderAuthConfig::None);
    }

    #[test]
    fn test_config_with_token_verifier_auth_jwt() {
        use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};
        use slim_config::auth::jwt::{Claims, JwtKey};
        use std::time::Duration;

        let claims = Claims::default();
        let duration = Duration::from_secs(3600);
        let key = JwtKey::Decoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("test-secret".to_string()),
        });
        let jwt_config = JwtConfig::new(claims, duration, key);
        let auth = TokenVerifierAuthConfig::Jwt(jwt_config);
        let config = Config::new().with_token_verifier_auth(auth.clone());

        assert_eq!(config.token_verifier, auth);
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
        let provider_auth = TokenProviderAuthConfig::SharedSecret {
            data: "provider-secret".to_string(),
        };
        let verifier_auth = TokenVerifierAuthConfig::SharedSecret {
            data: "verifier-secret".to_string(),
        };

        let config = Config::new()
            .with_servers(vec![server_config.clone()])
            .with_clients(vec![client_config.clone()])
            .with_token_provider_auth(provider_auth.clone())
            .with_token_verifier_auth(verifier_auth.clone());

        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.clients.len(), 1);
        assert_eq!(config.token_provider, provider_auth);
        assert_eq!(config.token_verifier, verifier_auth);
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
    fn test_token_provider_auth_config_equality() {
        let secret1 = TokenProviderAuthConfig::SharedSecret {
            data: "secret0".to_string(),
        };
        let secret2 = TokenProviderAuthConfig::SharedSecret {
            data: "secret0".to_string(),
        };
        let secret3 = TokenProviderAuthConfig::SharedSecret {
            data: "secret2".to_string(),
        };

        assert_eq!(secret1, secret2);
        assert_ne!(secret1, secret3);
    }

    #[test]
    fn test_token_verifier_auth_config_equality() {
        let secret1 = TokenVerifierAuthConfig::SharedSecret {
            data: "secret0".to_string(),
        };
        let secret2 = TokenVerifierAuthConfig::SharedSecret {
            data: "secret0".to_string(),
        };
        let secret3 = TokenVerifierAuthConfig::SharedSecret {
            data: "secret2".to_string(),
        };

        assert_eq!(secret1, secret2);
        assert_ne!(secret1, secret3);
    }

    #[test]
    fn test_config_clone() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();
        let auth = TokenProviderAuthConfig::SharedSecret {
            data: "secret0".to_string(),
        };

        let config1 = Config::new()
            .with_servers(vec![server_config])
            .with_clients(vec![client_config])
            .with_token_provider_auth(auth);

        let config2 = config1.clone();

        assert_eq!(config1.servers, config2.servers);
        assert_eq!(config1.clients, config2.clients);
        assert_eq!(config1.token_provider, config2.token_provider);
        assert_eq!(config1.token_verifier, config2.token_verifier);
    }

    #[tokio::test]
    async fn test_config_into_service() {
        let server_config = create_test_server_config();
        let client_config = create_test_client_config();
        let auth = TokenProviderAuthConfig::SharedSecret {
            data: TEST_VALID_SECRET.to_string(),
        };

        let config = Config::new()
            .with_servers(vec![server_config.clone()])
            .with_clients(vec![client_config])
            .with_token_provider_auth(auth);

        let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-instance").unwrap();
        let group_name = Some("test-group".to_string());
        let (_, rx_drain) = drain::channel();
        let message_processor = Arc::new(MessageProcessor::with_drain_channel(rx_drain.clone()));
        let servers = vec![server_config];

        let _control_plane =
            config.into_service(id, group_name, rx_drain, message_processor, &servers);
    }

    #[test]
    fn test_config_debug_trait() {
        let config = Config::new();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("Config"));
        assert!(debug_str.contains("servers"));
        assert!(debug_str.contains("clients"));
    }

    mod serde_tests {
        use super::*;
        use serde_json;

        #[test]
        fn test_token_provider_auth_config_serialize_shared_secret() {
            let auth = TokenProviderAuthConfig::SharedSecret {
                data: "test-secret".to_string(),
            };
            let json = serde_json::to_string(&auth).unwrap();
            println!("Serialized JSON: {}", json);
            assert!(json.contains("shared_secret"));
            assert!(json.contains("test-secret"));
        }

        #[test]
        fn test_token_provider_auth_config_deserialize_shared_secret() {
            let json = r#"{"type": "shared_secret", "data": "test-secret"}"#;
            let auth: TokenProviderAuthConfig = serde_json::from_str(json).unwrap();

            match auth {
                TokenProviderAuthConfig::SharedSecret { data } => {
                    assert_eq!(data, "test-secret");
                }
                _ => panic!("Expected SharedSecret variant"),
            }
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
        fn test_config_with_all_auth_combinations() {
            let provider_auth = TokenProviderAuthConfig::SharedSecret {
                data: "provider-secret".to_string(),
            };
            let verifier_auth = TokenVerifierAuthConfig::SharedSecret {
                data: "verifier-secret".to_string(),
            };

            let config = Config::new()
                .with_token_provider_auth(provider_auth.clone())
                .with_token_verifier_auth(verifier_auth.clone());

            assert_eq!(config.token_provider, provider_auth);
            assert_eq!(config.token_verifier, verifier_auth);
        }

        #[test]
        fn test_empty_servers_slice() {
            let config = Config::new();
            let servers = config.servers();
            assert!(servers.is_empty());
            assert_eq!(servers.len(), 0);
        }

        #[test]
        fn test_empty_clients_slice() {
            let config = Config::new();
            let clients = config.clients();
            assert!(clients.is_empty());
            assert_eq!(clients.len(), 0);
        }

        #[test]
        fn test_config_partial_eq() {
            let config1 = Config::new();
            let config2 = Config::new();

            // Default configs should be equal
            assert_eq!(config1, config2);

            // Add server to one config
            let server_config = create_test_server_config();
            let config3 = config1.clone().with_servers(vec![server_config]);

            // Should not be equal anymore
            assert_ne!(config1, config3);
        }

        #[test]
        fn test_mixed_auth_types() {
            use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};

            let static_jwt =
                TokenProviderAuthConfig::StaticJwt(StaticJwtConfig::with_file("test-token.jwt"));

            let jwt = TokenVerifierAuthConfig::Jwt(JwtConfig::new(
                slim_config::auth::jwt::Claims::default(),
                std::time::Duration::from_secs(3600),
                slim_config::auth::jwt::JwtKey::Decoding(Key {
                    algorithm: Algorithm::HS256,
                    format: KeyFormat::Pem,
                    key: KeyData::Data("test-key".to_string()),
                }),
            ));

            let config = Config::new()
                .with_token_provider_auth(static_jwt.clone())
                .with_token_verifier_auth(jwt.clone());

            assert_eq!(config.token_provider, static_jwt);
            assert_eq!(config.token_verifier, jwt);
        }

        mod edge_case_tests {
            use super::*;

            #[test]
            fn test_config_builder_pattern_reuse() {
                let base_config = Config::new();

                let config1 = base_config
                    .clone()
                    .with_servers(vec![create_test_server_config()]);
                let config2 = base_config
                    .clone()
                    .with_clients(vec![create_test_client_config()]);

                // Base config should still be empty
                assert!(base_config.servers.is_empty());
                assert!(base_config.clients.is_empty());

                // Derived configs should have their respective additions
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
                    .with_servers(vec![server2.clone()]); // This should overwrite, not append

                assert_eq!(config.servers.len(), 1);
                assert_eq!(config.servers[0], server2);
            }

            #[test]
            fn test_auth_config_none_variants() {
                let config = Config::new();

                assert_eq!(config.token_provider, TokenProviderAuthConfig::None);
                assert_eq!(config.token_verifier, TokenVerifierAuthConfig::None);

                // Adding one shouldn't affect the other
                let config_with_provider = config.clone().with_token_provider_auth(
                    TokenProviderAuthConfig::SharedSecret {
                        data: "secret".to_string(),
                    },
                );

                assert_ne!(
                    config_with_provider.token_provider,
                    TokenProviderAuthConfig::None
                );
                assert_eq!(
                    config_with_provider.token_verifier,
                    TokenVerifierAuthConfig::None
                );
            }
        }

        #[test]
        fn test_token_verifier_auth_config_serialize_shared_secret() {
            let auth = TokenVerifierAuthConfig::SharedSecret {
                data: "test-secret".to_string(),
            };
            let json = serde_json::to_string(&auth).unwrap();
            assert!(json.contains("shared_secret"));
            assert!(json.contains("test-secret"));
        }

        #[test]
        fn test_token_verifier_auth_config_deserialize_shared_secret() {
            let json = r#"{"type": "shared_secret", "data": "test-secret"}"#;
            let auth: TokenVerifierAuthConfig = serde_json::from_str(json).unwrap();

            match auth {
                TokenVerifierAuthConfig::SharedSecret { data: secret } => {
                    assert_eq!(secret, "test-secret");
                }
                _ => panic!("Expected SharedSecret variant"),
            }
        }
    }
}

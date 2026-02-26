// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::tls::client::TlsClientConfig as CoreTlsClientConfig;
use slim_config::tls::server::TlsServerConfig as CoreTlsServerConfig;

use crate::identity_config::{ClientJwtAuth, JwtAuth, StaticJwtAuth};

/// SPIRE configuration for SPIFFE Workload API integration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SpireConfig {
    /// Path to the SPIFFE Workload API socket (None => use SPIFFE_ENDPOINT_SOCKET env var)
    pub socket_path: Option<String>,
    /// Optional target SPIFFE ID when requesting JWT SVIDs
    pub target_spiffe_id: Option<String>,
    /// Audiences to request/verify for JWT SVIDs
    pub jwt_audiences: Vec<String>,
    /// Optional trust domains override for X.509 bundle retrieval
    pub trust_domains: Vec<String>,
}

impl Default for SpireConfig {
    fn default() -> Self {
        SpireConfig {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: vec!["slim".to_string()],
            trust_domains: vec![],
        }
    }
}

#[cfg(not(target_family = "windows"))]
impl From<SpireConfig> for slim_config::auth::spire::SpireConfig {
    fn from(config: SpireConfig) -> Self {
        slim_config::auth::spire::SpireConfig {
            socket_path: config.socket_path,
            target_spiffe_id: config.target_spiffe_id,
            jwt_audiences: config.jwt_audiences,
            trust_domains: config.trust_domains,
        }
    }
}

#[cfg(not(target_family = "windows"))]
impl From<slim_config::auth::spire::SpireConfig> for SpireConfig {
    fn from(config: slim_config::auth::spire::SpireConfig) -> Self {
        SpireConfig {
            socket_path: config.socket_path,
            target_spiffe_id: config.target_spiffe_id,
            jwt_audiences: config.jwt_audiences,
            trust_domains: config.trust_domains,
        }
    }
}

/// TLS certificate and key source configuration
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum TlsSource {
    /// Load certificate and key from PEM strings
    Pem { cert: String, key: String },
    /// Load certificate and key from files (with auto-reload support)
    File { cert: String, key: String },
    /// Load certificate and key from SPIRE Workload API
    Spire { config: SpireConfig },
    /// No certificate/key configured
    None,
}

impl From<TlsSource> for slim_config::tls::common::TlsSource {
    fn from(source: TlsSource) -> Self {
        match source {
            TlsSource::Pem { cert, key } => slim_config::tls::common::TlsSource::Pem { cert, key },
            TlsSource::File { cert, key } => {
                slim_config::tls::common::TlsSource::File { cert, key }
            }
            #[cfg(not(target_family = "windows"))]
            TlsSource::Spire { config } => slim_config::tls::common::TlsSource::Spire {
                config: config.into(),
            },
            #[cfg(target_family = "windows")]
            TlsSource::Spire { .. } => {
                panic!("SPIRE is not supported on Windows");
            }
            TlsSource::None => slim_config::tls::common::TlsSource::None,
        }
    }
}

impl From<slim_config::tls::common::TlsSource> for TlsSource {
    fn from(source: slim_config::tls::common::TlsSource) -> Self {
        match source {
            slim_config::tls::common::TlsSource::Pem { cert, key } => TlsSource::Pem { cert, key },
            slim_config::tls::common::TlsSource::File { cert, key } => {
                TlsSource::File { cert, key }
            }
            #[cfg(not(target_family = "windows"))]
            slim_config::tls::common::TlsSource::Spire { config } => TlsSource::Spire {
                config: config.into(),
            },
            slim_config::tls::common::TlsSource::None => TlsSource::None,
        }
    }
}

/// CA certificate source configuration
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum CaSource {
    /// Load CA from file
    File { path: String },
    /// Load CA from PEM string
    Pem { data: String },
    /// Load CA from SPIRE Workload API
    Spire { config: SpireConfig },
    /// No CA configured
    None,
}

impl From<CaSource> for slim_config::tls::common::CaSource {
    fn from(source: CaSource) -> Self {
        match source {
            CaSource::File { path } => slim_config::tls::common::CaSource::File { path },
            CaSource::Pem { data } => slim_config::tls::common::CaSource::Pem { data },
            #[cfg(not(target_family = "windows"))]
            CaSource::Spire { config } => slim_config::tls::common::CaSource::Spire {
                config: config.into(),
            },
            #[cfg(target_family = "windows")]
            CaSource::Spire { .. } => {
                panic!("SPIRE is not supported on Windows");
            }
            CaSource::None => slim_config::tls::common::CaSource::None,
        }
    }
}

impl From<slim_config::tls::common::CaSource> for CaSource {
    fn from(source: slim_config::tls::common::CaSource) -> Self {
        match source {
            slim_config::tls::common::CaSource::File { path } => CaSource::File { path },
            slim_config::tls::common::CaSource::Pem { data } => CaSource::Pem { data },
            #[cfg(not(target_family = "windows"))]
            slim_config::tls::common::CaSource::Spire { config } => CaSource::Spire {
                config: config.into(),
            },
            slim_config::tls::common::CaSource::None => CaSource::None,
        }
    }
}

/// TLS configuration for client connections
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct TlsClientConfig {
    /// Disable TLS entirely (plain text connection)
    pub insecure: bool,
    /// Skip server certificate verification (enables TLS but doesn't verify certs)
    /// WARNING: Only use for testing - insecure in production!
    pub insecure_skip_verify: bool,
    /// Certificate and key source for client authentication
    pub source: TlsSource,
    /// CA certificate source for verifying server certificates
    pub ca_source: CaSource,
    /// Include system CA certificates pool (default: true)
    pub include_system_ca_certs_pool: bool,
    /// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
    pub tls_version: String,
}

impl Default for TlsClientConfig {
    fn default() -> Self {
        let core_defaults = CoreTlsClientConfig::default();
        TlsClientConfig {
            insecure: core_defaults.insecure,
            insecure_skip_verify: core_defaults.insecure_skip_verify,
            source: core_defaults.config.source.into(),
            ca_source: core_defaults.config.ca_source.into(),
            include_system_ca_certs_pool: core_defaults.config.include_system_ca_certs_pool,
            tls_version: core_defaults.config.tls_version,
        }
    }
}

impl From<TlsClientConfig> for CoreTlsClientConfig {
    fn from(config: TlsClientConfig) -> Self {
        CoreTlsClientConfig {
            config: slim_config::tls::common::Config {
                source: config.source.into(),
                ca_source: config.ca_source.into(),
                include_system_ca_certs_pool: config.include_system_ca_certs_pool,
                tls_version: config.tls_version,
                reload_interval: None,
            },
            insecure: config.insecure,
            insecure_skip_verify: config.insecure_skip_verify,
        }
    }
}

impl From<CoreTlsClientConfig> for TlsClientConfig {
    fn from(config: CoreTlsClientConfig) -> Self {
        TlsClientConfig {
            insecure: config.insecure,
            insecure_skip_verify: config.insecure_skip_verify,
            source: config.config.source.into(),
            ca_source: config.config.ca_source.into(),
            include_system_ca_certs_pool: config.config.include_system_ca_certs_pool,
            tls_version: config.config.tls_version,
        }
    }
}

/// TLS configuration for server connections
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct TlsServerConfig {
    /// Disable TLS entirely (plain text connection)
    pub insecure: bool,
    /// Certificate and key source for server authentication
    pub source: TlsSource,
    /// CA certificate source for verifying client certificates
    pub client_ca: CaSource,
    /// Include system CA certificates pool (default: true)
    #[uniffi(default = None)]
    pub include_system_ca_certs_pool: Option<bool>,
    /// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
    #[uniffi(default = None)]
    pub tls_version: Option<String>,
    /// Reload client CA file when modified
    #[uniffi(default = None)]
    pub reload_client_ca_file: Option<bool>,
}

impl Default for TlsServerConfig {
    fn default() -> Self {
        TlsServerConfig {
            insecure: false,
            source: TlsSource::None,
            client_ca: CaSource::None,
            include_system_ca_certs_pool: None,
            tls_version: None,
            reload_client_ca_file: None,
        }
    }
}

impl From<TlsServerConfig> for CoreTlsServerConfig {
    fn from(config: TlsServerConfig) -> Self {
        let core_defaults = CoreTlsServerConfig::default();
        CoreTlsServerConfig {
            config: slim_config::tls::common::Config {
                source: config.source.into(),
                ca_source: slim_config::tls::common::CaSource::None,
                include_system_ca_certs_pool: config
                    .include_system_ca_certs_pool
                    .unwrap_or(core_defaults.config.include_system_ca_certs_pool),
                tls_version: config
                    .tls_version
                    .unwrap_or(core_defaults.config.tls_version),
                reload_interval: None,
            },
            insecure: config.insecure,
            client_ca: config.client_ca.into(),
            reload_client_ca_file: config
                .reload_client_ca_file
                .unwrap_or(core_defaults.reload_client_ca_file),
        }
    }
}

impl From<CoreTlsServerConfig> for TlsServerConfig {
    fn from(config: CoreTlsServerConfig) -> Self {
        TlsServerConfig {
            insecure: config.insecure,
            source: config.config.source.into(),
            client_ca: config.client_ca.into(),
            include_system_ca_certs_pool: Some(config.config.include_system_ca_certs_pool),
            tls_version: Some(config.config.tls_version),
            reload_client_ca_file: Some(config.reload_client_ca_file),
        }
    }
}

/// Basic authentication configuration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

impl From<BasicAuth> for BasicAuthConfig {
    fn from(config: BasicAuth) -> Self {
        BasicAuthConfig::new(&config.username, &config.password)
    }
}

impl From<BasicAuthConfig> for BasicAuth {
    fn from(config: BasicAuthConfig) -> Self {
        BasicAuth {
            username: config.username().to_string(),
            password: config.password().as_str().to_string(),
        }
    }
}

/// Authentication configuration enum for client
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ClientAuthenticationConfig {
    Basic { config: BasicAuth },
    StaticJwt { config: StaticJwtAuth },
    Jwt { config: ClientJwtAuth },
    None,
}

impl From<ClientAuthenticationConfig> for slim_config::grpc::client::AuthenticationConfig {
    fn from(config: ClientAuthenticationConfig) -> Self {
        match config {
            ClientAuthenticationConfig::Basic { config } => {
                slim_config::grpc::client::AuthenticationConfig::Basic(config.into())
            }
            ClientAuthenticationConfig::StaticJwt { config } => {
                slim_config::grpc::client::AuthenticationConfig::StaticJwt(config.into())
            }
            ClientAuthenticationConfig::Jwt { config } => {
                slim_config::grpc::client::AuthenticationConfig::Jwt(config.into())
            }
            ClientAuthenticationConfig::None => {
                slim_config::grpc::client::AuthenticationConfig::None
            }
        }
    }
}

impl From<slim_config::grpc::client::AuthenticationConfig> for ClientAuthenticationConfig {
    fn from(config: slim_config::grpc::client::AuthenticationConfig) -> Self {
        match config {
            slim_config::grpc::client::AuthenticationConfig::None => {
                ClientAuthenticationConfig::None
            }
            slim_config::grpc::client::AuthenticationConfig::Basic(basic) => {
                ClientAuthenticationConfig::Basic {
                    config: basic.into(),
                }
            }
            slim_config::grpc::client::AuthenticationConfig::StaticJwt(jwt) => {
                ClientAuthenticationConfig::StaticJwt { config: jwt.into() }
            }
            slim_config::grpc::client::AuthenticationConfig::Jwt(jwt) => {
                ClientAuthenticationConfig::Jwt { config: jwt.into() }
            }
        }
    }
}

/// Authentication configuration enum for server
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ServerAuthenticationConfig {
    Basic { config: BasicAuth },
    Jwt { config: JwtAuth },
    None,
}

impl From<ServerAuthenticationConfig> for slim_config::grpc::server::AuthenticationConfig {
    fn from(config: ServerAuthenticationConfig) -> Self {
        match config {
            ServerAuthenticationConfig::Basic { config } => {
                slim_config::grpc::server::AuthenticationConfig::Basic(config.into())
            }
            ServerAuthenticationConfig::Jwt { config } => {
                slim_config::grpc::server::AuthenticationConfig::Jwt(config.into())
            }
            ServerAuthenticationConfig::None => {
                slim_config::grpc::server::AuthenticationConfig::None
            }
        }
    }
}

impl From<slim_config::grpc::server::AuthenticationConfig> for ServerAuthenticationConfig {
    fn from(config: slim_config::grpc::server::AuthenticationConfig) -> Self {
        match config {
            slim_config::grpc::server::AuthenticationConfig::None => {
                ServerAuthenticationConfig::None
            }
            slim_config::grpc::server::AuthenticationConfig::Basic(basic) => {
                ServerAuthenticationConfig::Basic {
                    config: basic.into(),
                }
            }
            slim_config::grpc::server::AuthenticationConfig::Jwt(jwt) => {
                ServerAuthenticationConfig::Jwt { config: jwt.into() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_config::{
        JwtAlgorithm, JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType,
    };
    use std::time::Duration;

    // Test SpireConfig default
    #[test]
    fn test_spire_config_default() {
        let config = SpireConfig::default();
        assert_eq!(config.socket_path, None);
        assert_eq!(config.target_spiffe_id, None);
        assert_eq!(config.jwt_audiences, vec!["slim".to_string()]);
        assert_eq!(config.trust_domains, Vec::<String>::new());
    }

    // Test SpireConfig conversions (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_spire_config_conversions() {
        let config = SpireConfig {
            socket_path: Some("/var/run/spire/socket".to_string()),
            target_spiffe_id: Some("spiffe://example.com/service".to_string()),
            jwt_audiences: vec!["audience1".to_string(), "audience2".to_string()],
            trust_domains: vec!["example.com".to_string()],
        };

        let core_config: slim_config::auth::spire::SpireConfig = config.clone().into();
        assert_eq!(core_config.socket_path, config.socket_path);
        assert_eq!(core_config.target_spiffe_id, config.target_spiffe_id);
        assert_eq!(core_config.jwt_audiences, config.jwt_audiences);
        assert_eq!(core_config.trust_domains, config.trust_domains);

        let back: SpireConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test TlsSource conversions - Pem variant
    #[test]
    fn test_tls_source_pem_conversion() {
        let source = TlsSource::Pem {
            cert: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----".to_string(),
            key: "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----".to_string(),
        };

        let core_source: slim_config::tls::common::TlsSource = source.clone().into();
        let back: TlsSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test TlsSource conversions - File variant
    #[test]
    fn test_tls_source_file_conversion() {
        let source = TlsSource::File {
            cert: "/path/to/cert.pem".to_string(),
            key: "/path/to/key.pem".to_string(),
        };

        let core_source: slim_config::tls::common::TlsSource = source.clone().into();
        let back: TlsSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test TlsSource conversions - None variant
    #[test]
    fn test_tls_source_none_conversion() {
        let source = TlsSource::None;
        let core_source: slim_config::tls::common::TlsSource = source.clone().into();
        let back: TlsSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test TlsSource conversions - Spire variant (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_tls_source_spire_conversion() {
        let source = TlsSource::Spire {
            config: SpireConfig::default(),
        };

        let core_source: slim_config::tls::common::TlsSource = source.clone().into();
        let back: TlsSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test CaSource conversions - File variant
    #[test]
    fn test_ca_source_file_conversion() {
        let source = CaSource::File {
            path: "/path/to/ca.pem".to_string(),
        };

        let core_source: slim_config::tls::common::CaSource = source.clone().into();
        let back: CaSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test CaSource conversions - Pem variant
    #[test]
    fn test_ca_source_pem_conversion() {
        let source = CaSource::Pem {
            data: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----".to_string(),
        };

        let core_source: slim_config::tls::common::CaSource = source.clone().into();
        let back: CaSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test CaSource conversions - None variant
    #[test]
    fn test_ca_source_none_conversion() {
        let source = CaSource::None;
        let core_source: slim_config::tls::common::CaSource = source.clone().into();
        let back: CaSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test CaSource conversions - Spire variant (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_ca_source_spire_conversion() {
        let source = CaSource::Spire {
            config: SpireConfig::default(),
        };

        let core_source: slim_config::tls::common::CaSource = source.clone().into();
        let back: CaSource = core_source.into();
        assert_eq!(back, source);
    }

    // Test TlsClientConfig default
    #[test]
    fn test_tls_client_config_default() {
        let config = TlsClientConfig::default();
        assert!(!config.insecure);
        assert!(!config.insecure_skip_verify);
        assert_eq!(config.source, TlsSource::None);
        assert_eq!(config.ca_source, CaSource::None);
        assert!(config.include_system_ca_certs_pool);
        assert_eq!(config.tls_version, "tls1.3");
    }

    // Test TlsClientConfig conversions
    #[test]
    fn test_tls_client_config_conversion() {
        let config = TlsClientConfig {
            insecure: false,
            insecure_skip_verify: false,
            source: TlsSource::File {
                cert: "/path/to/cert.pem".to_string(),
                key: "/path/to/key.pem".to_string(),
            },
            ca_source: CaSource::File {
                path: "/path/to/ca.pem".to_string(),
            },
            include_system_ca_certs_pool: true,
            tls_version: "tls1.3".to_string(),
        };

        let core_config: CoreTlsClientConfig = config.clone().into();
        assert_eq!(core_config.insecure, config.insecure);
        assert_eq!(
            core_config.insecure_skip_verify,
            config.insecure_skip_verify
        );
        assert_eq!(
            core_config.config.include_system_ca_certs_pool,
            config.include_system_ca_certs_pool
        );
        assert_eq!(core_config.config.tls_version, config.tls_version);

        let back: TlsClientConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test TlsClientConfig with insecure options
    #[test]
    fn test_tls_client_config_insecure() {
        let config = TlsClientConfig {
            insecure: true,
            insecure_skip_verify: true,
            source: TlsSource::None,
            ca_source: CaSource::None,
            include_system_ca_certs_pool: false,
            tls_version: "tls1.2".to_string(),
        };

        let core_config: CoreTlsClientConfig = config.clone().into();
        let back: TlsClientConfig = core_config.into();
        assert!(back.insecure);
        assert!(back.insecure_skip_verify);
        assert!(!back.include_system_ca_certs_pool);
        assert_eq!(back.tls_version, "tls1.2");
    }

    // Test TlsServerConfig default
    #[test]
    fn test_tls_server_config_default() {
        let config = TlsServerConfig::default();
        assert!(!config.insecure);
        assert_eq!(config.source, TlsSource::None);
        assert_eq!(config.client_ca, CaSource::None);
        assert_eq!(config.include_system_ca_certs_pool, None);
        assert_eq!(config.tls_version, None);
        assert_eq!(config.reload_client_ca_file, None);

        // Verify core defaults are applied when converting
        let core: CoreTlsServerConfig = config.into();
        assert!(core.config.include_system_ca_certs_pool);
        assert_eq!(core.config.tls_version, "tls1.3");
        assert!(!core.reload_client_ca_file);
    }

    // Test TlsServerConfig conversions
    #[test]
    fn test_tls_server_config_conversion() {
        let config = TlsServerConfig {
            insecure: false,
            source: TlsSource::Pem {
                cert: "cert-data".to_string(),
                key: "key-data".to_string(),
            },
            client_ca: CaSource::Pem {
                data: "ca-data".to_string(),
            },
            include_system_ca_certs_pool: Some(true),
            tls_version: Some("tls1.3".to_string()),
            reload_client_ca_file: Some(true),
        };

        let core_config: CoreTlsServerConfig = config.clone().into();
        assert_eq!(core_config.insecure, config.insecure);
        assert_eq!(
            core_config.reload_client_ca_file,
            config.reload_client_ca_file.unwrap()
        );
        assert_eq!(
            core_config.config.include_system_ca_certs_pool,
            config.include_system_ca_certs_pool.unwrap()
        );
        assert_eq!(
            core_config.config.tls_version,
            config.tls_version.clone().unwrap()
        );

        let back: TlsServerConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test BasicAuth conversions
    #[test]
    fn test_basic_auth_conversion() {
        let auth = BasicAuth {
            username: "testuser".to_string(),
            password: "testpassword".to_string(),
        };

        let core_auth: BasicAuthConfig = auth.clone().into();
        assert_eq!(core_auth.username(), "testuser");
        assert_eq!(core_auth.password().as_str(), "testpassword");

        let back: BasicAuth = core_auth.into();
        assert_eq!(back, auth);
    }

    // Test ClientAuthenticationConfig - Basic variant
    #[test]
    fn test_client_auth_config_basic() {
        let config = ClientAuthenticationConfig::Basic {
            config: BasicAuth {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
        };

        let core_config: slim_config::grpc::client::AuthenticationConfig = config.clone().into();
        let back: ClientAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ClientAuthenticationConfig - StaticJwt variant
    #[test]
    fn test_client_auth_config_static_jwt() {
        let config = ClientAuthenticationConfig::StaticJwt {
            config: StaticJwtAuth {
                token_file: "/path/to/token.jwt".to_string(),
                duration: Duration::from_secs(3600),
            },
        };

        let core_config: slim_config::grpc::client::AuthenticationConfig = config.clone().into();
        let back: ClientAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ClientAuthenticationConfig - Jwt variant
    #[test]
    fn test_client_auth_config_jwt() {
        let config = ClientAuthenticationConfig::Jwt {
            config: ClientJwtAuth {
                key: JwtKeyType::Autoresolve,
                audience: Some(vec!["api".to_string()]),
                issuer: None,
                subject: None,
                duration: Duration::from_secs(3600),
            },
        };

        let core_config: slim_config::grpc::client::AuthenticationConfig = config.clone().into();
        let back: ClientAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ClientAuthenticationConfig - None variant
    #[test]
    fn test_client_auth_config_none() {
        let config = ClientAuthenticationConfig::None;
        let core_config: slim_config::grpc::client::AuthenticationConfig = config.clone().into();
        let back: ClientAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ServerAuthenticationConfig - Basic variant
    #[test]
    fn test_server_auth_config_basic() {
        let config = ServerAuthenticationConfig::Basic {
            config: BasicAuth {
                username: "admin".to_string(),
                password: "secret".to_string(),
            },
        };

        let core_config: slim_config::grpc::server::AuthenticationConfig = config.clone().into();
        let back: ServerAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ServerAuthenticationConfig - Jwt variant
    #[test]
    fn test_server_auth_config_jwt() {
        let config = ServerAuthenticationConfig::Jwt {
            config: JwtAuth {
                key: JwtKeyType::Decoding {
                    key: JwtKeyConfig {
                        algorithm: JwtAlgorithm::RS256,
                        format: JwtKeyFormat::Pem,
                        key: JwtKeyData::File {
                            path: "/path/to/key.pem".to_string(),
                        },
                    },
                },
                audience: Some(vec!["service".to_string()]),
                issuer: Some("issuer".to_string()),
                subject: None,
                duration: Duration::from_secs(7200),
            },
        };

        let core_config: slim_config::grpc::server::AuthenticationConfig = config.clone().into();
        let back: ServerAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test ServerAuthenticationConfig - None variant
    #[test]
    fn test_server_auth_config_none() {
        let config = ServerAuthenticationConfig::None;
        let core_config: slim_config::grpc::server::AuthenticationConfig = config.clone().into();
        let back: ServerAuthenticationConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test complex TlsClientConfig with all options
    #[test]
    fn test_complex_tls_client_config() {
        let config = TlsClientConfig {
            insecure: false,
            insecure_skip_verify: false,
            source: TlsSource::Pem {
                cert: "-----BEGIN CERTIFICATE-----\ncert\n-----END CERTIFICATE-----".to_string(),
                key: "-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----".to_string(),
            },
            ca_source: CaSource::Pem {
                data: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----".to_string(),
            },
            include_system_ca_certs_pool: false,
            tls_version: "tls1.2".to_string(),
        };

        let core_config: CoreTlsClientConfig = config.clone().into();
        let back: TlsClientConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test complex TlsServerConfig with all options
    #[test]
    fn test_complex_tls_server_config() {
        let config = TlsServerConfig {
            insecure: false,
            source: TlsSource::File {
                cert: "/etc/tls/server.crt".to_string(),
                key: "/etc/tls/server.key".to_string(),
            },
            client_ca: CaSource::File {
                path: "/etc/tls/ca.crt".to_string(),
            },
            include_system_ca_certs_pool: Some(false),
            tls_version: Some("tls1.2".to_string()),
            reload_client_ca_file: Some(true),
        };

        let core_config: CoreTlsServerConfig = config.clone().into();
        let back: TlsServerConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test all TlsSource variants
    #[test]
    fn test_all_tls_source_variants() {
        let variants = vec![
            TlsSource::Pem {
                cert: "cert1".to_string(),
                key: "key1".to_string(),
            },
            TlsSource::File {
                cert: "/path1".to_string(),
                key: "/path2".to_string(),
            },
            TlsSource::None,
        ];

        for variant in variants {
            let core: slim_config::tls::common::TlsSource = variant.clone().into();
            let back: TlsSource = core.into();
            assert_eq!(back, variant);
        }
    }

    // Test all CaSource variants
    #[test]
    fn test_all_ca_source_variants() {
        let variants = vec![
            CaSource::File {
                path: "/ca/path".to_string(),
            },
            CaSource::Pem {
                data: "ca-pem-data".to_string(),
            },
            CaSource::None,
        ];

        for variant in variants {
            let core: slim_config::tls::common::CaSource = variant.clone().into();
            let back: CaSource = core.into();
            assert_eq!(back, variant);
        }
    }

    // Test SpireConfig with minimal configuration
    #[test]
    fn test_spire_config_minimal() {
        let config = SpireConfig {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: vec![],
            trust_domains: vec![],
        };

        assert_eq!(config.socket_path, None);
        assert_eq!(config.target_spiffe_id, None);
        assert!(config.jwt_audiences.is_empty());
        assert!(config.trust_domains.is_empty());
    }

    // Test SpireConfig with full configuration
    #[test]
    fn test_spire_config_full() {
        let config = SpireConfig {
            socket_path: Some("/var/run/spire.sock".to_string()),
            target_spiffe_id: Some("spiffe://example.com/workload".to_string()),
            jwt_audiences: vec!["aud1".to_string(), "aud2".to_string(), "aud3".to_string()],
            trust_domains: vec!["domain1.com".to_string(), "domain2.com".to_string()],
        };

        assert_eq!(config.socket_path, Some("/var/run/spire.sock".to_string()));
        assert_eq!(
            config.target_spiffe_id,
            Some("spiffe://example.com/workload".to_string())
        );
        assert_eq!(config.jwt_audiences.len(), 3);
        assert_eq!(config.trust_domains.len(), 2);
    }
}

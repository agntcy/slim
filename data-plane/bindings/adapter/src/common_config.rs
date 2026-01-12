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
    pub include_system_ca_certs_pool: bool,
    /// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
    pub tls_version: String,
    /// Reload client CA file when modified
    pub reload_client_ca_file: bool,
}

impl Default for TlsServerConfig {
    fn default() -> Self {
        let core_defaults = CoreTlsServerConfig::default();
        TlsServerConfig {
            insecure: core_defaults.insecure,
            source: core_defaults.config.source.into(),
            client_ca: core_defaults.client_ca.into(),
            include_system_ca_certs_pool: core_defaults.config.include_system_ca_certs_pool,
            tls_version: core_defaults.config.tls_version,
            reload_client_ca_file: core_defaults.reload_client_ca_file,
        }
    }
}

impl From<TlsServerConfig> for CoreTlsServerConfig {
    fn from(config: TlsServerConfig) -> Self {
        CoreTlsServerConfig {
            config: slim_config::tls::common::Config {
                source: config.source.into(),
                ca_source: slim_config::tls::common::CaSource::None,
                include_system_ca_certs_pool: config.include_system_ca_certs_pool,
                tls_version: config.tls_version,
                reload_interval: None,
            },
            insecure: config.insecure,
            client_ca: config.client_ca.into(),
            reload_client_ca_file: config.reload_client_ca_file,
        }
    }
}

impl From<CoreTlsServerConfig> for TlsServerConfig {
    fn from(config: CoreTlsServerConfig) -> Self {
        TlsServerConfig {
            insecure: config.insecure,
            source: config.config.source.into(),
            client_ca: config.client_ca.into(),
            include_system_ca_certs_pool: config.config.include_system_ca_certs_pool,
            tls_version: config.config.tls_version,
            reload_client_ca_file: config.reload_client_ca_file,
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

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::auth::jwt::Config as JwtAuthConfig;
use slim_config::auth::jwt::{Claims as JwtClaims, JwtKey};
use slim_config::auth::static_jwt::Config as StaticJwtConfig;
use slim_config::tls::client::TlsClientConfig as CoreTlsClientConfig;
use slim_config::tls::server::TlsServerConfig as CoreTlsServerConfig;
use std::time::Duration;

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
                config: slim_config::auth::spire::SpireConfig {
                    socket_path: config.socket_path,
                    target_spiffe_id: config.target_spiffe_id,
                    jwt_audiences: config.jwt_audiences,
                    trust_domains: config.trust_domains,
                },
            },
            #[cfg(target_family = "windows")]
            TlsSource::Spire { .. } => {
                panic!("SPIRE is not supported on Windows");
            }
            TlsSource::None => slim_config::tls::common::TlsSource::None,
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
                config: slim_config::auth::spire::SpireConfig {
                    socket_path: config.socket_path,
                    target_spiffe_id: config.target_spiffe_id,
                    jwt_audiences: config.jwt_audiences,
                    trust_domains: config.trust_domains,
                },
            },
            #[cfg(target_family = "windows")]
            CaSource::Spire { .. } => {
                panic!("SPIRE is not supported on Windows");
            }
            CaSource::None => slim_config::tls::common::CaSource::None,
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
            source: TlsSource::None, // Keep as None since we can't convert back from core
            ca_source: CaSource::None, // Keep as None since we can't convert back from core
            include_system_ca_certs_pool: core_defaults.config.include_system_ca_certs_pool,
            tls_version: core_defaults.config.tls_version,
        }
    }
}

impl From<TlsClientConfig> for CoreTlsClientConfig {
    fn from(config: TlsClientConfig) -> Self {
        let mut core_config = CoreTlsClientConfig {
            insecure: config.insecure,
            insecure_skip_verify: config.insecure_skip_verify,
            ..Default::default()
        };

        // Use From trait for conversions
        core_config.config.source = config.source.into();
        core_config.config.ca_source = config.ca_source.into();
        core_config.config.include_system_ca_certs_pool = config.include_system_ca_certs_pool;
        core_config.config.tls_version = config.tls_version;

        core_config
    }
}

impl From<CoreTlsClientConfig> for TlsClientConfig {
    fn from(config: CoreTlsClientConfig) -> Self {
        TlsClientConfig {
            insecure: config.insecure,
            insecure_skip_verify: config.insecure_skip_verify,
            source: TlsSource::None, // Can't convert back from core - use default
            ca_source: CaSource::None, // Can't convert back from core - use default
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
            source: TlsSource::None, // Keep as None since we can't convert back from core
            client_ca: CaSource::None, // Keep as None since we can't convert back from core
            include_system_ca_certs_pool: core_defaults.config.include_system_ca_certs_pool,
            tls_version: core_defaults.config.tls_version,
            reload_client_ca_file: core_defaults.reload_client_ca_file,
        }
    }
}

impl From<TlsServerConfig> for CoreTlsServerConfig {
    fn from(config: TlsServerConfig) -> Self {
        let mut core_config = CoreTlsServerConfig {
            insecure: config.insecure,
            reload_client_ca_file: config.reload_client_ca_file,
            ..Default::default()
        };

        // Use From trait for conversions
        core_config.config.source = config.source.into();
        core_config.client_ca = config.client_ca.into();
        core_config.config.include_system_ca_certs_pool = config.include_system_ca_certs_pool;
        core_config.config.tls_version = config.tls_version;

        core_config
    }
}

impl From<CoreTlsServerConfig> for TlsServerConfig {
    fn from(config: CoreTlsServerConfig) -> Self {
        TlsServerConfig {
            insecure: config.insecure,
            source: TlsSource::None, // Can't convert back from core - use default
            client_ca: CaSource::None, // Can't convert back from core - use default
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

/// Static JWT (Bearer token) authentication configuration
/// The token is loaded from a file and automatically reloaded when changed
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct StaticJwtAuth {
    /// Path to file containing the JWT token
    pub token_file: String,
    /// Duration in seconds for caching the token before re-reading from file (default: 3600)
    pub duration_secs: Option<u64>,
}

impl From<StaticJwtAuth> for StaticJwtConfig {
    fn from(config: StaticJwtAuth) -> Self {
        let mut jwt_config = StaticJwtConfig::with_file(&config.token_file);
        if let Some(duration) = config.duration_secs {
            jwt_config = jwt_config.with_duration(duration);
        }
        jwt_config
    }
}

/// JWT authentication configuration for server-side verification
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct JwtAuth {
    /// Path to file containing the decoding key (public key for verification)
    pub key_file: String,
    /// JWT audience claims to verify
    pub audience: Option<Vec<String>>,
    /// JWT issuer to verify
    pub issuer: Option<String>,
    /// JWT subject to verify
    pub subject: Option<String>,
    /// Token validity duration in seconds (default: 3600)
    pub duration_secs: Option<u64>,
}

impl From<JwtAuth> for JwtAuthConfig {
    fn from(config: JwtAuth) -> Self {
        let mut claims = JwtClaims::default();

        if let Some(audience) = config.audience {
            claims = claims.with_audience(&audience);
        }

        if let Some(issuer) = config.issuer {
            claims = claims.with_issuer(issuer);
        }

        if let Some(subject) = config.subject {
            claims = claims.with_subject(subject);
        }

        let duration = Duration::from_secs(config.duration_secs.unwrap_or(3600));

        // For server-side verification, we use a decoding key from file
        // Create the Key struct with proper fields
        let key = JwtKey::Decoding(slim_auth::jwt::Key {
            algorithm: slim_auth::jwt::Algorithm::RS256, // Default to RS256
            format: slim_auth::jwt::KeyFormat::Pem,
            key: slim_auth::jwt::KeyData::File(config.key_file),
        });

        JwtAuthConfig::new(claims, duration, key)
    }
}

/// Authentication configuration enum for client
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum ClientAuthenticationConfig {
    Basic { config: BasicAuth },
    StaticJwt { config: StaticJwtAuth },
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
            ClientAuthenticationConfig::None => {
                slim_config::grpc::client::AuthenticationConfig::None
            }
        }
    }
}

/// Helper function to convert Option<ClientAuthenticationConfig> to CoreAuthenticationConfig
pub fn convert_client_auth(
    config: Option<ClientAuthenticationConfig>,
) -> slim_config::grpc::client::AuthenticationConfig {
    match config {
        Some(auth) => auth.into(),
        None => slim_config::grpc::client::AuthenticationConfig::None,
    }
}

/// Helper function to convert CoreAuthenticationConfig to Option<ClientAuthenticationConfig>
pub fn convert_core_to_client_auth(
    config: slim_config::grpc::client::AuthenticationConfig,
) -> Option<ClientAuthenticationConfig> {
    match config {
        slim_config::grpc::client::AuthenticationConfig::None => None,
        // For Basic and StaticJwt auth types, we can't fully convert back since we'd lose
        // credential information (passwords, tokens). Return None to indicate no auth in FFI.
        _ => None,
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
            // For Basic and Jwt auth types, we can't fully convert back since we'd lose
            // credential information (passwords, tokens, keys). Return None variant.
            _ => ServerAuthenticationConfig::None,
        }
    }
}

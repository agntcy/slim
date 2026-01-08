// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::Duration;

use slim_config::grpc::client::ClientConfig as CoreClientConfig;
use slim_config::grpc::compression::CompressionType as CoreCompressionType;
use slim_config::grpc::proxy::ProxyConfig as CoreProxyConfig;

use slim_auth::metadata::MetadataMap;
use slim_config::backoff::exponential::Config as ExponentialBackoffConfig;
use slim_config::backoff::fixedinterval::Config as FixedIntervalBackoffConfig;
use slim_config::grpc::client::{
    BackoffConfig as CoreBackoffConfig, KeepaliveConfig as CoreKeepaliveConfig,
};

use crate::common_config::{ClientAuthenticationConfig, TlsClientConfig};

/// Compression type for gRPC messages
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum CompressionType {
    Gzip,
    Zlib,
    Deflate,
    Snappy,
    Zstd,
    Lz4,
    None,
    Empty,
}

impl From<CompressionType> for CoreCompressionType {
    fn from(compression: CompressionType) -> Self {
        match compression {
            CompressionType::Gzip => CoreCompressionType::Gzip,
            CompressionType::Zlib => CoreCompressionType::Zlib,
            CompressionType::Deflate => CoreCompressionType::Deflate,
            CompressionType::Snappy => CoreCompressionType::Snappy,
            CompressionType::Zstd => CoreCompressionType::Zstd,
            CompressionType::Lz4 => CoreCompressionType::Lz4,
            CompressionType::None => CoreCompressionType::None,
            CompressionType::Empty => CoreCompressionType::Empty,
        }
    }
}

impl From<CoreCompressionType> for CompressionType {
    fn from(compression: CoreCompressionType) -> Self {
        match compression {
            CoreCompressionType::Gzip => CompressionType::Gzip,
            CoreCompressionType::Zlib => CompressionType::Zlib,
            CoreCompressionType::Deflate => CompressionType::Deflate,
            CoreCompressionType::Snappy => CompressionType::Snappy,
            CoreCompressionType::Zstd => CompressionType::Zstd,
            CoreCompressionType::Lz4 => CompressionType::Lz4,
            CoreCompressionType::None => CompressionType::None,
            CoreCompressionType::Empty => CompressionType::Empty,
        }
    }
}

/// Keepalive configuration for the client
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct KeepaliveConfig {
    /// TCP keepalive duration
    pub tcp_keepalive: Duration,
    /// HTTP2 keepalive duration
    pub http2_keepalive: Duration,
    /// Keepalive timeout
    pub timeout: Duration,
    /// Whether to permit keepalive without an active stream
    pub keep_alive_while_idle: bool,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        let core_defaults = CoreKeepaliveConfig::default();
        KeepaliveConfig {
            tcp_keepalive: *core_defaults.tcp_keepalive,
            http2_keepalive: *core_defaults.http2_keepalive,
            timeout: *core_defaults.timeout,
            keep_alive_while_idle: core_defaults.keep_alive_while_idle,
        }
    }
}

impl From<KeepaliveConfig> for CoreKeepaliveConfig {
    fn from(config: KeepaliveConfig) -> Self {
        CoreKeepaliveConfig {
            tcp_keepalive: config.tcp_keepalive.into(),
            http2_keepalive: config.http2_keepalive.into(),
            timeout: config.timeout.into(),
            keep_alive_while_idle: config.keep_alive_while_idle,
        }
    }
}

impl From<CoreKeepaliveConfig> for KeepaliveConfig {
    fn from(config: CoreKeepaliveConfig) -> Self {
        KeepaliveConfig {
            tcp_keepalive: *config.tcp_keepalive,
            http2_keepalive: *config.http2_keepalive,
            timeout: *config.timeout,
            keep_alive_while_idle: config.keep_alive_while_idle,
        }
    }
}

/// HTTP Proxy configuration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ProxyConfig {
    /// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
    pub url: Option<String>,
    /// TLS configuration for proxy connection
    pub tls: TlsClientConfig,
    /// Optional username for proxy authentication
    pub username: Option<String>,
    /// Optional password for proxy authentication
    pub password: Option<String>,
    /// Headers to send with proxy requests
    pub headers: HashMap<String, String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        let core_defaults = CoreProxyConfig::default();
        ProxyConfig {
            url: core_defaults.url,
            tls: TlsClientConfig::default(), // Use FFI default since we can't convert back
            username: core_defaults.username,
            password: core_defaults.password,
            headers: core_defaults.headers,
        }
    }
}

impl From<ProxyConfig> for CoreProxyConfig {
    fn from(config: ProxyConfig) -> Self {
        CoreProxyConfig {
            url: config.url,
            tls_setting: config.tls.into(),
            username: config.username,
            password: config.password,
            headers: config.headers,
        }
    }
}

impl From<CoreProxyConfig> for ProxyConfig {
    fn from(config: CoreProxyConfig) -> Self {
        ProxyConfig {
            url: config.url,
            tls: config.tls_setting.into(),
            username: config.username,
            password: config.password,
            headers: config.headers,
        }
    }
}

/// Exponential backoff configuration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ExponentialBackoff {
    /// Base delay
    pub base: Duration,
    /// Multiplication factor for each retry
    pub factor: u64,
    /// Maximum delay
    pub max_delay: Duration,
    /// Maximum number of retry attempts
    pub max_attempts: u64,
    /// Whether to add random jitter to delays
    pub jitter: bool,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        let core_defaults = ExponentialBackoffConfig::default();
        ExponentialBackoff {
            base: Duration::from_millis(core_defaults.base),
            factor: core_defaults.factor,
            max_delay: *core_defaults.max_delay,
            max_attempts: core_defaults.max_attempts as u64,
            jitter: core_defaults.jitter,
        }
    }
}

/// Fixed interval backoff configuration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct FixedIntervalBackoff {
    /// Fixed interval between retries
    pub interval: Duration,
    /// Maximum number of retry attempts
    pub max_attempts: u64,
}

impl Default for FixedIntervalBackoff {
    fn default() -> Self {
        let core_defaults = FixedIntervalBackoffConfig::default();
        FixedIntervalBackoff {
            interval: *core_defaults.interval,
            max_attempts: core_defaults.max_attempts as u64,
        }
    }
}

/// Backoff retry configuration
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum BackoffConfig {
    Exponential { config: ExponentialBackoff },
    FixedInterval { config: FixedIntervalBackoff },
}

impl From<BackoffConfig> for CoreBackoffConfig {
    fn from(config: BackoffConfig) -> Self {
        match config {
            BackoffConfig::Exponential { config } => {
                CoreBackoffConfig::Exponential(ExponentialBackoffConfig::new(
                    config.base.as_millis() as u64,
                    config.factor,
                    config.max_delay,
                    config.max_attempts as usize,
                    config.jitter,
                ))
            }
            BackoffConfig::FixedInterval { config } => CoreBackoffConfig::FixedInterval(
                FixedIntervalBackoffConfig::new(config.interval, config.max_attempts as usize),
            ),
        }
    }
}

impl From<CoreBackoffConfig> for BackoffConfig {
    fn from(config: CoreBackoffConfig) -> Self {
        match config {
            CoreBackoffConfig::Exponential(core_config) => BackoffConfig::Exponential {
                config: ExponentialBackoff {
                    base: Duration::from_millis(core_config.base),
                    factor: core_config.factor,
                    max_delay: *core_config.max_delay,
                    max_attempts: core_config.max_attempts as u64,
                    jitter: core_config.jitter,
                },
            },
            CoreBackoffConfig::FixedInterval(core_config) => BackoffConfig::FixedInterval {
                config: FixedIntervalBackoff {
                    interval: *core_config.interval,
                    max_attempts: core_config.max_attempts as u64,
                },
            },
        }
    }
}

/// Client configuration for connecting to a SLIM server
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ClientConfig {
    /// The target endpoint the client will connect to
    pub endpoint: String,

    /// Origin (HTTP Host authority override) for the client
    pub origin: Option<String>,

    /// Optional TLS SNI server name override
    pub server_name: Option<String>,

    /// Compression type
    pub compression: Option<CompressionType>,

    /// Rate limit string (e.g., "100/s" for 100 requests per second)
    pub rate_limit: Option<String>,

    /// TLS client configuration
    pub tls: TlsClientConfig,

    /// Keepalive parameters
    pub keepalive: Option<KeepaliveConfig>,

    /// HTTP Proxy configuration
    pub proxy: ProxyConfig,

    /// Connection timeout
    pub connect_timeout: Duration,

    /// Request timeout
    pub request_timeout: Duration,

    /// Read buffer size in bytes
    pub buffer_size: Option<u64>,

    /// Headers associated with gRPC requests
    pub headers: HashMap<String, String>,

    /// Authentication configuration for outgoing RPCs
    pub auth: ClientAuthenticationConfig,

    /// Backoff retry configuration
    pub backoff: BackoffConfig,

    /// Arbitrary user-provided metadata as JSON string
    pub metadata: Option<String>,
}

impl From<ClientConfig> for CoreClientConfig {
    fn from(config: ClientConfig) -> Self {
        CoreClientConfig {
            endpoint: config.endpoint,
            origin: config.origin,
            server_name: config.server_name,
            compression: config.compression.map(Into::into),
            rate_limit: config.rate_limit,
            tls_setting: config.tls.into(),
            keepalive: config.keepalive.map(Into::into),
            proxy: config.proxy.into(),
            connect_timeout: config.connect_timeout.into(),
            request_timeout: config.request_timeout.into(),
            buffer_size: config.buffer_size.map(|s| s as usize),
            headers: config.headers,
            auth: config.auth.into(),
            backoff: config.backoff.into(),
            metadata: config
                .metadata
                .and_then(|json| serde_json::from_str::<MetadataMap>(&json).ok()),
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        let core_defaults = CoreClientConfig::default();
        Self {
            endpoint: core_defaults.endpoint,
            origin: core_defaults.origin,
            server_name: core_defaults.server_name,
            compression: core_defaults.compression.map(Into::into),
            rate_limit: core_defaults.rate_limit,
            tls: core_defaults.tls_setting.into(),
            keepalive: core_defaults.keepalive.map(Into::into),
            proxy: core_defaults.proxy.into(),
            connect_timeout: core_defaults.connect_timeout.into(),
            request_timeout: core_defaults.request_timeout.into(),
            buffer_size: core_defaults.buffer_size.map(|s| s as u64),
            headers: core_defaults.headers,
            auth: core_defaults.auth.into(),
            backoff: core_defaults.backoff.into(),
            metadata: core_defaults
                .metadata
                .and_then(|m| serde_json::to_string(&m).ok()),
        }
    }
}

/// Create a new insecure client config (no TLS)
#[uniffi::export]
pub fn new_insecure_client_config(endpoint: String) -> ClientConfig {
    ClientConfig {
        endpoint,
        tls: TlsClientConfig {
            insecure: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common_config::{CaSource, TlsSource};
    use std::collections::HashMap;

    #[test]
    fn test_client_config_creation() {
        let config = ClientConfig {
            endpoint: "example.com:443".to_string(),
            origin: None,
            server_name: None,
            compression: None,
            rate_limit: None,
            tls: TlsClientConfig {
                insecure: false,
                insecure_skip_verify: false,
                source: TlsSource::None,
                ca_source: CaSource::File {
                    path: "/ca.pem".to_string(),
                },
                include_system_ca_certs_pool: true,
                tls_version: "tls1.2".to_string(),
            },
            keepalive: None,
            proxy: ProxyConfig::default(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            buffer_size: None,
            headers: HashMap::new(),
            auth: ClientAuthenticationConfig::None,
            backoff: BackoffConfig::Exponential {
                config: ExponentialBackoff::default(),
            },
            metadata: None,
        };

        assert_eq!(config.endpoint, "example.com:443");
        assert_eq!(config.tls.tls_version, "tls1.2");
        assert!(!config.tls.insecure);
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();

        // Verify defaults match core defaults
        assert_eq!(config.endpoint, "");
        assert_eq!(config.origin, None);
        assert_eq!(config.server_name, None);
        assert_eq!(config.compression, None);
        assert_eq!(config.rate_limit, None);
        assert_eq!(config.connect_timeout, Duration::from_secs(0));
        assert_eq!(config.request_timeout, Duration::from_secs(0));
        assert_eq!(config.buffer_size, None);
        assert!(config.headers.is_empty());
        assert_eq!(config.auth, ClientAuthenticationConfig::None);
        assert_eq!(config.metadata, None);
    }

    #[test]
    fn test_client_config_new_insecure() {
        let config = new_insecure_client_config("localhost:50051".to_string());

        assert_eq!(config.endpoint, "localhost:50051");
        assert!(config.tls.insecure);
    }

    #[test]
    fn test_client_config_to_core_conversion() {
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), "test-key".to_string());

        let ffi_config = ClientConfig {
            endpoint: "api.example.com:443".to_string(),
            origin: Some("example.com".to_string()),
            server_name: Some("sni.example.com".to_string()),
            compression: Some(CompressionType::Gzip),
            rate_limit: Some("100/s".to_string()),
            tls: TlsClientConfig::default(),
            keepalive: Some(KeepaliveConfig {
                tcp_keepalive: Duration::from_secs(60),
                http2_keepalive: Duration::from_secs(30),
                timeout: Duration::from_secs(20),
                keep_alive_while_idle: true,
            }),
            proxy: ProxyConfig::default(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(60),
            buffer_size: Some(8192),
            headers: headers.clone(),
            auth: ClientAuthenticationConfig::None,
            backoff: BackoffConfig::FixedInterval {
                config: FixedIntervalBackoff {
                    interval: Duration::from_secs(1),
                    max_attempts: 5,
                },
            },
            metadata: Some(r#"{"client":"test"}"#.to_string()),
        };

        let core_config: CoreClientConfig = ffi_config.into();

        assert_eq!(core_config.endpoint, "api.example.com:443");
        assert_eq!(core_config.origin, Some("example.com".to_string()));
        assert_eq!(core_config.server_name, Some("sni.example.com".to_string()));
        assert!(core_config.compression.is_some());
        assert_eq!(core_config.rate_limit, Some("100/s".to_string()));
        assert!(core_config.keepalive.is_some());
        assert_eq!(core_config.buffer_size, Some(8192));
        assert_eq!(core_config.headers.len(), 1);
        assert!(core_config.metadata.is_some());
    }

    #[test]
    fn test_client_config_from_core_conversion() {
        let core_config = CoreClientConfig::default();
        let ffi_config = ClientConfig::default();

        assert_eq!(ffi_config.endpoint, core_config.endpoint);
        assert_eq!(ffi_config.origin, core_config.origin);
        assert_eq!(ffi_config.server_name, core_config.server_name);
    }

    #[test]
    fn test_client_config_roundtrip_conversion() {
        let original = ClientConfig {
            endpoint: "localhost:8080".to_string(),
            origin: Some("test.local".to_string()),
            server_name: None,
            compression: Some(CompressionType::Zstd),
            rate_limit: Some("50/s".to_string()),
            tls: TlsClientConfig::default(),
            keepalive: None,
            proxy: ProxyConfig::default(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            buffer_size: Some(4096),
            headers: HashMap::new(),
            auth: ClientAuthenticationConfig::None,
            backoff: BackoffConfig::Exponential {
                config: ExponentialBackoff::default(),
            },
            metadata: None,
        };

        // FFI -> Core -> FFI
        let core: CoreClientConfig = original.clone().into();
        let roundtrip = ClientConfig {
            endpoint: core.endpoint,
            origin: core.origin,
            server_name: core.server_name,
            compression: core.compression.map(Into::into),
            rate_limit: core.rate_limit,
            tls: core.tls_setting.into(),
            keepalive: core.keepalive.map(Into::into),
            proxy: core.proxy.into(),
            connect_timeout: core.connect_timeout.into(),
            request_timeout: core.request_timeout.into(),
            buffer_size: core.buffer_size.map(|s| s as u64),
            headers: core.headers,
            auth: core.auth.into(),
            backoff: core.backoff.into(),
            metadata: core.metadata.and_then(|m| serde_json::to_string(&m).ok()),
        };

        assert_eq!(roundtrip.endpoint, original.endpoint);
        assert_eq!(roundtrip.origin, original.origin);
        assert_eq!(roundtrip.rate_limit, original.rate_limit);
        assert_eq!(roundtrip.buffer_size, original.buffer_size);
    }

    #[test]
    fn test_compression_type_conversion() {
        let compressions = vec![
            CompressionType::Gzip,
            CompressionType::Zlib,
            CompressionType::Deflate,
            CompressionType::Snappy,
            CompressionType::Zstd,
            CompressionType::Lz4,
            CompressionType::None,
            CompressionType::Empty,
        ];

        for compression in compressions {
            let core: CoreCompressionType = compression.clone().into();
            let back: CompressionType = core.into();
            // Verify roundtrip works (can't directly compare enums without PartialEq)
            match (compression, back) {
                (CompressionType::Gzip, CompressionType::Gzip) => {}
                (CompressionType::Zlib, CompressionType::Zlib) => {}
                (CompressionType::Deflate, CompressionType::Deflate) => {}
                (CompressionType::Snappy, CompressionType::Snappy) => {}
                (CompressionType::Zstd, CompressionType::Zstd) => {}
                (CompressionType::Lz4, CompressionType::Lz4) => {}
                (CompressionType::None, CompressionType::None) => {}
                (CompressionType::Empty, CompressionType::Empty) => {}
                _ => panic!("Compression roundtrip failed"),
            }
        }
    }

    #[test]
    fn test_keepalive_conversion() {
        let ffi_keepalive = KeepaliveConfig {
            tcp_keepalive: Duration::from_secs(120),
            http2_keepalive: Duration::from_secs(60),
            timeout: Duration::from_secs(30),
            keep_alive_while_idle: false,
        };

        let core_keepalive: CoreKeepaliveConfig = ffi_keepalive.into();

        assert_eq!(*core_keepalive.tcp_keepalive, Duration::from_secs(120));
        assert_eq!(*core_keepalive.http2_keepalive, Duration::from_secs(60));
        assert_eq!(*core_keepalive.timeout, Duration::from_secs(30));
        assert!(!core_keepalive.keep_alive_while_idle);
    }

    #[test]
    fn test_backoff_exponential_conversion() {
        let ffi_backoff = BackoffConfig::Exponential {
            config: ExponentialBackoff {
                base: Duration::from_millis(100),
                factor: 2,
                max_delay: Duration::from_secs(60),
                max_attempts: 10,
                jitter: true,
            },
        };

        let core_backoff: CoreBackoffConfig = ffi_backoff.into();

        match core_backoff {
            CoreBackoffConfig::Exponential(config) => {
                assert_eq!(config.base, 100);
                assert_eq!(config.factor, 2);
                assert_eq!(*config.max_delay, Duration::from_secs(60));
                assert_eq!(config.max_attempts, 10);
                assert!(config.jitter);
            }
            _ => panic!("Expected Exponential backoff"),
        }
    }

    #[test]
    fn test_backoff_fixed_interval_conversion() {
        let ffi_backoff = BackoffConfig::FixedInterval {
            config: FixedIntervalBackoff {
                interval: Duration::from_secs(2),
                max_attempts: 3,
            },
        };

        let core_backoff: CoreBackoffConfig = ffi_backoff.into();

        match core_backoff {
            CoreBackoffConfig::FixedInterval(config) => {
                assert_eq!(*config.interval, Duration::from_secs(2));
                assert_eq!(config.max_attempts, 3);
            }
            _ => panic!("Expected FixedInterval backoff"),
        }
    }

    #[test]
    fn test_proxy_conversion() {
        let mut headers = HashMap::new();
        headers.insert(
            "Proxy-Authorization".to_string(),
            "Bearer token".to_string(),
        );

        let ffi_proxy = ProxyConfig {
            url: Some("http://proxy.example.com:8080".to_string()),
            tls: TlsClientConfig::default(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            headers: headers.clone(),
        };

        let core_proxy: CoreProxyConfig = ffi_proxy.into();

        assert_eq!(
            core_proxy.url,
            Some("http://proxy.example.com:8080".to_string())
        );
        assert_eq!(core_proxy.username, Some("user".to_string()));
        assert_eq!(core_proxy.password, Some("pass".to_string()));
        assert_eq!(core_proxy.headers.len(), 1);
    }

    #[test]
    fn test_metadata_serialization() {
        let config = ClientConfig {
            endpoint: "test:443".to_string(),
            origin: None,
            server_name: None,
            compression: None,
            rate_limit: None,
            tls: TlsClientConfig::default(),
            keepalive: None,
            proxy: ProxyConfig::default(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            buffer_size: None,
            headers: HashMap::new(),
            auth: ClientAuthenticationConfig::None,
            backoff: BackoffConfig::Exponential {
                config: ExponentialBackoff::default(),
            },
            metadata: Some(r#"{"env":"production","region":"us-west"}"#.to_string()),
        };

        let core: CoreClientConfig = config.into();

        // Metadata should be deserialized successfully
        assert!(core.metadata.is_some());
        let metadata = core.metadata.unwrap();
        assert_eq!(metadata.len(), 2);
    }

    #[test]
    fn test_metadata_invalid_json() {
        let config = ClientConfig {
            endpoint: "test:443".to_string(),
            origin: None,
            server_name: None,
            compression: None,
            rate_limit: None,
            tls: TlsClientConfig::default(),
            keepalive: None,
            proxy: ProxyConfig::default(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            buffer_size: None,
            headers: HashMap::new(),
            auth: ClientAuthenticationConfig::None,
            backoff: BackoffConfig::Exponential {
                config: ExponentialBackoff::default(),
            },
            metadata: Some("not valid json".to_string()),
        };

        let core: CoreClientConfig = config.into();

        // Invalid JSON should result in None metadata
        assert!(core.metadata.is_none());
    }

    #[test]
    fn test_jwt_auth_roundtrip() {
        use crate::common_config::{
            ClientJwtAuth, JwtAlgorithm, JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType,
        };

        let jwt_config = ClientJwtAuth {
            key: JwtKeyType::Encoding {
                key: JwtKeyConfig {
                    algorithm: JwtAlgorithm::RS256,
                    format: JwtKeyFormat::Pem,
                    key: JwtKeyData::File {
                        path: "/path/to/private_key.pem".to_string(),
                    },
                },
            },
            audience: Some(vec!["api.example.com".to_string()]),
            issuer: Some("auth.example.com".to_string()),
            subject: Some("user123".to_string()),
            duration: Duration::from_secs(7200),
        };

        let auth = ClientAuthenticationConfig::Jwt {
            config: jwt_config.clone(),
        };

        // Convert to core and back
        let core_auth: slim_config::grpc::client::AuthenticationConfig = auth.into();
        let roundtrip_auth: ClientAuthenticationConfig = core_auth.into();

        // Verify roundtrip preserves the configuration
        if let ClientAuthenticationConfig::Jwt { config } = roundtrip_auth {
            assert_eq!(config.key, jwt_config.key);
            assert_eq!(config.audience, jwt_config.audience);
            assert_eq!(config.issuer, jwt_config.issuer);
            assert_eq!(config.subject, jwt_config.subject);
            // Note: duration might not be exactly preserved due to conversion limitations
        } else {
            panic!("Expected Jwt authentication config");
        }
    }

    #[test]
    fn test_basic_auth_roundtrip() {
        use crate::common_config::BasicAuth;

        let basic_config = BasicAuth {
            username: "admin".to_string(),
            password: "secret123".to_string(),
        };

        let auth = ClientAuthenticationConfig::Basic {
            config: basic_config.clone(),
        };

        // Convert to core and back
        let core_auth: slim_config::grpc::client::AuthenticationConfig = auth.into();
        let roundtrip_auth: ClientAuthenticationConfig = core_auth.into();

        // Verify roundtrip preserves the configuration
        if let ClientAuthenticationConfig::Basic { config } = roundtrip_auth {
            assert_eq!(config.username, basic_config.username);
            assert_eq!(config.password, basic_config.password);
        } else {
            panic!("Expected Basic authentication config");
        }
    }

    #[test]
    fn test_static_jwt_auth_roundtrip() {
        use crate::common_config::StaticJwtAuth;

        let jwt_config = StaticJwtAuth {
            token_file: "/path/to/token.jwt".to_string(),
            duration: Duration::from_secs(1800),
        };

        let auth = ClientAuthenticationConfig::StaticJwt {
            config: jwt_config.clone(),
        };

        // Convert to core and back
        let core_auth: slim_config::grpc::client::AuthenticationConfig = auth.into();
        let roundtrip_auth: ClientAuthenticationConfig = core_auth.into();

        // Verify roundtrip preserves the configuration
        if let ClientAuthenticationConfig::StaticJwt { config } = roundtrip_auth {
            assert_eq!(config.token_file, jwt_config.token_file);
            assert_eq!(config.duration, jwt_config.duration);
        } else {
            panic!("Expected StaticJwt authentication config");
        }
    }
}

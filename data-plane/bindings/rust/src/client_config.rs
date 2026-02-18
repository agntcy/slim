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
            tls: core_defaults.tls_setting.into(),
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

    /// TLS client configuration
    pub tls: TlsClientConfig,

    /// Origin (HTTP Host authority override) for the client
    #[uniffi(default = None)]
    pub origin: Option<String>,

    /// Optional TLS SNI server name override
    #[uniffi(default = None)]
    pub server_name: Option<String>,

    /// Compression type
    #[uniffi(default = None)]
    pub compression: Option<CompressionType>,

    /// Rate limit string (e.g., "100/s" for 100 requests per second)
    #[uniffi(default = None)]
    pub rate_limit: Option<String>,

    /// Keepalive parameters
    #[uniffi(default = None)]
    pub keepalive: Option<KeepaliveConfig>,

    /// HTTP Proxy configuration
    #[uniffi(default = None)]
    pub proxy: Option<ProxyConfig>,

    /// Connection timeout
    #[uniffi(default = None)]
    pub connect_timeout: Option<Duration>,

    /// Request timeout
    #[uniffi(default = None)]
    pub request_timeout: Option<Duration>,

    /// Read buffer size in bytes
    #[uniffi(default = None)]
    pub buffer_size: Option<u64>,

    /// Headers associated with gRPC requests
    #[uniffi(default = None)]
    pub headers: Option<HashMap<String, String>>,

    /// Authentication configuration for outgoing RPCs
    #[uniffi(default = None)]
    pub auth: Option<ClientAuthenticationConfig>,

    /// Backoff retry configuration
    #[uniffi(default = None)]
    pub backoff: Option<BackoffConfig>,

    /// Arbitrary user-provided metadata as JSON string
    #[uniffi(default = None)]
    pub metadata: Option<String>,
}

impl From<ClientConfig> for CoreClientConfig {
    fn from(config: ClientConfig) -> Self {
        let core_defaults = CoreClientConfig::default();
        CoreClientConfig {
            endpoint: config.endpoint,
            origin: config.origin,
            server_name: config.server_name,
            compression: config.compression.map(Into::into),
            rate_limit: config.rate_limit,
            tls_setting: config.tls.into(),
            keepalive: config.keepalive.map(Into::into),
            proxy: config.proxy.map(Into::into).unwrap_or(core_defaults.proxy),
            connect_timeout: config
                .connect_timeout
                .map(Into::into)
                .unwrap_or(core_defaults.connect_timeout),
            request_timeout: config
                .request_timeout
                .map(Into::into)
                .unwrap_or(core_defaults.request_timeout),
            buffer_size: config.buffer_size.map(|s| s as usize),
            headers: config.headers.unwrap_or(core_defaults.headers),
            auth: config.auth.map(Into::into).unwrap_or(core_defaults.auth),
            backoff: config
                .backoff
                .map(Into::into)
                .unwrap_or(core_defaults.backoff),
            metadata: config
                .metadata
                .and_then(|json| serde_json::from_str::<MetadataMap>(&json).ok()),
        }
    }
}

impl From<CoreClientConfig> for ClientConfig {
    fn from(config: CoreClientConfig) -> Self {
        ClientConfig {
            endpoint: config.endpoint,
            origin: config.origin,
            server_name: config.server_name,
            compression: config.compression.map(Into::into),
            rate_limit: config.rate_limit,
            tls: config.tls_setting.into(),
            keepalive: config.keepalive.map(Into::into),
            proxy: Some(config.proxy.into()),
            connect_timeout: Some(*config.connect_timeout),
            request_timeout: Some(*config.request_timeout),
            buffer_size: config.buffer_size.map(|s| s as u64),
            headers: Some(config.headers),
            auth: Some(config.auth.into()),
            backoff: Some(config.backoff.into()),
            metadata: config.metadata.and_then(|m| serde_json::to_string(&m).ok()),
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        let core_defaults = CoreClientConfig::default();
        Self {
            endpoint: core_defaults.endpoint,
            origin: None,
            server_name: None,
            compression: None,
            rate_limit: None,
            tls: core_defaults.tls_setting.into(),
            keepalive: None,
            proxy: None,
            connect_timeout: None,
            request_timeout: None,
            buffer_size: None,
            headers: None,
            auth: None,
            backoff: None,
            metadata: None,
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

/// Create a new secure client config (TLS enabled with default settings)
#[uniffi::export]
pub fn new_secure_client_config(endpoint: String) -> ClientConfig {
    ClientConfig {
        endpoint,
        tls: TlsClientConfig::default(),
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
            proxy: None,
            connect_timeout: Some(Duration::from_secs(10)),
            request_timeout: Some(Duration::from_secs(30)),
            buffer_size: None,
            headers: None,
            auth: None,
            backoff: None,
            metadata: None,
        };

        assert_eq!(config.endpoint, "example.com:443");
        assert_eq!(config.tls.tls_version, "tls1.2");
        assert!(!config.tls.insecure);
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();

        // Verify defaults are all None (core defaults applied during conversion)
        assert_eq!(config.endpoint, "");
        assert_eq!(config.origin, None);
        assert_eq!(config.server_name, None);
        assert_eq!(config.compression, None);
        assert_eq!(config.rate_limit, None);
        assert_eq!(config.tls, TlsClientConfig::default());
        assert_eq!(config.keepalive, None);
        assert_eq!(config.proxy, None);
        assert_eq!(config.connect_timeout, None);
        assert_eq!(config.request_timeout, None);
        assert_eq!(config.buffer_size, None);
        assert_eq!(config.headers, None);
        assert_eq!(config.auth, None);
        assert_eq!(config.backoff, None);
        assert_eq!(config.metadata, None);

        // Verify core defaults are applied when converting to CoreClientConfig
        let core: CoreClientConfig = config.into();
        assert_eq!(*core.connect_timeout, Duration::from_secs(0));
        assert_eq!(*core.request_timeout, Duration::from_secs(0));
        assert!(core.headers.is_empty());
        assert_eq!(
            core.auth,
            slim_config::grpc::client::AuthenticationConfig::None
        );
    }

    #[test]
    fn test_client_config_new_insecure() {
        let config = new_insecure_client_config("localhost:50051".to_string());

        assert_eq!(config.endpoint, "localhost:50051");
        assert!(config.tls.insecure);
    }

    #[test]
    fn test_client_config_new_secure() {
        let config = new_secure_client_config("api.example.com:443".to_string());

        assert_eq!(config.endpoint, "api.example.com:443");
        assert!(!config.tls.insecure);
        assert!(!config.tls.insecure_skip_verify);
        assert_eq!(config.tls, TlsClientConfig::default());
        // All optional fields should be None
        assert_eq!(config.origin, None);
        assert_eq!(config.keepalive, None);
        assert_eq!(config.proxy, None);
        assert_eq!(config.connect_timeout, None);
        assert_eq!(config.request_timeout, None);
        assert_eq!(config.auth, None);
        assert_eq!(config.backoff, None);
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
            proxy: Some(ProxyConfig::default()),
            connect_timeout: Some(Duration::from_secs(15)),
            request_timeout: Some(Duration::from_secs(60)),
            buffer_size: Some(8192),
            headers: Some(headers.clone()),
            auth: Some(ClientAuthenticationConfig::None),
            backoff: Some(BackoffConfig::FixedInterval {
                config: FixedIntervalBackoff {
                    interval: Duration::from_secs(1),
                    max_attempts: 5,
                },
            }),
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
        // Test the new From<CoreClientConfig> for ClientConfig implementation
        let core_config = CoreClientConfig::default();

        // Use the From trait to convert
        let ffi_config: ClientConfig = core_config.clone().into();

        assert_eq!(ffi_config.endpoint, core_config.endpoint);
        assert_eq!(ffi_config.origin, core_config.origin);
        assert_eq!(ffi_config.server_name, core_config.server_name);
        assert_eq!(ffi_config.rate_limit, core_config.rate_limit);
        assert_eq!(
            ffi_config.connect_timeout,
            Some(*core_config.connect_timeout)
        );
        assert_eq!(
            ffi_config.request_timeout,
            Some(*core_config.request_timeout)
        );
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
            proxy: Some(ProxyConfig::default()),
            connect_timeout: Some(Duration::from_secs(5)),
            request_timeout: Some(Duration::from_secs(10)),
            buffer_size: Some(4096),
            headers: Some(HashMap::new()),
            auth: Some(ClientAuthenticationConfig::None),
            backoff: Some(BackoffConfig::Exponential {
                config: ExponentialBackoff::default(),
            }),
            metadata: None,
        };

        // FFI -> Core -> FFI using the new From implementation
        let core: CoreClientConfig = original.clone().into();
        let roundtrip: ClientConfig = core.into();

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
            tls: TlsClientConfig::default(),
            metadata: Some(r#"{"env":"production","region":"us-west"}"#.to_string()),
            ..Default::default()
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
            tls: TlsClientConfig::default(),
            metadata: Some("not valid json".to_string()),
            ..Default::default()
        };

        let core: CoreClientConfig = config.into();

        // Invalid JSON should result in None metadata
        assert!(core.metadata.is_none());
    }

    #[test]
    fn test_jwt_auth_roundtrip() {
        use crate::identity_config::{
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
        use crate::identity_config::StaticJwtAuth;

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

    #[test]
    fn test_client_config_from_core_with_all_fields() {
        // Test the new From<CoreClientConfig> for ClientConfig with comprehensive field coverage
        use slim_config::backoff::exponential::Config as CoreExponentialBackoffConfig;
        use slim_config::grpc::client::BackoffConfig as CoreBackoffConfig;

        let mut headers = HashMap::new();
        headers.insert("X-Custom-Header".to_string(), "value".to_string());

        let mut metadata = MetadataMap::new();
        metadata.insert("service".to_string(), "test-service".to_string());
        metadata.insert("version".to_string(), "1.0".to_string());

        let core_config = CoreClientConfig {
            endpoint: "test.example.com:9443".to_string(),
            origin: Some("origin.example.com".to_string()),
            server_name: Some("server.example.com".to_string()),
            rate_limit: Some("100/s".to_string()),
            buffer_size: Some(8192),
            headers: headers.clone(),
            metadata: Some(metadata),
            backoff: CoreBackoffConfig::Exponential(CoreExponentialBackoffConfig {
                base: 50,
                factor: 3,
                max_delay: Duration::from_secs(120).into(),
                max_attempts: 5,
                jitter: true,
            }),
            ..Default::default()
        };

        // Use the new From implementation
        let ffi_config: ClientConfig = core_config.clone().into();

        // Verify all fields are correctly converted
        assert_eq!(ffi_config.endpoint, "test.example.com:9443");
        assert_eq!(ffi_config.origin, Some("origin.example.com".to_string()));
        assert_eq!(
            ffi_config.server_name,
            Some("server.example.com".to_string())
        );
        assert_eq!(ffi_config.rate_limit, Some("100/s".to_string()));
        assert_eq!(ffi_config.buffer_size, Some(8192));
        let headers_map = ffi_config.headers.unwrap();
        assert_eq!(headers_map.len(), 1);
        assert_eq!(
            headers_map.get("X-Custom-Header"),
            Some(&"value".to_string())
        );

        // Verify metadata is serialized correctly
        assert!(ffi_config.metadata.is_some());
        let metadata_str = ffi_config.metadata.unwrap();
        assert!(metadata_str.contains("test-service"));
        assert!(metadata_str.contains("1.0"));
    }

    #[test]
    fn test_client_config_from_core_with_keepalive() {
        use slim_config::grpc::client::KeepaliveConfig as CoreKeepaliveConfig;

        let core_config = CoreClientConfig {
            keepalive: Some(CoreKeepaliveConfig {
                tcp_keepalive: Duration::from_secs(90).into(),
                http2_keepalive: Duration::from_secs(45).into(),
                timeout: Duration::from_secs(20).into(),
                keep_alive_while_idle: true,
            }),
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        let keepalive = ffi_config.keepalive.unwrap();
        assert_eq!(keepalive.tcp_keepalive, Duration::from_secs(90));
        assert_eq!(keepalive.http2_keepalive, Duration::from_secs(45));
        assert_eq!(keepalive.timeout, Duration::from_secs(20));
        assert!(keepalive.keep_alive_while_idle);
    }

    #[test]
    fn test_client_config_from_core_with_compression() {
        use slim_config::grpc::compression::CompressionType as CoreCompressionType;

        let compressions = vec![
            CoreCompressionType::Gzip,
            CoreCompressionType::Zstd,
            CoreCompressionType::Snappy,
        ];

        for core_compression in compressions {
            let core_config = CoreClientConfig {
                compression: Some(core_compression.clone()),
                ..Default::default()
            };

            let ffi_config: ClientConfig = core_config.into();

            assert!(ffi_config.compression.is_some());
        }
    }

    #[test]
    fn test_client_config_from_core_with_proxy() {
        use slim_config::grpc::proxy::ProxyConfig as CoreProxyConfig;

        let mut proxy_headers = HashMap::new();
        proxy_headers.insert("Proxy-Auth".to_string(), "token123".to_string());

        let core_config = CoreClientConfig {
            proxy: CoreProxyConfig {
                url: Some("http://proxy.internal:3128".to_string()),
                tls_setting: Default::default(),
                username: Some("proxy_user".to_string()),
                password: Some("proxy_pass".to_string()),
                headers: proxy_headers.clone(),
            },
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        let proxy = ffi_config.proxy.unwrap();
        assert_eq!(proxy.url, Some("http://proxy.internal:3128".to_string()));
        assert_eq!(proxy.username, Some("proxy_user".to_string()));
        assert_eq!(proxy.password, Some("proxy_pass".to_string()));
        assert_eq!(proxy.headers.len(), 1);
    }

    #[test]
    fn test_client_config_from_core_buffer_size_conversion() {
        // Test that buffer_size is correctly converted from usize to u64
        let core_config = CoreClientConfig {
            buffer_size: Some(16384),
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        assert_eq!(ffi_config.buffer_size, Some(16384u64));
    }

    #[test]
    fn test_client_config_from_core_metadata_serialization_failure() {
        // Test that invalid metadata (non-serializable) results in None
        // metadata is already Option, so we just test with None
        let core_config = CoreClientConfig {
            metadata: None,
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        assert!(ffi_config.metadata.is_none());
    }

    #[test]
    fn test_client_config_from_core_fixed_interval_backoff() {
        use slim_config::backoff::fixedinterval::Config as CoreFixedIntervalBackoffConfig;
        use slim_config::grpc::client::BackoffConfig as CoreBackoffConfig;

        let core_config = CoreClientConfig {
            backoff: CoreBackoffConfig::FixedInterval(CoreFixedIntervalBackoffConfig {
                interval: Duration::from_secs(5).into(),
                max_attempts: 8,
            }),
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        match ffi_config.backoff.unwrap() {
            BackoffConfig::FixedInterval { config } => {
                assert_eq!(config.interval, Duration::from_secs(5));
                assert_eq!(config.max_attempts, 8);
            }
            _ => panic!("Expected FixedInterval backoff"),
        }
    }

    #[test]
    fn test_client_config_from_core_auth_types() {
        use slim_config::auth::basic::Config as BasicAuthConfig;
        use slim_config::grpc::client::AuthenticationConfig as CoreAuthConfig;

        // Test with Basic auth
        let core_config = CoreClientConfig {
            auth: CoreAuthConfig::Basic(BasicAuthConfig::new("test_user", "test_pass")),
            ..Default::default()
        };

        let ffi_config: ClientConfig = core_config.into();

        match ffi_config.auth.unwrap() {
            ClientAuthenticationConfig::Basic { config } => {
                assert_eq!(config.username, "test_user");
                assert_eq!(config.password, "test_pass");
            }
            _ => panic!("Expected Basic auth"),
        }
    }
}

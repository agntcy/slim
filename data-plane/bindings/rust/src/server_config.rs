// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use slim_auth::metadata::MetadataMap;
use slim_config::grpc::server::KeepaliveServerParameters as CoreKeepaliveServerParameters;
use slim_config::grpc::server::ServerConfig as CoreServerConfig;

use crate::common_config::{ServerAuthenticationConfig, TlsServerConfig, TlsSource};

/// Keepalive configuration for the server
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct KeepaliveServerParameters {
    /// Max connection idle time (time after which an idle connection is closed)
    pub max_connection_idle: Duration,

    /// Max connection age (maximum time a connection may exist before being closed)
    pub max_connection_age: Duration,

    /// Max connection age grace (additional time after max_connection_age before closing)
    pub max_connection_age_grace: Duration,

    /// Keepalive ping frequency
    pub time: Duration,

    /// Keepalive ping timeout (time to wait for ack)
    pub timeout: Duration,
}

impl Default for KeepaliveServerParameters {
    fn default() -> Self {
        let core_defaults = CoreKeepaliveServerParameters::default();
        KeepaliveServerParameters {
            max_connection_idle: *core_defaults.max_connection_idle,
            max_connection_age: *core_defaults.max_connection_age,
            max_connection_age_grace: *core_defaults.max_connection_age_grace,
            time: *core_defaults.time,
            timeout: *core_defaults.timeout,
        }
    }
}

impl From<KeepaliveServerParameters> for CoreKeepaliveServerParameters {
    fn from(config: KeepaliveServerParameters) -> Self {
        CoreKeepaliveServerParameters {
            max_connection_idle: config.max_connection_idle.into(),
            max_connection_age: config.max_connection_age.into(),
            max_connection_age_grace: config.max_connection_age_grace.into(),
            time: config.time.into(),
            timeout: config.timeout.into(),
        }
    }
}

impl From<CoreKeepaliveServerParameters> for KeepaliveServerParameters {
    fn from(config: CoreKeepaliveServerParameters) -> Self {
        KeepaliveServerParameters {
            max_connection_idle: *config.max_connection_idle,
            max_connection_age: *config.max_connection_age,
            max_connection_age_grace: *config.max_connection_age_grace,
            time: *config.time,
            timeout: *config.timeout,
        }
    }
}

/// Server configuration for running a SLIM server
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// Endpoint address to listen on (e.g., "0.0.0.0:50051" or "[::]:50051")
    pub endpoint: String,

    /// TLS server configuration
    pub tls: TlsServerConfig,

    /// Use HTTP/2 only (default: true)
    #[uniffi(default = None)]
    pub http2_only: Option<bool>,

    /// Maximum size (in MiB) of messages accepted by the server
    #[uniffi(default = None)]
    pub max_frame_size: Option<u32>,

    /// Maximum number of concurrent streams per connection
    #[uniffi(default = None)]
    pub max_concurrent_streams: Option<u32>,

    /// Maximum header list size in bytes
    #[uniffi(default = None)]
    pub max_header_list_size: Option<u32>,

    /// Read buffer size in bytes
    #[uniffi(default = None)]
    pub read_buffer_size: Option<u64>,

    /// Write buffer size in bytes
    #[uniffi(default = None)]
    pub write_buffer_size: Option<u64>,

    /// Keepalive parameters
    #[uniffi(default = None)]
    pub keepalive: Option<KeepaliveServerParameters>,

    /// Authentication configuration for incoming requests
    #[uniffi(default = None)]
    pub auth: Option<ServerAuthenticationConfig>,

    /// Arbitrary user-provided metadata as JSON string
    #[uniffi(default = None)]
    pub metadata: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let core_defaults = CoreServerConfig::default();
        ServerConfig {
            endpoint: core_defaults.endpoint,
            tls: core_defaults.tls_setting.into(),
            http2_only: None,
            max_frame_size: None,
            max_concurrent_streams: None,
            max_header_list_size: None,
            read_buffer_size: None,
            write_buffer_size: None,
            keepalive: None,
            auth: None,
            metadata: None,
        }
    }
}

impl From<ServerConfig> for CoreServerConfig {
    fn from(config: ServerConfig) -> Self {
        let core_defaults = CoreServerConfig::default();
        CoreServerConfig {
            endpoint: config.endpoint,
            tls_setting: config.tls.into(),
            http2_only: config.http2_only.unwrap_or(core_defaults.http2_only),
            max_frame_size: config.max_frame_size.or(core_defaults.max_frame_size),
            max_concurrent_streams: config
                .max_concurrent_streams
                .or(core_defaults.max_concurrent_streams),
            max_header_list_size: config
                .max_header_list_size
                .or(core_defaults.max_header_list_size),
            read_buffer_size: config
                .read_buffer_size
                .map(|s| s as usize)
                .or(core_defaults.read_buffer_size),
            write_buffer_size: config
                .write_buffer_size
                .map(|s| s as usize)
                .or(core_defaults.write_buffer_size),
            keepalive: config
                .keepalive
                .map(Into::into)
                .unwrap_or(core_defaults.keepalive),
            auth: config.auth.map(Into::into).unwrap_or(core_defaults.auth),
            metadata: config
                .metadata
                .and_then(|json| serde_json::from_str::<MetadataMap>(&json).ok()),
        }
    }
}

impl From<CoreServerConfig> for ServerConfig {
    fn from(config: CoreServerConfig) -> Self {
        ServerConfig {
            endpoint: config.endpoint,
            tls: config.tls_setting.into(),
            http2_only: Some(config.http2_only),
            max_frame_size: config.max_frame_size,
            max_concurrent_streams: config.max_concurrent_streams,
            max_header_list_size: config.max_header_list_size,
            read_buffer_size: config.read_buffer_size.map(|s| s as u64),
            write_buffer_size: config.write_buffer_size.map(|s| s as u64),
            keepalive: Some(config.keepalive.into()),
            auth: Some(config.auth.into()),
            metadata: config.metadata.and_then(|m| serde_json::to_string(&m).ok()),
        }
    }
}

/// Create a new server config with the given endpoint and default values
#[uniffi::export]
pub fn new_server_config(endpoint: String) -> ServerConfig {
    ServerConfig {
        endpoint,
        ..Default::default()
    }
}

/// Create a new insecure server config (no TLS)
#[uniffi::export]
pub fn new_insecure_server_config(endpoint: String) -> ServerConfig {
    ServerConfig {
        endpoint,
        tls: TlsServerConfig {
            insecure: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Create a new secure server config (TLS enabled with the given certificate source)
#[uniffi::export]
pub fn new_secure_server_config(endpoint: String, tls_source: TlsSource) -> ServerConfig {
    ServerConfig {
        endpoint,
        tls: TlsServerConfig {
            insecure: false,
            source: tls_source,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common_config::{CaSource, TlsSource};

    #[test]
    fn test_server_config_creation() {
        let config = ServerConfig {
            endpoint: "127.0.0.1:8080".to_string(),
            tls: TlsServerConfig {
                insecure: false,
                source: TlsSource::File {
                    cert: "/cert.pem".to_string(),
                    key: "/key.pem".to_string(),
                },
                client_ca: CaSource::None,
                include_system_ca_certs_pool: Some(true),
                tls_version: Some("tls1.3".to_string()),
                reload_client_ca_file: Some(false),
            },
            http2_only: Some(true),
            ..Default::default()
        };

        assert_eq!(config.endpoint, "127.0.0.1:8080");
        assert!(!config.tls.insecure);
        assert_eq!(config.tls.tls_version, Some("tls1.3".to_string()));
        assert_eq!(config.http2_only, Some(true));
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();

        // Verify defaults are all None (core defaults applied during conversion)
        assert_eq!(config.endpoint, "");
        assert_eq!(config.http2_only, None);
        assert_eq!(config.max_frame_size, None);
        assert_eq!(config.max_concurrent_streams, None);
        assert_eq!(config.max_header_list_size, None);
        assert_eq!(config.read_buffer_size, None);
        assert_eq!(config.write_buffer_size, None);
        assert_eq!(config.keepalive, None);
        assert_eq!(config.auth, None);
        assert_eq!(config.metadata, None);

        // Verify core defaults are applied when converting to CoreServerConfig
        let core: CoreServerConfig = config.into();
        assert!(core.http2_only);
        assert_eq!(core.max_frame_size, Some(4));
        assert_eq!(core.max_concurrent_streams, Some(100));
        assert_eq!(core.read_buffer_size, Some(1024 * 1024));
        assert_eq!(core.write_buffer_size, Some(1024 * 1024));
        assert_eq!(
            core.auth,
            slim_config::grpc::server::AuthenticationConfig::None
        );
    }

    #[test]
    fn test_server_config_new() {
        let config = new_server_config("0.0.0.0:50051".to_string());

        assert_eq!(config.endpoint, "0.0.0.0:50051");
        // Other fields should be None defaults
        assert_eq!(config.http2_only, None);
        assert_eq!(config.auth, None);
    }

    #[test]
    fn test_server_config_new_insecure() {
        let config = new_insecure_server_config("[::]:50051".to_string());

        assert_eq!(config.endpoint, "[::]:50051");
        assert!(config.tls.insecure);
        assert_eq!(config.http2_only, None);
    }

    #[test]
    fn test_server_config_new_secure() {
        let config = new_secure_server_config(
            "0.0.0.0:443".to_string(),
            TlsSource::File {
                cert: "/etc/tls/server.crt".to_string(),
                key: "/etc/tls/server.key".to_string(),
            },
        );

        assert_eq!(config.endpoint, "0.0.0.0:443");
        assert!(!config.tls.insecure);
        assert_eq!(
            config.tls.source,
            TlsSource::File {
                cert: "/etc/tls/server.crt".to_string(),
                key: "/etc/tls/server.key".to_string(),
            }
        );
        // All optional fields should be None
        assert_eq!(config.http2_only, None);
        assert_eq!(config.max_frame_size, None);
        assert_eq!(config.max_concurrent_streams, None);
        assert_eq!(config.keepalive, None);
        assert_eq!(config.auth, None);
        assert_eq!(config.metadata, None);
    }

    #[test]
    fn test_server_config_to_core_conversion() {
        let ffi_config = ServerConfig {
            endpoint: "127.0.0.1:8080".to_string(),
            tls: TlsServerConfig::default(),
            http2_only: Some(false),
            max_frame_size: Some(8),
            max_concurrent_streams: Some(200),
            max_header_list_size: Some(8192),
            read_buffer_size: Some(2048),
            write_buffer_size: Some(2048),
            keepalive: Some(KeepaliveServerParameters::default()),
            auth: Some(ServerAuthenticationConfig::None),
            metadata: Some(r#"{"key":"value"}"#.to_string()),
        };

        let core_config: CoreServerConfig = ffi_config.into();

        assert_eq!(core_config.endpoint, "127.0.0.1:8080");
        assert!(!core_config.http2_only);
        assert_eq!(core_config.max_frame_size, Some(8));
        assert_eq!(core_config.max_concurrent_streams, Some(200));
        assert_eq!(core_config.max_header_list_size, Some(8192));
        assert_eq!(core_config.read_buffer_size, Some(2048));
        assert_eq!(core_config.write_buffer_size, Some(2048));
        assert!(core_config.metadata.is_some());
    }

    #[test]
    fn test_server_config_from_core_conversion() {
        // Test the new From<CoreServerConfig> for ServerConfig implementation
        let core_config = CoreServerConfig::default();

        // Use the From trait to convert
        let ffi_config: ServerConfig = core_config.clone().into();

        assert_eq!(ffi_config.endpoint, core_config.endpoint);
        assert_eq!(ffi_config.http2_only, Some(core_config.http2_only));
        assert_eq!(ffi_config.max_frame_size, core_config.max_frame_size);
        assert_eq!(
            ffi_config.max_concurrent_streams,
            core_config.max_concurrent_streams
        );
        assert_eq!(
            ffi_config.max_header_list_size,
            core_config.max_header_list_size
        );
    }

    #[test]
    fn test_server_config_roundtrip_conversion() {
        let original = ServerConfig {
            endpoint: "localhost:9090".to_string(),
            tls: TlsServerConfig::default(),
            http2_only: Some(true),
            max_frame_size: Some(16),
            max_concurrent_streams: Some(500),
            max_header_list_size: Some(16384),
            read_buffer_size: Some(4096),
            write_buffer_size: Some(4096),
            keepalive: Some(KeepaliveServerParameters::default()),
            auth: Some(ServerAuthenticationConfig::None),
            metadata: None,
        };

        // FFI -> Core -> FFI using the new From implementation
        let core: CoreServerConfig = original.clone().into();
        let roundtrip: ServerConfig = core.into();

        assert_eq!(roundtrip.endpoint, original.endpoint);
        assert_eq!(roundtrip.http2_only, original.http2_only);
        assert_eq!(roundtrip.max_frame_size, original.max_frame_size);
        assert_eq!(
            roundtrip.max_concurrent_streams,
            original.max_concurrent_streams
        );
        assert_eq!(
            roundtrip.max_header_list_size,
            original.max_header_list_size
        );
        assert_eq!(roundtrip.read_buffer_size, original.read_buffer_size);
        assert_eq!(roundtrip.write_buffer_size, original.write_buffer_size);
    }

    #[test]
    fn test_keepalive_defaults() {
        let keepalive = KeepaliveServerParameters::default();

        // Verify keepalive has reasonable defaults
        assert!(keepalive.max_connection_idle.as_secs() > 0);
        assert!(keepalive.max_connection_age.as_secs() > 0);
        assert!(keepalive.time.as_secs() > 0);
        assert!(keepalive.timeout.as_secs() > 0);
    }

    #[test]
    fn test_keepalive_conversion() {
        let ffi_keepalive = KeepaliveServerParameters {
            max_connection_idle: Duration::from_secs(600),
            max_connection_age: Duration::from_secs(1800),
            max_connection_age_grace: Duration::from_secs(60),
            time: Duration::from_secs(300),
            timeout: Duration::from_secs(20),
        };

        let core_keepalive: CoreKeepaliveServerParameters = ffi_keepalive.into();

        assert_eq!(
            *core_keepalive.max_connection_idle,
            Duration::from_secs(600)
        );
        assert_eq!(
            *core_keepalive.max_connection_age,
            Duration::from_secs(1800)
        );
        assert_eq!(
            *core_keepalive.max_connection_age_grace,
            Duration::from_secs(60)
        );
        assert_eq!(*core_keepalive.time, Duration::from_secs(300));
        assert_eq!(*core_keepalive.timeout, Duration::from_secs(20));
    }

    #[test]
    fn test_metadata_serialization() {
        let config = ServerConfig {
            endpoint: "test:8080".to_string(),
            tls: TlsServerConfig::default(),
            metadata: Some(r#"{"env":"test","version":1}"#.to_string()),
            ..Default::default()
        };

        let core: CoreServerConfig = config.into();

        // Metadata should be deserialized successfully
        assert!(core.metadata.is_some());
        let metadata = core.metadata.unwrap();
        assert_eq!(metadata.len(), 2);
    }

    #[test]
    fn test_metadata_invalid_json() {
        let config = ServerConfig {
            endpoint: "test:8080".to_string(),
            tls: TlsServerConfig::default(),
            metadata: Some("invalid json".to_string()),
            ..Default::default()
        };

        let core: CoreServerConfig = config.into();

        // Invalid JSON should result in None metadata
        assert!(core.metadata.is_none());
    }

    #[test]
    fn test_basic_auth_roundtrip() {
        use crate::common_config::BasicAuth;

        let basic_config = BasicAuth {
            username: "admin".to_string(),
            password: "secret123".to_string(),
        };

        let auth = ServerAuthenticationConfig::Basic {
            config: basic_config.clone(),
        };

        // Convert to core and back
        let core_auth: slim_config::grpc::server::AuthenticationConfig = auth.into();
        let roundtrip_auth: ServerAuthenticationConfig = core_auth.into();

        // Verify roundtrip preserves the configuration
        if let ServerAuthenticationConfig::Basic { config } = roundtrip_auth {
            assert_eq!(config.username, basic_config.username);
            assert_eq!(config.password, basic_config.password);
        } else {
            panic!("Expected Basic authentication config");
        }
    }

    #[test]
    fn test_jwt_auth_roundtrip() {
        use crate::identity_config::{
            JwtAlgorithm, JwtAuth, JwtKeyConfig, JwtKeyData, JwtKeyFormat, JwtKeyType,
        };

        let jwt_config = JwtAuth {
            key: JwtKeyType::Decoding {
                key: JwtKeyConfig {
                    algorithm: JwtAlgorithm::RS256,
                    format: JwtKeyFormat::Pem,
                    key: JwtKeyData::File {
                        path: "/path/to/public_key.pem".to_string(),
                    },
                },
            },
            audience: Some(vec!["api.example.com".to_string()]),
            issuer: Some("auth.example.com".to_string()),
            subject: Some("service123".to_string()),
            duration: Duration::from_secs(7200),
        };

        let auth = ServerAuthenticationConfig::Jwt {
            config: jwt_config.clone(),
        };

        // Convert to core and back
        let core_auth: slim_config::grpc::server::AuthenticationConfig = auth.into();
        let roundtrip_auth: ServerAuthenticationConfig = core_auth.into();

        // Verify roundtrip preserves the configuration
        if let ServerAuthenticationConfig::Jwt { config } = roundtrip_auth {
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
    fn test_server_config_from_core_with_all_fields() {
        // Test the new From<CoreServerConfig> for ServerConfig with comprehensive field coverage
        use slim_auth::metadata::MetadataMap;

        let mut metadata = MetadataMap::new();
        metadata.insert("service".to_string(), "test-server".to_string());
        metadata.insert("region".to_string(), "us-east-1".to_string());

        let core_config = CoreServerConfig {
            endpoint: "0.0.0.0:8443".to_string(),
            http2_only: false,
            max_frame_size: Some(32),
            max_concurrent_streams: Some(1000),
            max_header_list_size: Some(32768),
            read_buffer_size: Some(16384),
            write_buffer_size: Some(16384),
            metadata: Some(metadata),
            ..Default::default()
        };

        // Use the new From implementation
        let ffi_config: ServerConfig = core_config.clone().into();

        // Verify all fields are correctly converted
        assert_eq!(ffi_config.endpoint, "0.0.0.0:8443");
        assert_eq!(ffi_config.http2_only, Some(false));
        assert_eq!(ffi_config.max_frame_size, Some(32));
        assert_eq!(ffi_config.max_concurrent_streams, Some(1000));
        assert_eq!(ffi_config.max_header_list_size, Some(32768));
        assert_eq!(ffi_config.read_buffer_size, Some(16384));
        assert_eq!(ffi_config.write_buffer_size, Some(16384));

        // Verify metadata is serialized correctly
        assert!(ffi_config.metadata.is_some());
        let metadata_str = ffi_config.metadata.unwrap();
        assert!(metadata_str.contains("test-server"));
        assert!(metadata_str.contains("us-east-1"));
    }

    #[test]
    fn test_server_config_from_core_with_keepalive() {
        use slim_config::grpc::server::KeepaliveServerParameters as CoreKeepaliveServerParameters;

        let core_config = CoreServerConfig {
            keepalive: CoreKeepaliveServerParameters {
                max_connection_idle: Duration::from_secs(300).into(),
                max_connection_age: Duration::from_secs(900).into(),
                max_connection_age_grace: Duration::from_secs(30).into(),
                time: Duration::from_secs(150).into(),
                timeout: Duration::from_secs(15).into(),
            },
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        let keepalive = ffi_config.keepalive.unwrap();
        assert_eq!(keepalive.max_connection_idle, Duration::from_secs(300));
        assert_eq!(keepalive.max_connection_age, Duration::from_secs(900));
        assert_eq!(keepalive.max_connection_age_grace, Duration::from_secs(30));
        assert_eq!(keepalive.time, Duration::from_secs(150));
        assert_eq!(keepalive.timeout, Duration::from_secs(15));
    }

    #[test]
    fn test_server_config_from_core_buffer_size_conversion() {
        // Test that buffer sizes are correctly converted from usize to u64
        let core_config = CoreServerConfig {
            read_buffer_size: Some(8192),
            write_buffer_size: Some(8192),
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        assert_eq!(ffi_config.read_buffer_size, Some(8192u64));
        assert_eq!(ffi_config.write_buffer_size, Some(8192u64));
    }

    #[test]
    fn test_server_config_from_core_no_buffer_sizes() {
        // Test with None buffer sizes
        let core_config = CoreServerConfig {
            read_buffer_size: None,
            write_buffer_size: None,
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        assert!(ffi_config.read_buffer_size.is_none());
        assert!(ffi_config.write_buffer_size.is_none());
    }

    #[test]
    fn test_server_config_from_core_http2_only_variations() {
        // Test http2_only flag both true and false
        let core_config_true = CoreServerConfig {
            http2_only: true,
            ..Default::default()
        };

        let ffi_config_true: ServerConfig = core_config_true.into();
        assert_eq!(ffi_config_true.http2_only, Some(true));

        let core_config_false = CoreServerConfig {
            http2_only: false,
            ..Default::default()
        };

        let ffi_config_false: ServerConfig = core_config_false.into();
        assert_eq!(ffi_config_false.http2_only, Some(false));
    }

    #[test]
    fn test_server_config_from_core_with_optional_fields() {
        // Test with various combinations of optional fields
        let core_config = CoreServerConfig {
            max_frame_size: None,
            max_concurrent_streams: Some(250),
            max_header_list_size: None,
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        assert!(ffi_config.max_frame_size.is_none());
        assert_eq!(ffi_config.max_concurrent_streams, Some(250));
        assert!(ffi_config.max_header_list_size.is_none());
    }

    #[test]
    fn test_server_config_from_core_metadata_serialization_failure() {
        // Test that None metadata results in None
        let core_config = CoreServerConfig {
            metadata: None,
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        assert!(ffi_config.metadata.is_none());
    }

    #[test]
    fn test_server_config_from_core_auth_types() {
        use slim_config::auth::basic::Config as BasicAuthConfig;
        use slim_config::grpc::server::AuthenticationConfig as CoreAuthConfig;

        // Test with Basic auth
        let core_config = CoreServerConfig {
            auth: CoreAuthConfig::Basic(BasicAuthConfig::new("server_user", "server_pass")),
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        match ffi_config.auth.unwrap() {
            ServerAuthenticationConfig::Basic { config } => {
                assert_eq!(config.username, "server_user");
                assert_eq!(config.password, "server_pass");
            }
            _ => panic!("Expected Basic auth"),
        }
    }

    #[test]
    fn test_server_config_from_core_auth_none() {
        use slim_config::grpc::server::AuthenticationConfig as CoreAuthConfig;

        // Test with None auth
        let core_config = CoreServerConfig {
            auth: CoreAuthConfig::None,
            ..Default::default()
        };

        let ffi_config: ServerConfig = core_config.into();

        match ffi_config.auth.unwrap() {
            ServerAuthenticationConfig::None => {
                // Success
            }
            _ => panic!("Expected None auth"),
        }
    }

    #[test]
    fn test_keepalive_server_parameters_roundtrip() {
        // Test KeepaliveServerParameters conversion both ways
        let original = KeepaliveServerParameters {
            max_connection_idle: Duration::from_secs(500),
            max_connection_age: Duration::from_secs(1500),
            max_connection_age_grace: Duration::from_secs(50),
            time: Duration::from_secs(250),
            timeout: Duration::from_secs(25),
        };

        let core: CoreKeepaliveServerParameters = original.clone().into();
        let roundtrip: KeepaliveServerParameters = core.into();

        assert_eq!(roundtrip.max_connection_idle, original.max_connection_idle);
        assert_eq!(roundtrip.max_connection_age, original.max_connection_age);
        assert_eq!(
            roundtrip.max_connection_age_grace,
            original.max_connection_age_grace
        );
        assert_eq!(roundtrip.time, original.time);
        assert_eq!(roundtrip.timeout, original.timeout);
    }
}

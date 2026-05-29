// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic server configuration.
//!
//! [`ServerConfig`] carries connection-agnostic settings (endpoint, TLS,
//! keepalive, auth, metadata, frame/header limits). Transport-specific server
//! plumbing lives in:
//!
//! * `crate::grpc::server` — tonic / gRPC server (`to_server_future`,
//!   `run_server`)
//! * `crate::websocket::server` — WebSocket server (`run_websocket_server`)
//!
//! The two server-side public entry points stay asymmetric on purpose: gRPC
//! takes typed `NamedService` impls; WebSocket takes a connection-accepted
//! callback. The polymorphic dispatch happens in the datapath
//! (`message_processing`) based on `config.transport`.

use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::server_handler::ServerHandler;

use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig as SpireAuthConfig;
use crate::component::configuration::Configuration;
use crate::errors::ConfigError;
use crate::tls::server::TlsServerConfig as TLSSetting;
use crate::transport::{TransportProtocol, validate_endpoint_scheme};
use slim_auth::metadata::MetadataMap;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
pub struct KeepaliveServerParameters {
    /// max_connection_idle sets the time after which an idle connection is closed.
    #[serde(default = "default_max_connection_idle")]
    #[schemars(with = "String")]
    pub max_connection_idle: DurationString,

    /// max_connection_age sets the maximum amount of time a connection may exist before it will be closed.
    #[serde(default = "default_max_connection_age")]
    #[schemars(with = "String")]
    pub max_connection_age: DurationString,

    /// max_connection_age_grace is an additional time given after MaxConnectionAge before closing the connection.
    #[serde(default = "default_max_connection_age_grace")]
    #[schemars(with = "String")]
    pub max_connection_age_grace: DurationString,

    /// Time sets the frequency of the keepalive ping.
    #[serde(default = "default_time")]
    #[schemars(with = "String")]
    pub time: DurationString,

    /// Timeout sets the amount of time the server waits for a keepalive ping ack.
    #[serde(default = "default_timeout")]
    #[schemars(with = "String")]
    pub timeout: DurationString,
}

/// Enum holding one configuration for the server.
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// JWT authentication configuration.
    Jwt(JwtAuthenticationConfig),
    /// SPIRE/SPIFFE authentication configuration.
    #[cfg(not(target_family = "windows"))]
    Spire(SpireAuthConfig),
    /// None
    #[default]
    None,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
pub struct ServerConfig {
    /// Endpoint is the address to listen on.
    ///
    /// The transport protocol is inferred from the endpoint scheme:
    /// * `ws://`, `wss://`  → WebSocket (TLS when `wss`)
    /// * `http://`, `https://`, `unix://`, bare `host:port` → gRPC
    pub endpoint: String,

    /// Configures the protocol to use TLS.
    #[serde(default, rename = "tls")]
    pub tls_setting: TLSSetting,

    /// Use HTTP 2 only.
    #[serde(default = "default_http2_only")]
    pub http2_only: bool,

    /// Maximum size (in MiB) of messages accepted by the server.
    pub max_frame_size: Option<u32>,

    /// MaxConcurrentStreams sets the limit on the number of concurrent streams to each ServerTransport.
    pub max_concurrent_streams: Option<u32>,

    /// Max header list size
    pub max_header_list_size: Option<u32>,

    /// ReadBufferSize for gRPC server.
    // TODO(msardara): not implemented yet
    pub read_buffer_size: Option<usize>,

    /// WriteBufferSize for gRPC server.
    // TODO(msardara): not implemented yet
    pub write_buffer_size: Option<usize>,

    /// Keepalive anchor for all the settings related to keepalive.
    #[serde(default)]
    pub keepalive: KeepaliveServerParameters,

    /// Auth for this receiver.
    #[serde(default)]
    pub auth: AuthenticationConfig,

    /// Arbitrary user-provided metadata.
    pub metadata: Option<MetadataMap>,

    /// Flag to enforce header integrity validation
    /// By default it is `true`
    #[serde(default = "default_require_header_mac")]
    pub require_header_mac: bool,

    /// Timeout (in seconds) to wait for the link HMAC session to be installed.
    /// By default it is 5 seconds.
    #[serde(default = "default_link_hmac_timeout_secs")]
    pub link_hmac_timeout_secs: u64,

    /// Polling interval (in milliseconds) to wait between HMAC existence checks.
    /// By default it is 5 milliseconds.
    #[serde(default = "default_link_hmac_poll_interval_ms")]
    pub link_hmac_poll_interval_ms: u64,
}

/// Default values for KeepaliveServerParameters
impl Default for KeepaliveServerParameters {
    fn default() -> Self {
        Self {
            max_connection_idle: default_max_connection_idle(),
            max_connection_age: default_max_connection_age(),
            max_connection_age_grace: default_max_connection_age_grace(),
            time: default_time(),
            timeout: default_timeout(),
        }
    }
}

fn default_max_connection_idle() -> DurationString {
    Duration::from_secs(3600).into()
}

fn default_max_connection_age() -> DurationString {
    Duration::from_secs(2 * 3600).into()
}

fn default_max_connection_age_grace() -> DurationString {
    Duration::from_secs(5 * 60).into()
}

fn default_time() -> DurationString {
    Duration::from_secs(2 * 60).into()
}

fn default_timeout() -> DurationString {
    Duration::from_secs(20).into()
}

/// Default values for ServerConfig
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            tls_setting: TLSSetting::default(),
            http2_only: default_http2_only(),
            max_frame_size: Some(4),
            max_concurrent_streams: Some(100),
            max_header_list_size: None,
            read_buffer_size: Some(1024 * 1024),
            write_buffer_size: Some(1024 * 1024),
            keepalive: KeepaliveServerParameters::default(),
            auth: AuthenticationConfig::default(),
            metadata: None,
            require_header_mac: true,
            link_hmac_timeout_secs: default_link_hmac_timeout_secs(),
            link_hmac_poll_interval_ms: default_link_hmac_poll_interval_ms(),
        }
    }
}

fn default_http2_only() -> bool {
    true
}

fn default_require_header_mac() -> bool {
    true
}

fn default_link_hmac_timeout_secs() -> u64 {
    5
}

fn default_link_hmac_poll_interval_ms() -> u64 {
    5
}

/// Display implementation for ServerConfig
impl std::fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ServerConfig {{ endpoint: {}, transport: {:?}, tls_setting: {}, http2_only: {}, max_frame_size: {:?}, max_concurrent_streams: {:?}, max_header_list_size: {:?}, read_buffer_size: {:?}, write_buffer_size: {:?}, keepalive: {:?}, auth: {:?}, metadata: {:?}, require_header_mac: {}, link_hmac_timeout_secs: {}, link_hmac_poll_interval_ms: {} }}",
            self.endpoint,
            self.resolved_transport(),
            self.tls_setting,
            self.http2_only,
            self.max_frame_size,
            self.max_concurrent_streams,
            self.max_header_list_size,
            self.read_buffer_size,
            self.write_buffer_size,
            self.keepalive,
            self.auth,
            self.metadata,
            self.require_header_mac,
            self.link_hmac_timeout_secs,
            self.link_hmac_poll_interval_ms,
        )
    }
}

impl Configuration for ServerConfig {
    type Error = ConfigError;

    fn validate(&self) -> Result<(), Self::Error> {
        self.tls_setting.validate()?;
        validate_endpoint_scheme(&self.endpoint)?;
        Ok(())
    }
}

impl ServerConfig {
    pub fn with_endpoint(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            ..Default::default()
        }
    }

    pub fn with_tls_settings(self, tls_setting: TLSSetting) -> Self {
        Self {
            tls_setting,
            ..self
        }
    }

    pub fn with_http2_only(self, http2_only: bool) -> Self {
        Self { http2_only, ..self }
    }

    pub fn with_max_frame_size(self, max_frame_size: Option<u32>) -> Self {
        Self {
            max_frame_size,
            ..self
        }
    }

    pub fn with_max_concurrent_streams(self, max_concurrent_streams: Option<u32>) -> Self {
        Self {
            max_concurrent_streams,
            ..self
        }
    }

    pub fn with_max_header_list_size(self, max_header_list_size: Option<u32>) -> Self {
        Self {
            max_header_list_size,
            ..self
        }
    }

    pub fn with_read_buffer_size(self, read_buffer_size: Option<usize>) -> Self {
        Self {
            read_buffer_size,
            ..self
        }
    }

    pub fn with_write_buffer_size(self, write_buffer_size: Option<usize>) -> Self {
        Self {
            write_buffer_size,
            ..self
        }
    }

    pub fn with_keepalive(self, keepalive: KeepaliveServerParameters) -> Self {
        Self { keepalive, ..self }
    }

    pub fn with_auth(self, auth: AuthenticationConfig) -> Self {
        Self { auth, ..self }
    }

    /// Transport-agnostic server entry point. Dispatches on `self.transport`
    /// and calls into the matching transport-specific run helper, requesting
    /// the appropriate adapter from `handler`. Returns
    /// [`ConfigError::HandlerMissingGrpcSupport`] /
    /// [`ConfigError::HandlerMissingWebSocketSupport`] when the handler does
    /// not implement the method for the configured transport.
    pub async fn run_server<H: ServerHandler>(
        &self,
        watch: drain::Watch,
        handler: Arc<H>,
    ) -> Result<CancellationToken, ConfigError> {
        match self.resolved_transport() {
            TransportProtocol::Grpc => {
                let routes = handler
                    .grpc_routes()
                    .ok_or(ConfigError::HandlerMissingGrpcSupport)?;
                self.run_grpc_server_with_routes(routes, watch).await
            }
            TransportProtocol::Websocket => {
                let on_accepted = handler
                    .on_websocket_accepted()
                    .ok_or(ConfigError::HandlerMissingWebSocketSupport)?;
                self.run_websocket_server(watch, on_accepted).await
            }
        }
    }

    /// Resolve the transport protocol for this configuration by inspecting
    /// the endpoint URI scheme. See [`TransportProtocol::from_endpoint`].
    pub fn resolved_transport(&self) -> TransportProtocol {
        TransportProtocol::from_endpoint(&self.endpoint)
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn server_config_with_metadata_roundtrip_yaml() {
        let mut md = MetadataMap::default();
        md.insert("role", "ingress");
        md.insert("replicas", 3u64);
        let mut nested = MetadataMap::default();
        nested.insert("inner", "v");
        md.insert("nested", nested);

        let cfg = ServerConfig {
            endpoint: "127.0.0.1:50051".to_string(),
            metadata: Some(md.clone()),
            ..Default::default()
        };

        let s = serde_yaml::to_string(&cfg).expect("serialize");
        let deser: ServerConfig = serde_yaml::from_str(&s).expect("deserialize");
        assert_eq!(deser.metadata, Some(md));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_rejects_unknown_scheme() {
        let cfg = ServerConfig::with_endpoint("ftp://0.0.0.0:46357");
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::InvalidEndpointScheme)
        ));
    }

    #[test]
    fn test_validate_accepts_supported_schemes() {
        for endpoint in [
            "0.0.0.0:46357",
            "http://0.0.0.0:46357",
            "https://0.0.0.0:46357",
            "ws://0.0.0.0:46357",
            "wss://0.0.0.0:46357",
            "unix:///tmp/slim.sock",
        ] {
            let cfg = ServerConfig::with_endpoint(endpoint);
            cfg.validate()
                .unwrap_or_else(|e| panic!("endpoint {endpoint} rejected: {e}"));
        }
    }

    #[test]
    fn test_default_keepalive_server_parameters() {
        let keepalive = KeepaliveServerParameters::default();
        assert_eq!(keepalive.max_connection_idle, default_max_connection_idle());
        assert_eq!(keepalive.max_connection_age, default_max_connection_age());
        assert_eq!(
            keepalive.max_connection_age_grace,
            default_max_connection_age_grace()
        );
        assert_eq!(keepalive.time, default_time());
        assert_eq!(keepalive.timeout, default_timeout());
    }

    #[test]
    fn test_default_server_config() {
        let server_config = ServerConfig::default();
        assert_eq!(server_config.endpoint, String::new());
        assert_eq!(server_config.resolved_transport(), TransportProtocol::Grpc);
        assert_eq!(server_config.tls_setting, TLSSetting::default());
        assert_eq!(server_config.http2_only, default_http2_only());
        assert_eq!(server_config.max_frame_size, Some(4));
        assert_eq!(server_config.max_concurrent_streams, Some(100));
        assert_eq!(server_config.max_header_list_size, None);
        assert_eq!(server_config.read_buffer_size, Some(1024 * 1024));
        assert_eq!(server_config.write_buffer_size, Some(1024 * 1024));
        assert_eq!(
            server_config.keepalive,
            KeepaliveServerParameters::default()
        );
        assert_eq!(server_config.auth, AuthenticationConfig::None);
    }

    #[test]
    fn test_keepalive_server_parameters_valid_durations_deserialize() {
        let json = r#"{
            "endpoint": "0.0.0.0:12345",
            "keepalive": {
                "max_connection_idle": "30m",
                "max_connection_age": "1h30m",
                "max_connection_age_grace": "15s",
                "time": "5s",
                "timeout": "2s"
            }
        }"#;

        let cfg: ServerConfig = serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(
            cfg.keepalive.max_connection_idle,
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            cfg.keepalive.max_connection_age,
            Duration::from_secs(90 * 60)
        );
        assert_eq!(
            cfg.keepalive.max_connection_age_grace,
            Duration::from_secs(15)
        );
        assert_eq!(cfg.keepalive.time, Duration::from_secs(5));
        assert_eq!(cfg.keepalive.timeout, Duration::from_secs(2));
    }

    #[test]
    fn test_invalid_keepalive_duration_strings_fail_deserialize() {
        let invalid_json_cases = [
            r#"{ "keepalive": { "time": "zz" } }"#,
            r#"{ "keepalive": { "timeout": "-5s" } }"#,
            r#"{ "keepalive": { "max_connection_age": "10x" } }"#,
        ];
        for js in invalid_json_cases {
            let res: Result<ServerConfig, _> = serde_json::from_str(js);
            assert!(res.is_err(), "expected error for json: {}", js);
        }
    }

    #[test]
    fn test_server_config_keepalive_roundtrip_duration_serialization() {
        let keepalive = KeepaliveServerParameters {
            max_connection_idle: Duration::from_secs(10).into(),
            max_connection_age: Duration::from_secs(20).into(),
            max_connection_age_grace: Duration::from_secs(30).into(),
            time: Duration::from_secs(3).into(),
            timeout: Duration::from_secs(1).into(),
        };

        let cfg = ServerConfig::with_endpoint("127.0.0.1:50000").with_keepalive(keepalive.clone());
        let serialized = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: ServerConfig = serde_json::from_str(&serialized).expect("deserialize");

        assert_eq!(
            deserialized.keepalive.max_connection_idle,
            Duration::from_secs(10)
        );
        assert_eq!(
            deserialized.keepalive.max_connection_age,
            Duration::from_secs(20)
        );
        assert_eq!(
            deserialized.keepalive.max_connection_age_grace,
            Duration::from_secs(30)
        );
        assert_eq!(deserialized.keepalive.time, Duration::from_secs(3));
        assert_eq!(deserialized.keepalive.timeout, Duration::from_secs(1));
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic client configuration.
//!
//! [`ClientConfig`] carries all the connection settings (endpoint, TLS,
//! keepalive, auth, headers, proxy, etc.) and exposes the polymorphic
//! [`ClientConfig::to_channel`] entry point. The transport-specific
//! channel-construction code lives in:
//!
//! * `crate::grpc::client` — gRPC tonic channel building
//! * `crate::websocket::client` — WebSocket channel building

use duration_string::DurationString;
use std::{collections::HashMap, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tonic::codegen::{Body, Bytes, StdError};

use slim_auth::metadata::MetadataMap;

use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig as SpireAuthConfig;
use crate::auth::static_jwt::Config as BearerAuthenticationConfig;
use crate::backoff::Strategy;
use crate::backoff::exponential::Config as ExponentialBackoff;
use crate::backoff::fixedinterval::Config as FixedIntervalBackoff;

/// The type of connection a client establishes.
///
/// Determines how the datapath treats messages on this connection
/// (routing rules, subscription forwarding, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConnType {
    /// Connection with a local application (agent).
    /// Not exposed in configuration — only used internally.
    #[serde(skip)]
    Local,
    /// Connection with a remote SLIM instance in other deployment.
    /// Always handled by the control plane.
    #[default]
    Remote,
    /// Connection with a peer replica in the same deployment.
    Peer,
    /// Connection to the first SLIM node in the network
    /// Edge connections are handled by the data plane
    Edge,
}

impl ConnType {
    /// All variants, for iteration.
    pub const ALL: [ConnType; 4] = [
        ConnType::Local,
        ConnType::Remote,
        ConnType::Peer,
        ConnType::Edge,
    ];

    /// Number of ConnType variants (derived automatically).
    pub const COUNT: usize = Self::ALL.len();

    /// Index for array-based storage. Stable mapping.
    pub const fn index(self) -> usize {
        match self {
            ConnType::Local => 0,
            ConnType::Remote => 1,
            ConnType::Peer => 2,
            ConnType::Edge => 3,
        }
    }

    /// Converts from the legacy `is_local` boolean for backward compatibility.
    pub fn from_is_local(is_local: bool) -> Self {
        if is_local {
            ConnType::Local
        } else {
            ConnType::Remote
        }
    }

    /// Returns true if this is a local connection (app/agent).
    pub fn is_local(self) -> bool {
        matches!(self, ConnType::Local)
    }
}
use crate::component::configuration::Configuration;
use crate::errors::ConfigError;
use crate::grpc::compression::CompressionType;
use crate::grpc::proxy::ProxyConfig;
use crate::tls::client::TlsClientConfig as TLSSetting;
use crate::transport::{TransportProtocol, validate_endpoint_scheme};
use crate::websocket::client::WebSocketClientChannel;

/// Result of [`ClientConfig::to_channel`]: either a gRPC channel (the generic
/// `G` parameter) or a WebSocket channel.
///
/// [`WebSocketClientChannel`] is internally `Arc`-backed and cheap to clone,
/// matching the gRPC variant (a `tonic::transport::Channel` also wraps an
/// internal `Arc`).
pub enum TransportChannel<G> {
    Grpc(G),
    Websocket(WebSocketClientChannel),
}

/// Keepalive configuration for the client.
/// This struct contains the keepalive time for TCP and HTTP2,
/// the timeout duration for the keepalive, and whether to permit
/// keepalive without an active stream.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
pub struct KeepaliveConfig {
    /// The duration of the keepalive time for TCP
    #[serde(default = "default_tcp_keepalive")]
    #[schemars(with = "String")]
    pub tcp_keepalive: DurationString,

    /// The duration of the keepalive time for HTTP2
    #[serde(default = "default_http2_keepalive")]
    #[schemars(with = "String")]
    pub http2_keepalive: DurationString,

    /// The timeout duration for the keepalive
    #[serde(default = "default_timeout")]
    #[schemars(with = "String")]
    pub timeout: DurationString,

    /// Whether to permit keepalive without an active stream
    #[serde(default = "default_keep_alive_while_idle")]
    pub keep_alive_while_idle: bool,
}

/// Defaults for KeepaliveConfig
impl Default for KeepaliveConfig {
    fn default() -> Self {
        KeepaliveConfig {
            tcp_keepalive: default_tcp_keepalive(),
            http2_keepalive: default_http2_keepalive(),
            timeout: default_timeout(),
            keep_alive_while_idle: default_keep_alive_while_idle(),
        }
    }
}

fn default_tcp_keepalive() -> DurationString {
    Duration::from_secs(60).into()
}

fn default_http2_keepalive() -> DurationString {
    Duration::from_secs(60).into()
}

fn default_timeout() -> DurationString {
    Duration::from_secs(10).into()
}

fn default_keep_alive_while_idle() -> bool {
    false
}

/// Enum holding one configuration for the client.
#[derive(Debug, Serialize, Default, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// Bearer authentication configuration.
    StaticJwt(BearerAuthenticationConfig),
    /// JWT authentication configuration.
    Jwt(JwtAuthenticationConfig),
    /// SPIRE/SPIFFE authentication configuration.
    #[cfg(not(target_family = "windows"))]
    Spire(SpireAuthConfig),
    /// None
    #[default]
    None,
}

/// Enum holding one configuration for the client.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BackoffConfig {
    // Exponential backoff retry config.
    Exponential(ExponentialBackoff),
    /// FixedInterval backoff retry config.
    FixedInterval(FixedIntervalBackoff),
}

impl BackoffConfig {
    /// Creates a new Exponential backoff configuration
    pub fn new_exponential(
        base: u64,
        factor: u64,
        max_delay: Duration,
        max_attempts: usize,
        jitter: bool,
    ) -> Self {
        BackoffConfig::Exponential(ExponentialBackoff::new(
            base,
            factor,
            max_delay,
            max_attempts,
            jitter,
        ))
    }

    /// Creates a new FixedInterval backoff configuration
    pub fn new_fixed_interval(interval: Duration, max_attempts: usize) -> Self {
        BackoffConfig::FixedInterval(FixedIntervalBackoff::new(interval, max_attempts))
    }
}

impl Default for BackoffConfig {
    fn default() -> Self {
        BackoffConfig::Exponential(ExponentialBackoff::default())
    }
}

impl Strategy for BackoffConfig {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        match self {
            BackoffConfig::Exponential(b) => b.get_strategy(),
            BackoffConfig::FixedInterval(b) => b.get_strategy(),
        }
    }
}

/// Struct for the client configuration.
/// This struct contains the endpoint, origin, compression type, rate limit,
/// TLS settings, keepalive settings, proxy settings, timeout settings, buffer size settings,
/// headers, and auth settings.
/// The client configuration can be converted to a transport channel via
/// [`ClientConfig::to_channel`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ClientConfig {
    /// The target the client will connect to.
    ///
    /// The transport protocol is inferred from the endpoint scheme:
    /// * `ws://`, `wss://`  → WebSocket (TLS when `wss`)
    /// * `http://`, `https://`, `unix://`, bare `host:port` → gRPC
    pub endpoint: String,

    /// Origin (HTTP Host authority override) for the client.
    pub origin: Option<String>,

    /// Optional TLS SNI server name override. If set, this value is used for TLS
    /// server name verification (SNI) instead of the host extracted from endpoint/origin.
    pub server_name: Option<String>,

    /// Compression type - TODO(msardara): not implemented yet.
    pub compression: Option<CompressionType>,

    /// Rate Limits
    pub rate_limit: Option<String>,

    /// TLS client configuration.
    #[serde(default, rename = "tls")]
    pub tls_setting: TLSSetting,

    /// Keepalive parameters.
    pub keepalive: Option<KeepaliveConfig>,

    /// HTTP Proxy configuration.
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Timeout for the connection.
    #[serde(default = "default_connect_timeout")]
    #[schemars(with = "String")]
    pub connect_timeout: DurationString,

    /// Timeout per request.
    #[serde(default = "default_request_timeout")]
    #[schemars(with = "String")]
    pub request_timeout: DurationString,

    /// ReadBufferSize.
    pub buffer_size: Option<usize>,

    /// The headers associated with gRPC requests.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Auth configuration for outgoing RPCs.
    #[serde(default)]
    pub auth: AuthenticationConfig,

    /// Backoff retry configuration.
    #[serde(default)]
    pub backoff: BackoffConfig,

    /// Arbitrary user-provided metadata.
    pub metadata: Option<MetadataMap>,

    /// Link identifier for this connection, used during link negotiation.
    /// Defaults to a randomly generated UUID v4.
    #[serde(default = "default_link_id")]
    pub link_id: String,

    /// Flag to enforce header integrity validation
    #[serde(default = "default_require_header_mac")]
    pub require_header_mac: bool,

    /// The type of connection this client establishes.
    /// Defaults to `remote`. Set to `peer` for intra-deployment peer connections.
    #[serde(default)]
    pub connection_type: ConnType,
}

/// Defaults for ClientConfig
impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            endpoint: String::new(),
            origin: None,
            server_name: None,
            compression: None,
            rate_limit: None,
            tls_setting: TLSSetting::default(),
            keepalive: None,
            proxy: ProxyConfig::default(),
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            buffer_size: None,
            headers: HashMap::new(),
            auth: AuthenticationConfig::None,
            backoff: BackoffConfig::default(),
            metadata: None,
            link_id: default_link_id(),
            require_header_mac: true,
            connection_type: ConnType::default(),
        }
    }
}

fn default_link_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_require_header_mac() -> bool {
    true
}

fn default_connect_timeout() -> DurationString {
    Duration::from_secs(0).into()
}

fn default_request_timeout() -> DurationString {
    Duration::from_secs(0).into()
}

// Display for ClientConfig
impl std::fmt::Display for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ClientConfig {{ endpoint: {}, transport: {:?}, origin: {:?}, server_name: {:?}, compression: {:?}, rate_limit: {:?}, tls_setting: {:?}, keepalive: {:?}, proxy: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, auth: {:?}, backoff: {:?}, metadata: {:?}, link_id: {:?}, connection_type: {:?} }}",
            self.endpoint,
            self.resolved_transport(),
            self.origin,
            self.server_name,
            self.compression,
            self.rate_limit,
            self.tls_setting,
            self.keepalive,
            self.proxy,
            self.connect_timeout,
            self.request_timeout,
            self.buffer_size,
            self.headers,
            self.auth,
            self.backoff,
            self.metadata,
            self.link_id,
            self.connection_type
        )
    }
}

impl Configuration for ClientConfig {
    type Error = ConfigError;

    fn validate(&self) -> Result<(), Self::Error> {
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        // Validate the client configuration
        self.tls_setting.validate()?;
        validate_endpoint_scheme(&self.endpoint)?;

        Ok(())
    }
}

impl ClientConfig {
    /// Creates a new client configuration with the given endpoint.
    /// This function will return a ClientConfig with the endpoint set
    /// and all other fields set to default.
    pub fn with_endpoint(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            ..Self::default()
        }
    }

    pub fn with_origin(self, origin: &str) -> Self {
        Self {
            origin: Some(origin.to_string()),
            ..self
        }
    }

    pub fn with_server_name(self, server_name: &str) -> Self {
        Self {
            server_name: Some(server_name.to_string()),
            ..self
        }
    }

    pub fn with_compression(self, compression: CompressionType) -> Self {
        Self {
            compression: Some(compression),
            ..self
        }
    }

    pub fn with_rate_limit(self, rate_limit: &str) -> Self {
        Self {
            rate_limit: Some(rate_limit.to_string()),
            ..self
        }
    }

    pub fn with_tls_setting(self, tls_setting: TLSSetting) -> Self {
        Self {
            tls_setting,
            ..self
        }
    }

    pub fn with_keepalive(self, keepalive: KeepaliveConfig) -> Self {
        Self {
            keepalive: Some(keepalive),
            ..self
        }
    }

    pub fn with_proxy(self, proxy: ProxyConfig) -> Self {
        Self { proxy, ..self }
    }

    pub fn with_connect_timeout(self, connect_timeout: Duration) -> Self {
        Self {
            connect_timeout: connect_timeout.into(),
            ..self
        }
    }

    pub fn with_request_timeout(self, request_timeout: Duration) -> Self {
        Self {
            request_timeout: request_timeout.into(),
            ..self
        }
    }

    pub fn with_buffer_size(self, buffer_size: usize) -> Self {
        Self {
            buffer_size: Some(buffer_size),
            ..self
        }
    }

    pub fn with_headers(self, headers: HashMap<String, String>) -> Self {
        Self { headers, ..self }
    }

    pub fn with_auth(self, auth: AuthenticationConfig) -> Self {
        Self { auth, ..self }
    }

    pub fn with_backoff(self, backoff: BackoffConfig) -> Self {
        Self { backoff, ..self }
    }

    /// Run a single connect attempt under this config's backoff/retry policy.
    ///
    /// This is the single place where the retry loop lives: all transports
    /// (gRPC, WebSocket, future ones) share the same backoff strategy and the
    /// same retryable-error classification ([`ConfigError::is_retryable_connect_error`]).
    /// Each transport-specific builder only needs to expose a one-shot attempt
    /// and call this helper.
    pub(crate) async fn retry_connect<T, F, Fut>(&self, attempt: F) -> Result<T, ConfigError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, ConfigError>>,
    {
        use crate::backoff::Strategy;
        use tokio_retry::RetryIf;

        let strategy = self.backoff.get_strategy();
        RetryIf::spawn(strategy, attempt, |e: &ConfigError| {
            let retry = e.is_retryable_connect_error();
            if retry {
                tracing::warn!(error = %e, "transient connect error, retrying");
            } else {
                tracing::error!(error = %e, "non-retryable connect error");
            }
            retry
        })
        .await
    }

    pub fn with_metadata(self, metadata: MetadataMap) -> Self {
        Self {
            metadata: Some(metadata),
            ..self
        }
    }

    pub fn with_connection_type(self, connection_type: ConnType) -> Self {
        Self {
            connection_type,
            ..self
        }
    }

    /// Build a transport channel from this configuration. The returned
    /// [`TransportChannel`] variant matches `self.transport`.
    ///
    /// This is the single public entry point callers should use; the
    /// transport-specific builders (`to_grpc_channel` / `to_websocket_channel`)
    /// are crate-private.
    pub async fn to_channel(
        &self,
    ) -> Result<
        TransportChannel<
            impl tonic::client::GrpcService<
                tonic::body::Body,
                Error: Into<StdError> + Send,
                ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                                  + Send
                                  + 'static,
                Future: Send,
            >
            + Send
            + Clone
            + 'static
            + use<>,
        >,
        ConfigError,
    > {
        match self.resolved_transport() {
            TransportProtocol::Grpc => Ok(TransportChannel::Grpc(self.to_grpc_channel().await?)),
            TransportProtocol::Websocket => Ok(TransportChannel::Websocket(
                self.to_websocket_channel().await?,
            )),
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
    fn client_config_with_metadata_roundtrip_json() {
        let mut md = MetadataMap::default();
        md.insert("feature", "alpha");
        md.insert("level", 2u64);

        let cfg = ClientConfig::with_endpoint("http://localhost:1234").with_metadata(md.clone());
        let s = serde_json::to_string(&cfg).expect("serialize");
        let deser: ClientConfig = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(deser.metadata, Some(md));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_keepalive_config() {
        let keepalive = KeepaliveConfig::default();
        assert_eq!(keepalive.tcp_keepalive, Duration::from_secs(60));
        assert_eq!(keepalive.http2_keepalive, Duration::from_secs(60));
        assert_eq!(keepalive.timeout, Duration::from_secs(10));
        assert!(!keepalive.keep_alive_while_idle);
    }

    #[test]
    fn test_default_client_config() {
        let client = ClientConfig::default();
        assert_eq!(client.endpoint, String::new());
        assert_eq!(client.resolved_transport(), TransportProtocol::Grpc);
        assert_eq!(client.origin, None);
        assert_eq!(client.compression, None);
        assert_eq!(client.rate_limit, None);
        assert_eq!(client.tls_setting, TLSSetting::default());
        assert_eq!(client.keepalive, None);
        assert_eq!(client.connect_timeout, Duration::from_secs(0));
        assert_eq!(client.request_timeout, Duration::from_secs(0));
        assert_eq!(client.buffer_size, None);
        assert_eq!(client.headers, HashMap::new());
        assert_eq!(client.auth, AuthenticationConfig::None);
    }

    #[test]
    fn test_transport_inferred_from_endpoint_scheme() {
        assert_eq!(
            ClientConfig::with_endpoint("ws://localhost:46357").resolved_transport(),
            TransportProtocol::Websocket
        );
        assert_eq!(
            ClientConfig::with_endpoint("wss://localhost:46357").resolved_transport(),
            TransportProtocol::Websocket
        );
        assert_eq!(
            ClientConfig::with_endpoint("http://localhost:46357").resolved_transport(),
            TransportProtocol::Grpc
        );
        assert_eq!(
            ClientConfig::with_endpoint("https://localhost:46357").resolved_transport(),
            TransportProtocol::Grpc
        );
        assert_eq!(
            ClientConfig::with_endpoint("127.0.0.1:46357").resolved_transport(),
            TransportProtocol::Grpc
        );

        // Validation rejects unknown schemes.
        let invalid = ClientConfig::with_endpoint("ftp://localhost:46357");
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::InvalidEndpointScheme)
        ));
    }

    #[test]
    fn test_client_config_with_proxy() {
        let test_password: String = format!("test-{}-{}", std::process::id(), line!()); // pragma: allowlist secret
        let proxy =
            ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", &test_password);
        let client = ClientConfig::with_endpoint("http://localhost:8080").with_proxy(proxy.clone());
        assert_eq!(client.proxy, proxy);
    }

    #[test]
    fn test_connect_and_request_timeout_valid_durations_deserialize() {
        let json = r#"{
            "endpoint": "http://localhost:1234",
            "connect_timeout": "1m30s",
            "request_timeout": "250ms"
        }"#;

        let cfg: ClientConfig = serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(cfg.connect_timeout, Duration::from_secs(90));
        assert_eq!(cfg.request_timeout, Duration::from_millis(250));

        // More complex duration
        let json = r#"{
            "endpoint": "http://localhost:1234",
            "connect_timeout": "1h2m3s4ms",
            "request_timeout": "1500ms"
        }"#;
        let cfg: ClientConfig =
            serde_json::from_str(json).expect("complex duration should deserialize");
        assert_eq!(
            cfg.connect_timeout,
            Duration::from_secs(3723) + Duration::from_millis(4)
        );
        assert_eq!(cfg.request_timeout, Duration::from_millis(1500));
    }

    #[test]
    fn test_invalid_duration_strings_fail_deserialize() {
        let invalids = [
            r#"{ "endpoint": "http://localhost:1234", "connect_timeout": "abc" }"#,
            r#"{ "endpoint": "http://localhost:1234", "request_timeout": "10x" }"#,
            r#"{ "endpoint": "http://localhost:1234", "request_timeout": "--5s" }"#,
        ];
        for js in invalids {
            let res: Result<ClientConfig, _> = serde_json::from_str(js);
            assert!(res.is_err(), "expected error for json: {}", js);
        }
    }

    #[test]
    fn test_keepalive_config_duration_parsing() {
        let json = r#"{
            "endpoint": "http://localhost:1234",
            "keepalive": {
                "tcp_keepalive": "30s",
                "http2_keepalive": "45s",
                "timeout": "5s",
                "keep_alive_while_idle": true
            }
        }"#;
        let cfg: ClientConfig = serde_json::from_str(json).expect("keepalive should deserialize");
        let ka = cfg.keepalive.expect("keepalive should be present");
        assert_eq!(ka.tcp_keepalive, Duration::from_secs(30));
        assert_eq!(ka.http2_keepalive, Duration::from_secs(45));
        assert_eq!(ka.timeout, Duration::from_secs(5));
        assert!(ka.keep_alive_while_idle);

        // Invalid keepalive duration
        let invalid_json = r#"{
            "endpoint": "http://localhost:1234",
            "keepalive": { "tcp_keepalive": "zz", "http2_keepalive": "10s", "timeout": "5s", "keep_alive_while_idle": false }
        }"#;
        let res: Result<ClientConfig, _> = serde_json::from_str(invalid_json);
        assert!(res.is_err(), "invalid tcp_keepalive should fail");
    }

    #[test]
    fn test_client_config_roundtrip_duration_serialization() {
        let mut cfg = ClientConfig::with_endpoint("http://localhost:9999")
            .with_connect_timeout(Duration::from_secs(90))
            .with_request_timeout(Duration::from_millis(750));

        cfg.keepalive = Some(KeepaliveConfig {
            tcp_keepalive: Duration::from_secs(11).into(),
            http2_keepalive: Duration::from_secs(22).into(),
            timeout: Duration::from_secs(3).into(),
            keep_alive_while_idle: true,
        });

        let serialized = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: ClientConfig = serde_json::from_str(&serialized).expect("deserialize");

        assert_eq!(deserialized.connect_timeout, Duration::from_secs(90));
        assert_eq!(deserialized.request_timeout, Duration::from_millis(750));
        let ka = deserialized.keepalive.expect("keepalive present");
        assert_eq!(ka.tcp_keepalive, Duration::from_secs(11));
        assert_eq!(ka.http2_keepalive, Duration::from_secs(22));
        assert_eq!(ka.timeout, Duration::from_secs(3));
        assert!(ka.keep_alive_while_idle);
    }

    #[test]
    fn test_validate_accepts_any_link_id() {
        let mut config = ClientConfig::with_endpoint("http://localhost:1234");
        config.link_id = "my-custom-link-id".to_string();
        assert!(config.validate().is_ok());
    }
}

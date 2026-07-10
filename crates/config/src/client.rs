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

use crate::component::configuration::Configuration;
use crate::conn_type::ConnType;
use crate::errors::ConfigError;
#[cfg(not(target_arch = "wasm32"))]
use crate::grpc::compression::CompressionType;
#[cfg(not(target_arch = "wasm32"))]
use crate::grpc::proxy::ProxyConfig;
#[cfg(not(target_arch = "wasm32"))]
use crate::tls::client::TlsClientConfig as TLSSetting;
#[cfg(not(target_arch = "wasm32"))]
use crate::tls::errors::ConfigError as TlsConfigError;
use crate::transport::{TransportProtocol, validate_endpoint_scheme};
use crate::websocket::client::WebSocketClientChannel;

// Auth, TLS, proxy, compression, gRPC and the backoff/retry strategy are
// native-only: the browser build connects out over `wss://` (TLS handled by the
// browser) without these layers, so they are gated off wasm32.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use display_error_chain::ErrorChainExt;
        use slim_auth::metadata::MetadataMap;
        use tonic::codegen::{Body, Bytes, StdError};

        use crate::auth::basic::Config as BasicAuthenticationConfig;
        use crate::auth::jwt::Config as JwtAuthenticationConfig;
        #[cfg(not(target_family = "windows"))]
        use crate::auth::spire::SpireConfig as SpireAuthConfig;
        use crate::auth::static_jwt::Config as BearerAuthenticationConfig;
        use crate::backoff::exponential::Config as ExponentialBackoff;
        use crate::backoff::fixedinterval::Config as FixedIntervalBackoff;
        use crate::backoff::Strategy;
    }
}

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

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        /// Enum holding one authentication configuration for the client.
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

        /// Enum holding one backoff configuration for the client.
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
    #[cfg(not(target_arch = "wasm32"))]
    pub compression: Option<CompressionType>,

    /// Rate Limits
    pub rate_limit: Option<String>,

    /// TLS client configuration.
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default, rename = "tls")]
    pub tls_setting: TLSSetting,

    /// Keepalive parameters.
    pub keepalive: Option<KeepaliveConfig>,

    /// HTTP Proxy configuration.
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub auth: AuthenticationConfig,

    /// Backoff retry configuration.
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(default)]
    pub backoff: BackoffConfig,

    /// Arbitrary user-provided metadata.
    #[cfg(not(target_arch = "wasm32"))]
    pub metadata: Option<MetadataMap>,

    /// Link identifier for this connection, used during link negotiation.
    /// Defaults to a randomly generated UUID v4.
    #[serde(default = "default_link_id")]
    pub link_id: String,

    /// Flag to enforce header integrity validation
    #[serde(default = "default_require_header_mac")]
    pub require_header_mac: bool,

    /// The type of connection this client establishes.
    /// Defaults to `edge`. Set to `peer` for intra-deployment peer connections,
    /// or `remote` for control-plane-managed inter-deployment links.
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
            #[cfg(not(target_arch = "wasm32"))]
            compression: None,
            rate_limit: None,
            #[cfg(not(target_arch = "wasm32"))]
            tls_setting: TLSSetting::default(),
            keepalive: None,
            #[cfg(not(target_arch = "wasm32"))]
            proxy: ProxyConfig::default(),
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            buffer_size: None,
            headers: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            auth: AuthenticationConfig::None,
            #[cfg(not(target_arch = "wasm32"))]
            backoff: BackoffConfig::default(),
            #[cfg(not(target_arch = "wasm32"))]
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

// `Display` prints the target's field set (the browser build has no
// auth/TLS/proxy/compression fields), so each impl lives in its own file and is
// selected here — file-split rather than an in-line `cfg_if!` so "what exists on
// wasm" is answerable by `ls`.
#[cfg(not(target_arch = "wasm32"))]
#[path = "client_display_native.rs"]
mod client_display;
#[cfg(target_arch = "wasm32")]
#[path = "client_display_wasm.rs"]
mod client_display;

impl Configuration for ClientConfig {
    type Error = ConfigError;

    fn validate(&self) -> Result<(), Self::Error> {
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        // Validate the client configuration
        #[cfg(not(target_arch = "wasm32"))]
        self.tls_setting.validate()?;
        validate_endpoint_scheme(&self.endpoint)?;

        Ok(())
    }
}

// Transport-agnostic surface: the builder methods and helpers that exist on
// every target. Native-only builders (TLS/auth/proxy/compression/backoff/
// metadata) and the native `to_channel` live in the `#[cfg(not(wasm32))]` impl
// below; the browser `to_channel` lives in the `#[cfg(wasm32)]` impl.
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

    pub fn with_rate_limit(self, rate_limit: &str) -> Self {
        Self {
            rate_limit: Some(rate_limit.to_string()),
            ..self
        }
    }

    pub fn with_keepalive(self, keepalive: KeepaliveConfig) -> Self {
        Self {
            keepalive: Some(keepalive),
            ..self
        }
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

    pub fn with_connection_type(self, connection_type: ConnType) -> Self {
        Self {
            connection_type,
            ..self
        }
    }

    /// Resolve the transport protocol for this configuration by inspecting
    /// the endpoint URI scheme. See [`TransportProtocol::from_endpoint`].
    pub fn resolved_transport(&self) -> TransportProtocol {
        TransportProtocol::from_endpoint(&self.endpoint)
    }
}

// Native surface. The browser build connects out over `wss://` (TLS handled by
// the browser) without auth/TLS/proxy/compression/backoff layers, so these
// builders, the shared connect-retry helper, and the gRPC-capable `to_channel`
// only exist off wasm32.
#[cfg(not(target_arch = "wasm32"))]
impl ClientConfig {
    pub fn merge_server_requirements(
        &mut self,
        server: &ServerConnectionConfig,
    ) -> Result<(), TlsConfigError> {
        self.endpoint = server.endpoint.clone();
        self.tls_setting.insecure = !server.tls_required;

        match &server.auth_method {
            RequiredAuthMethod::None => {
                self.auth = AuthenticationConfig::None;
            }
            RequiredAuthMethod::Spire { trust_domain } => {
                use crate::auth::spire::SpireConfig;
                use crate::tls::common::{CaSource, TlsSource};

                let trust_domains = trust_domain
                    .as_ref()
                    .map(|td| vec![td.clone()])
                    .unwrap_or_default();

                if server.tls_required {
                    // outbound_clients: socket comes from auth.spire only (not tls.source).
                    let socket_path = match &self.auth {
                        AuthenticationConfig::Spire(auth_cfg) => auth_cfg.socket_path.clone(),
                        _ => None,
                    };
                    self.tls_setting.config.source = TlsSource::Spire {
                        config: SpireConfig {
                            socket_path: socket_path.clone(),
                            ..Default::default()
                        },
                    };
                    self.tls_setting.config.ca_source = CaSource::Spire {
                        config: SpireConfig {
                            socket_path,
                            trust_domains,
                            ..Default::default()
                        },
                    };
                } else {
                    return Err(TlsConfigError::Spire(
                        "TLS needs to be enabled to use Spire".to_string(),
                    ));
                }
            }
            RequiredAuthMethod::Basic => {}
            RequiredAuthMethod::Jwt => {}
        }
        if let Some(ms) = server.timeout {
            self.connect_timeout = Duration::from_millis(ms as u64).into();
        }
        if let Some(ms) = server.backoff {
            self.backoff =
                BackoffConfig::new_fixed_interval(Duration::from_millis(ms as u64), usize::MAX);
        }
        Ok(())
    }

    pub fn with_compression(self, compression: CompressionType) -> Self {
        Self {
            compression: Some(compression),
            ..self
        }
    }

    pub fn with_tls_setting(self, tls_setting: TLSSetting) -> Self {
        Self {
            tls_setting,
            ..self
        }
    }

    pub fn with_proxy(self, proxy: ProxyConfig) -> Self {
        Self { proxy, ..self }
    }

    pub fn with_auth(self, auth: AuthenticationConfig) -> Self {
        Self { auth, ..self }
    }

    pub fn with_backoff(self, backoff: BackoffConfig) -> Self {
        Self { backoff, ..self }
    }

    pub fn with_metadata(self, metadata: MetadataMap) -> Self {
        Self {
            metadata: Some(metadata),
            ..self
        }
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
        RetryIf::start(strategy, attempt, |e: &ConfigError| {
            let retry = e.is_retryable_connect_error();
            if retry {
                tracing::warn!(error = %e.chain(), "transient connect error, retrying");
            } else {
                tracing::error!(error = %e.chain(), "non-retryable connect error");
            }
            retry
        })
        .await
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
}

// Browser surface: gRPC is unavailable on wasm, so only the WebSocket transport
// is supported.
#[cfg(target_arch = "wasm32")]
impl ClientConfig {
    /// Build a transport channel (browser build). gRPC is unavailable on wasm,
    /// so only the WebSocket transport is supported; a `ws://`/`wss://` endpoint
    /// is required. The `Grpc` generic is filled with [`std::convert::Infallible`]
    /// so callers can still pattern-match the (uninhabited) gRPC arm.
    pub async fn to_channel(
        &self,
    ) -> Result<TransportChannel<std::convert::Infallible>, ConfigError> {
        match self.resolved_transport() {
            TransportProtocol::Websocket => Ok(TransportChannel::Websocket(
                self.to_websocket_channel().await?,
            )),
            TransportProtocol::Grpc => Err(ConfigError::WebSocketClientUnsupportedTransport),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequiredAuthMethod {
    #[default]
    None,
    Basic,
    Jwt,
    Spire {
        trust_domain: Option<String>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ServerConnectionConfig {
    pub endpoint: String,
    pub tls_required: bool,
    pub auth_method: RequiredAuthMethod,
    pub timeout: Option<u32>,
    pub backoff: Option<u32>,
}

impl ServerConnectionConfig {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_client_config(client: &ClientConfig) -> Self {
        let tls_required = !client.tls_setting.insecure;
        let auth_method = match &client.auth {
            AuthenticationConfig::None => RequiredAuthMethod::None,
            AuthenticationConfig::Basic(_) => RequiredAuthMethod::Basic,
            AuthenticationConfig::StaticJwt(_) | AuthenticationConfig::Jwt(_) => {
                RequiredAuthMethod::Jwt
            }
            #[cfg(not(target_family = "windows"))]
            AuthenticationConfig::Spire(cfg) => RequiredAuthMethod::Spire {
                trust_domain: cfg.trust_domains.first().cloned(),
            },
        };
        Self {
            endpoint: client.endpoint.clone(),
            tls_required,
            auth_method,
            timeout: None,
            backoff: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn from_client_config(client: &ClientConfig) -> Self {
        Self {
            endpoint: client.endpoint.clone(),
            tls_required: false,
            auth_method: RequiredAuthMethod::None,
            timeout: None,
            backoff: None,
        }
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

    #[test]
    fn test_merge_server_requirements_none_auth() {
        let mut client = ClientConfig::with_endpoint("http://old:1234")
            .with_auth(AuthenticationConfig::Basic(Default::default()));
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: false,
            auth_method: RequiredAuthMethod::None,
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        assert_eq!(client.endpoint, "http://new:5678");
        assert!(client.tls_setting.insecure);
        assert_eq!(client.auth, AuthenticationConfig::None);
    }

    #[test]
    fn test_merge_server_requirements_basic_auth_preserved() {
        let basic = AuthenticationConfig::Basic(Default::default());
        let mut client = ClientConfig::with_endpoint("http://old:1234").with_auth(basic.clone());
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: true,
            auth_method: RequiredAuthMethod::Basic,
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        assert_eq!(client.endpoint, "http://new:5678");
        assert!(!client.tls_setting.insecure);
        assert_eq!(client.auth, basic);
    }

    #[test]
    fn test_merge_server_requirements_jwt_auth_preserved() {
        let jwt = AuthenticationConfig::StaticJwt(Default::default());
        let mut client = ClientConfig::with_endpoint("http://old:1234").with_auth(jwt.clone());
        let server = ServerConnectionConfig {
            endpoint: "http://new:9999".to_string(),
            tls_required: false,
            auth_method: RequiredAuthMethod::Jwt,
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        assert_eq!(client.endpoint, "http://new:9999");
        assert_eq!(client.auth, jwt);
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_merge_server_requirements_spire_with_tls_sets_sources() {
        use crate::tls::common::{CaSource, TlsSource};

        let mut client = ClientConfig::with_endpoint("http://old:1234");
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: true,
            auth_method: RequiredAuthMethod::Spire {
                trust_domain: Some("example.org".to_string()),
            },
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        assert_eq!(client.endpoint, "http://new:5678");
        assert!(!client.tls_setting.insecure);
        assert!(matches!(
            client.tls_setting.config.source,
            TlsSource::Spire { .. }
        ));
        assert!(matches!(
            client.tls_setting.config.ca_source,
            CaSource::Spire { .. }
        ));
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_merge_server_requirements_spire_auth_socket_path_propagates_to_tls() {
        use crate::auth::spire::SpireConfig;
        use crate::tls::common::{CaSource, TlsSource};

        let mut client = ClientConfig::with_endpoint("https://127.0.0.1:46201").with_auth(
            AuthenticationConfig::Spire(
                SpireConfig::new()
                    .with_socket_path("/run/spire/agent.sock")
                    .with_trust_domain("example.org"),
            ),
        );
        let server = ServerConnectionConfig {
            endpoint: "https://127.0.0.1:46201".to_string(),
            tls_required: true,
            auth_method: RequiredAuthMethod::Spire {
                trust_domain: Some("example.org".to_string()),
            },
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        if let TlsSource::Spire { config } = &client.tls_setting.config.source {
            assert_eq!(config.socket_path.as_deref(), Some("/run/spire/agent.sock"));
        } else {
            panic!("expected TlsSource::Spire");
        }
        if let CaSource::Spire { config } = &client.tls_setting.config.ca_source {
            assert_eq!(config.socket_path.as_deref(), Some("/run/spire/agent.sock"));
            assert_eq!(config.trust_domains, vec!["example.org"]);
        } else {
            panic!("expected CaSource::Spire");
        }
        if let AuthenticationConfig::Spire(auth) = &client.auth {
            assert_eq!(auth.socket_path.as_deref(), Some("/run/spire/agent.sock"));
        } else {
            panic!("expected AuthenticationConfig::Spire");
        }
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_merge_server_requirements_spire_ignores_tls_socket_path() {
        use crate::auth::spire::SpireConfig;
        use crate::tls::common::{CaSource, TlsSource};

        let mut client = ClientConfig::with_endpoint("http://old:1234");
        client.tls_setting.config.source = TlsSource::Spire {
            config: SpireConfig {
                socket_path: Some("/run/spire.sock".to_string()),
                ..Default::default()
            },
        };
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: true,
            auth_method: RequiredAuthMethod::Spire {
                trust_domain: Some("example.org".to_string()),
            },
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        if let TlsSource::Spire { config } = &client.tls_setting.config.source {
            assert_eq!(config.socket_path, None);
        } else {
            panic!("expected TlsSource::Spire");
        }
        if let CaSource::Spire { config } = &client.tls_setting.config.ca_source {
            assert_eq!(config.socket_path, None);
            assert_eq!(config.trust_domains, vec!["example.org"]);
        } else {
            panic!("expected CaSource::Spire");
        }
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_merge_server_requirements_spire_no_trust_domain() {
        use crate::tls::common::CaSource;

        let mut client = ClientConfig::with_endpoint("http://old:1234");
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: true,
            auth_method: RequiredAuthMethod::Spire { trust_domain: None },
            ..Default::default()
        };
        client.merge_server_requirements(&server).unwrap();
        if let CaSource::Spire { config } = &client.tls_setting.config.ca_source {
            assert!(config.trust_domains.is_empty());
        } else {
            panic!("expected CaSource::Spire");
        }
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_merge_server_requirements_spire_without_tls_errors() {
        let mut client = ClientConfig::with_endpoint("http://old:1234");
        let server = ServerConnectionConfig {
            endpoint: "http://new:5678".to_string(),
            tls_required: false,
            auth_method: RequiredAuthMethod::Spire {
                trust_domain: Some("example.org".to_string()),
            },
            ..Default::default()
        };
        assert!(client.merge_server_requirements(&server).is_err());
    }

    #[test]
    fn test_from_client_config_none_auth() {
        let client = ClientConfig::with_endpoint("http://host:1234")
            .with_tls_setting(TLSSetting::insecure());
        let server = ServerConnectionConfig::from_client_config(&client);
        assert_eq!(server.endpoint, "http://host:1234");
        assert!(!server.tls_required);
        assert_eq!(server.auth_method, RequiredAuthMethod::None);
        assert!(server.timeout.is_none());
        assert!(server.backoff.is_none());
    }

    #[test]
    fn test_from_client_config_basic_auth() {
        let client = ClientConfig::with_endpoint("http://host:1234")
            .with_auth(AuthenticationConfig::Basic(Default::default()));
        let server = ServerConnectionConfig::from_client_config(&client);
        assert_eq!(server.auth_method, RequiredAuthMethod::Basic);
    }

    #[test]
    fn test_from_client_config_jwt_auth() {
        let client = ClientConfig::with_endpoint("http://host:1234")
            .with_auth(AuthenticationConfig::StaticJwt(Default::default()));
        let server = ServerConnectionConfig::from_client_config(&client);
        assert_eq!(server.auth_method, RequiredAuthMethod::Jwt);
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_from_client_config_spire_auth() {
        use crate::auth::spire::SpireConfig;
        let spire = SpireConfig {
            trust_domains: vec!["example.org".to_string()],
            ..Default::default()
        };
        let client = ClientConfig::with_endpoint("https://host:1234")
            .with_auth(AuthenticationConfig::Spire(spire));
        let server = ServerConnectionConfig::from_client_config(&client);
        assert_eq!(
            server.auth_method,
            RequiredAuthMethod::Spire {
                trust_domain: Some("example.org".to_string())
            }
        );
        assert!(server.tls_required);
    }
}

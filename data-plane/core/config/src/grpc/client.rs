// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_string::DurationString;
use rustls_pki_types::ServerName;

use std::time::Duration;
use std::{collections::HashMap, str::FromStr};
use tower::ServiceExt;

use base64::prelude::*;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper_rustls;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::proxy::matcher::Intercept;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tonic::codegen::{Body, Bytes, StdError};
use tonic::transport::{Channel, Uri};
use tracing::warn;

use slim_auth::metadata::MetadataMap;

use super::compression::CompressionType;
use super::errors::ConfigError;
use super::headers_middleware::SetRequestHeaderLayer;
use crate::auth::ClientAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
use crate::auth::static_jwt::Config as BearerAuthenticationConfig;
use crate::component::configuration::{Configuration, ConfigurationError};
use crate::grpc::proxy::ProxyConfig;
use crate::tls::{client::TlsClientConfig as TLSSetting, common::RustlsConfigLoader};

/// Creates an HTTPS connector with optional SNI based on the origin
fn https_connector<S>(
    s: S,
    tls: &rustls::ClientConfig,
    server_name: Option<String>,
) -> hyper_rustls::HttpsConnector<S>
where
    S: tower::Service<Uri>,
{
    let tls = tls.clone();
    let mut builder = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http();

    if let Some(origin_str) = server_name {
        builder =
            builder.with_server_name_resolver(move |_: &_| ServerName::try_from(origin_str.clone()))
    }

    builder.enable_http2().wrap_connector(s)
}

/// Macro to create TLS-enabled or plain connectors based on TLS configuration,
/// applying the optional origin (for SNI) when TLS is enabled.
macro_rules! create_connector {
    ($builder:expr, $base_connector:expr, $tls_config:expr, $server_name:expr) => {
        match $tls_config {
            Some(tls) => {
                let connector = tower::ServiceBuilder::new()
                    .layer_fn(move |s| {
                        https_connector(s, &tls, $server_name.map(|s| s.to_string()))
                    })
                    .service($base_connector);
                Ok($builder.connect_with_connector_lazy(connector))
            }
            None => Ok($builder.connect_with_connector_lazy($base_connector)),
        }
    };
}

/// Macro to create authenticated service layers for auth types that don't need initialization
macro_rules! create_auth_service_no_init {
    ($self:expr, $auth_config:expr, $header_map:expr, $channel:expr) => {{
        let auth_layer = $auth_config
            .get_client_layer()
            .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

        $self.warn_insecure_auth();

        Ok(tower::ServiceBuilder::new()
            .layer(SetRequestHeaderLayer::new($header_map))
            .layer(auth_layer)
            .service($channel)
            .boxed_clone())
    }};
}

/// Macro to create authenticated service layers for auth types that need initialization
macro_rules! create_auth_service_with_init {
    ($self:expr, $auth_config:expr, $header_map:expr, $channel:expr) => {{
        let mut auth_layer = $auth_config
            .get_client_layer()
            .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

        // Initialize the auth layer
        auth_layer.initialize().await.map_err(|e| {
            ConfigError::AuthConfigError(format!("Failed to initialize auth layer: {}", e))
        })?;

        $self.warn_insecure_auth();

        Ok(tower::ServiceBuilder::new()
            .layer(SetRequestHeaderLayer::new($header_map))
            .layer(auth_layer)
            .service($channel)
            .boxed_clone())
    }};
}

/// Enum to handle all connection types: direct connections and proxy tunnels
enum ConnectionType {
    /// Direct HTTP connection without proxy
    Direct(HttpConnector),
    /// HTTP proxy tunnel connection
    ProxyHttp(Tunnel<HttpConnector>),
    /// HTTPS proxy tunnel connection
    ProxyHttps(Tunnel<hyper_rustls::HttpsConnector<HttpConnector>>),
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
    /// None
    #[default]
    None,
}

/// Struct for the client configuration.
/// This struct contains the endpoint, origin, compression type, rate limit,
/// TLS settings, keepalive settings, proxy settings, timeout settings, buffer size settings,
/// headers, and auth settings.
/// The client configuration can be converted to a tonic channel.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ClientConfig {
    /// The target the client will connect to.
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

    /// Arbitrary user-provided metadata.
    pub metadata: Option<MetadataMap>,
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
            metadata: None,
        }
    }
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
            "ClientConfig {{ endpoint: {}, origin: {:?}, server_name: {:?}, compression: {:?}, rate_limit: {:?}, tls_setting: {:?}, keepalive: {:?}, proxy: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, auth: {:?}, metadata: {:?} }}",
            self.endpoint,
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
            self.metadata
        )
    }
}

impl Configuration for ClientConfig {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate the client configuration
        self.tls_setting.validate()
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

    pub fn with_metadata(self, metadata: MetadataMap) -> Self {
        Self {
            metadata: Some(metadata),
            ..self
        }
    }

    /// Converts the client configuration to a tonic channel.
    /// This function will return a Result with the channel if the configuration is valid.
    /// If the configuration is invalid, it will return a ConfigError.
    /// The function will set the headers, tls settings, keepalive settings, rate limit settings
    /// timeout settings, buffer size settings, and origin settings.
    pub async fn to_channel(
        &self,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
        ConfigError,
    > {
        // Validate endpoint
        self.validate_endpoint()?;

        // Parse endpoint URI
        let uri = self.parse_endpoint_uri()?;

        // Create and configure HTTP connector
        let http_connector = self.create_http_connector()?;

        // Create channel builder with all settings
        let builder = self.create_channel_builder(uri.clone())?;

        // Parse headers
        let header_map = self.parse_headers()?;

        // Load TLS configuration
        let tls_config = self.load_tls_config().await?;

        // Create the channel with appropriate connector
        let channel = self
            .create_channel_with_connector(uri, builder, http_connector, tls_config)
            .await?;

        // Apply authentication and headers
        self.apply_auth_and_headers(channel, header_map).await
    }

    /// Validates that the endpoint is set and not empty
    fn validate_endpoint(&self) -> Result<(), ConfigError> {
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }
        Ok(())
    }

    /// Parses the endpoint string into a URI
    fn parse_endpoint_uri(&self) -> Result<Uri, ConfigError> {
        Uri::from_str(&self.endpoint).map_err(|e| ConfigError::UriParseError(e.to_string()))
    }

    /// Creates and configures the HTTP connector
    fn create_http_connector(&self) -> Result<HttpConnector, ConfigError> {
        let mut http = HttpConnector::new();

        // NOTE(msardara): we might want to make these configurable as well.
        http.enforce_http(false);
        http.set_nodelay(false);

        // set the connection timeout
        match self.connect_timeout.as_secs() {
            0 => http.set_connect_timeout(None),
            _ => http.set_connect_timeout(Some(self.connect_timeout.into())),
        }

        // set keepalive settings
        if let Some(keepalive) = &self.keepalive {
            http.set_keepalive(Some(keepalive.tcp_keepalive.into()));
        }

        Ok(http)
    }

    /// Creates the channel builder with all configuration settings
    fn create_channel_builder(&self, uri: Uri) -> Result<tonic::transport::Endpoint, ConfigError> {
        let mut builder = Channel::builder(uri);

        // set the buffer size
        if let Some(size) = self.buffer_size {
            builder = builder.buffer_size(size);
        }

        // set keepalive settings
        if let Some(keepalive) = &self.keepalive {
            builder = builder
                .keep_alive_timeout(keepalive.timeout.into())
                .keep_alive_while_idle(keepalive.keep_alive_while_idle)
                // HTTP level keepalive
                .http2_keep_alive_interval(keepalive.http2_keepalive.into());
        }

        // set origin settings
        if let Some(origin) = &self.origin {
            let origin_uri = Uri::from_str(origin.as_str())
                .map_err(|e| ConfigError::UriParseError(e.to_string()))?;
            builder = builder.origin(origin_uri);
        }

        // set rate limit settings
        if let Some(rate_limit) = &self.rate_limit {
            let (limit, duration) = parse_rate_limit(rate_limit)
                .map_err(|e| ConfigError::RateLimitParseError(e.to_string()))?;
            builder = builder.rate_limit(limit, duration);
        }

        // set the request timeout
        if self.request_timeout.as_secs() > 0 {
            builder = builder.timeout(self.request_timeout.into());
        }

        Ok(builder)
    }

    /// Parses headers from the configuration
    fn parse_headers(&self) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(&self.headers, "header")
    }

    /// Generic helper to parse a HashMap<String, String> into HeaderMap
    fn parse_header_map(
        headers: &HashMap<String, String>,
        context: &str,
    ) -> Result<HeaderMap, ConfigError> {
        let mut header_map = HeaderMap::new();
        for (key, value) in headers {
            let header_name = HeaderName::from_str(key).map_err(|_| {
                ConfigError::HeaderParseError(format!("Invalid {} name: {}", context, key))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|_| {
                ConfigError::HeaderParseError(format!("Invalid {} value: {}", context, value))
            })?;
            header_map.insert(header_name, header_value);
        }
        Ok(header_map)
    }

    /// Helper to create basic auth header for proxy authentication
    fn create_proxy_auth_header(
        username: &str,
        password: &str,
    ) -> Result<HeaderValue, ConfigError> {
        let auth_value = BASE64_STANDARD.encode(format!("{}:{}", username, password));
        HeaderValue::from_str(&format!("Basic {}", auth_value)).map_err(|_| {
            ConfigError::HeaderParseError("Invalid proxy auth credentials".to_string())
        })
    }

    /// Helper to apply authentication and headers to a tunnel
    fn apply_tunnel_config<T>(
        &self,
        mut tunnel: Tunnel<T>,
        proxy_config: &ProxyConfig,
        warn_insecure: bool,
    ) -> Result<Tunnel<T>, ConfigError> {
        // Set proxy authentication if provided
        if let (Some(username), Some(password)) = (&proxy_config.username, &proxy_config.password) {
            if warn_insecure {
                self.warn_insecure_auth();
            }

            let auth_header = Self::create_proxy_auth_header(username, password)?;
            tunnel = tunnel.with_auth(auth_header);
        }

        // Set custom headers for proxy requests
        if !proxy_config.headers.is_empty() {
            let proxy_headers = self.parse_proxy_headers(&proxy_config.headers)?;
            tunnel = tunnel.with_headers(proxy_headers);
        }

        Ok(tunnel)
    }

    /// Loads TLS configuration
    async fn load_tls_config(&self) -> Result<Option<rustls::ClientConfig>, ConfigError> {
        self.tls_setting
            .load_rustls_config()
            .await
            .map_err(|e| ConfigError::TLSSettingError(e.to_string()))
    }

    /// Creates the channel with the appropriate connector (proxy or direct)
    async fn create_channel_with_connector(
        &self,
        uri: Uri,
        builder: tonic::transport::Endpoint,
        http_connector: HttpConnector,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        // Create the appropriate connection type
        let connection = self.create_connection(uri, http_connector).await?;

        // Apply TLS and create the channel
        self.create_channel_from_connection(builder, connection, tls_config)
    }

    /// Creates the appropriate connection type based on proxy configuration
    async fn create_connection(
        &self,
        uri: Uri,
        http_connector: HttpConnector,
    ) -> Result<ConnectionType, ConfigError> {
        // Check if this host should bypass the proxy
        if let Some(intercept) = self.proxy.should_use_proxy(uri.to_string()) {
            // Use proxy for this host
            self.create_proxy_connection(intercept, http_connector)
                .await
        } else {
            // Skip proxy for this host, use direct connection
            Ok(ConnectionType::Direct(http_connector))
        }
    }

    /// Creates a proxy connection
    async fn create_proxy_connection(
        &self,
        intercept: Intercept,
        http_connector: HttpConnector,
    ) -> Result<ConnectionType, ConfigError> {
        let proxy_uri = intercept.uri();

        tracing::info!("Creating proxy tunnel to {}", proxy_uri);

        // Check if the proxy URL uses HTTPS
        if proxy_uri.scheme_str() == Some("https") {
            let proxy_tls_config = self
                .proxy
                .tls_setting
                .load_rustls_config()
                .await
                .map_err(|e| {
                    ConfigError::TLSSettingError(format!("Failed to load proxy TLS config: {}", e))
                })?
                .unwrap();

            // Create HTTPS connector for the proxy itself
            let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(proxy_tls_config)
                .https_or_http()
                .enable_http2()
                .wrap_connector(http_connector);

            let tunnel = Tunnel::new(proxy_uri.clone(), https_connector);
            let configured_tunnel = self.apply_tunnel_config(tunnel, &self.proxy, false)?;

            Ok(ConnectionType::ProxyHttps(configured_tunnel))
        } else {
            // Use HTTP connector for the proxy
            let tunnel = Tunnel::new(proxy_uri.clone(), http_connector);
            let configured_tunnel = self.apply_tunnel_config(tunnel, &self.proxy, true)?;

            Ok(ConnectionType::ProxyHttp(configured_tunnel))
        }
    }

    /// Creates a channel from any connection type with TLS support
    fn create_channel_from_connection(
        &self,
        builder: tonic::transport::Endpoint,
        connection: ConnectionType,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        match connection {
            ConnectionType::Direct(connector) => {
                create_connector!(builder, connector, tls_config, self.server_name.as_deref())
            }
            ConnectionType::ProxyHttp(tunnel) => {
                create_connector!(builder, tunnel, tls_config, self.server_name.as_deref())
            }
            ConnectionType::ProxyHttps(tunnel) => {
                create_connector!(builder, tunnel, tls_config, self.server_name.as_deref())
            }
        }
    }

    /// Parses proxy headers
    fn parse_proxy_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(headers, "proxy header")
    }

    /// Applies authentication and headers to the channel
    async fn apply_auth_and_headers(
        &self,
        channel: Channel,
        header_map: HeaderMap,
    ) -> Result<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + std::marker::Send>
                              + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
        ConfigError,
    > {
        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                create_auth_service_no_init!(self, basic, header_map, channel)
            }
            AuthenticationConfig::StaticJwt(jwt) => {
                create_auth_service_with_init!(self, jwt, header_map, channel)
            }
            AuthenticationConfig::Jwt(jwt) => {
                create_auth_service_with_init!(self, jwt, header_map, channel)
            }
            AuthenticationConfig::None => Ok(tower::ServiceBuilder::new()
                .layer(SetRequestHeaderLayer::new(header_map))
                .service(channel)
                .boxed_clone()),
        }
    }

    /// Warns if authentication is enabled without TLS
    fn warn_insecure_auth(&self) {
        if self.tls_setting.insecure {
            warn!("Auth is enabled without TLS. This is not recommended.");
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
        println!("{}", s);
        let deser: ClientConfig = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(deser.metadata, Some(md));
    }
}

/// Parse the rate limit string into a limit and a duration.
/// The rate limit string should be in the format of <limit>/<duration>,
/// with duration expressed in seconds.
/// This function will return a Result with the limit and duration if the
/// rate limit is valid.
fn parse_rate_limit(rate_limit: &str) -> Result<(u64, Duration), ConfigError> {
    let parts: Vec<&str> = rate_limit.split('/').collect();

    // Check the parts has two elements
    if parts.len() != 2 {
        return Err(
            ConfigError::RateLimitParseError(
                "rate limit should be in the format of <limit>/<duration>, with duration expressed in seconds".to_string(),
            ),
        );
    }

    let limit = parts[0]
        .parse::<u64>()
        .map_err(|e| ConfigError::RateLimitParseError(e.to_string()))?;
    let duration = Duration::from_secs(
        parts[1]
            .parse::<u64>()
            .map_err(|e| ConfigError::RateLimitParseError(e.to_string()))?,
    );
    Ok((limit, duration))
}

#[cfg(test)]
mod test {
    #[allow(unused_imports)]
    use super::*;
    use crate::tls::common::CaSource;
    use tracing::debug;
    use tracing_test::traced_test;

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
    fn test_parse_rate_limit() {
        let res = parse_rate_limit("100/10");
        assert!(res.is_ok());

        let (limit, duration) = res.unwrap();

        assert_eq!(limit, 100);
        assert_eq!(duration, Duration::from_secs(10));

        let res = parse_rate_limit("100");
        assert!(res.is_err());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_to_channel() {
        let test_path: &str = env!("CARGO_MANIFEST_DIR");

        // create a new client config
        let mut client = ClientConfig::default();

        // as the endpoint is missing, this should fail
        let mut channel = client.to_channel().await;
        assert!(channel.is_err());

        // Set the endpoint
        client.endpoint = "http://localhost:8080".to_string();
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set the tls settings
        client.tls_setting.insecure = true;
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set the tls settings
        client.tls_setting = {
            let mut tls = TLSSetting::default();
            // Updated for new Config fields: set CA via ca_source and leave source as default (None)
            tls.config.ca_source = CaSource::File {
                path: format!("{}/testdata/grpc/{}", test_path, "ca.crt"),
            };
            tls.insecure = false;
            tls
        };
        debug!("{}/testdata/{}", test_path, "ca.crt");
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set keepalive settings
        client.keepalive = Some(KeepaliveConfig::default());
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set rate limit settings
        client.rate_limit = Some("100/10".to_string());
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set rate limit settings wrong
        client.rate_limit = Some("100".to_string());
        channel = client.to_channel().await;
        assert!(channel.is_err());

        // reset config
        client.rate_limit = None;

        // Set timeout settings
        client.request_timeout = Duration::from_secs(10).into();
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set buffer size settings
        client.buffer_size = Some(1024);
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set origin settings
        client.origin = Some("http://example.com".to_string());
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // set additional header to add to the request
        client
            .headers
            .insert("X-Test".to_string(), "test".to_string());
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set proxy settings
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080");
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set proxy with authentication
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set proxy with headers
        let mut proxy_headers = HashMap::new();
        proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("http://proxy.example.com:8080").with_headers(proxy_headers);
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy settings
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080");
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy with authentication
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_channel().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy with headers
        let mut https_proxy_headers = HashMap::new();
        https_proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("https://proxy.example.com:8080").with_headers(https_proxy_headers);
        channel = client.to_channel().await;
        assert!(channel.is_ok());
    }

    #[test]
    fn test_client_config_with_proxy() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
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
}

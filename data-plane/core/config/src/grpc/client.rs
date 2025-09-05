// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_str::deserialize_duration;
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

/// Macro to create TLS-enabled or plain connectors based on TLS configuration
macro_rules! create_connector {
    ($builder:expr, $base_connector:expr, $tls_config:expr) => {
        match $tls_config {
            Some(tls) => {
                let connector = tower::ServiceBuilder::new()
                    .layer_fn(move |s| {
                        let tls = tls.clone();
                        hyper_rustls::HttpsConnectorBuilder::new()
                            .with_tls_config(tls)
                            .https_or_http()
                            .enable_http2()
                            .wrap_connector(s)
                    })
                    .service($base_connector);

                Ok($builder.connect_with_connector_lazy(connector))
            }
            None => Ok($builder.connect_with_connector_lazy($base_connector)),
        }
    };
}

/// Macro to create authenticated service layers for different auth types
macro_rules! create_auth_service {
    ($self:expr, $auth_config:expr, $header_map:expr, $channel:expr) => {{
        let auth_layer = $auth_config
            .get_client_layer()
            .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

        $self.warn_insecure_auth();

        Ok(tower::ServiceBuilder::new()
            .layer(SetRequestHeaderLayer::new($header_map))
            .layer(auth_layer)
            .service($channel)
            .boxed())
    }};
}

use super::compression::CompressionType;
use super::errors::ConfigError;
use super::headers_middleware::SetRequestHeaderLayer;
use crate::auth::ClientAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::bearer::Config as BearerAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
use crate::component::configuration::{Configuration, ConfigurationError};
use crate::grpc::proxy::ProxyConfig;
use crate::tls::{client::TlsClientConfig as TLSSetting, common::RustlsConfigLoader};

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
    #[serde(
        default = "default_tcp_keepalive",
        deserialize_with = "deserialize_duration"
    )]
    #[schemars(with = "String")]
    pub tcp_keepalive: Duration,

    /// The duration of the keepalive time for HTTP2
    #[serde(
        default = "default_http2_keepalive",
        deserialize_with = "deserialize_duration"
    )]
    #[schemars(with = "String")]
    pub http2_keepalive: Duration,

    /// The timeout duration for the keepalive
    #[serde(default = "default_timeout", deserialize_with = "deserialize_duration")]
    #[schemars(with = "String")]
    pub timeout: Duration,

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

fn default_tcp_keepalive() -> Duration {
    Duration::from_secs(60)
}

fn default_http2_keepalive() -> Duration {
    Duration::from_secs(60)
}

fn default_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_keep_alive_while_idle() -> bool {
    false
}

/// Enum holding one configuration for the client.
#[derive(Debug, Serialize, Default, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// Bearer authentication configuration.
    Bearer(BearerAuthenticationConfig),
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

    /// Origin for the client.
    pub origin: Option<String>,

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
    #[serde(
        default = "default_connect_timeout",
        deserialize_with = "deserialize_duration"
    )]
    #[schemars(with = "String")]
    pub connect_timeout: Duration,

    /// Timeout per request.
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "deserialize_duration"
    )]
    #[schemars(with = "String")]
    pub request_timeout: Duration,

    /// ReadBufferSize.
    pub buffer_size: Option<usize>,

    /// The headers associated with gRPC requests.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Auth configuration for outgoing RPCs.
    #[serde(default)]
    // #[serde(with = "serde_yaml::with::singleton_map")]
    pub auth: AuthenticationConfig,
}

/// Defaults for ClientConfig
impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            endpoint: String::new(),
            origin: None,
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
        }
    }
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(0)
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(0)
}

// Display for ClientConfig
impl std::fmt::Display for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ClientConfig {{ endpoint: {}, origin: {:?}, compression: {:?}, rate_limit: {:?}, tls_setting: {:?}, keepalive: {:?}, proxy: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, auth: {:?} }}",
            self.endpoint,
            self.origin,
            self.compression,
            self.rate_limit,
            self.tls_setting,
            self.keepalive,
            self.proxy,
            self.connect_timeout,
            self.request_timeout,
            self.buffer_size,
            self.headers,
            self.auth
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
            connect_timeout,
            ..self
        }
    }

    pub fn with_request_timeout(self, request_timeout: Duration) -> Self {
        Self {
            request_timeout,
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

    /// Converts the client configuration to a tonic channel.
    /// This function will return a Result with the channel if the configuration is valid.
    /// If the configuration is invalid, it will return a ConfigError.
    /// The function will set the headers, tls settings, keepalive settings, rate limit settings
    /// timeout settings, buffer size settings, and origin settings.
    pub fn to_channel(
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
        + use<>,
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
        let tls_config = self.load_tls_config()?;

        // Create the channel with appropriate connector
        let channel =
            self.create_channel_with_connector(uri, builder, http_connector, tls_config)?;

        // Apply authentication and headers
        self.apply_auth_and_headers(channel, header_map)
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
            _ => http.set_connect_timeout(Some(self.connect_timeout)),
        }

        // set keepalive settings
        if let Some(keepalive) = &self.keepalive {
            http.set_keepalive(Some(keepalive.tcp_keepalive));
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
                .keep_alive_timeout(keepalive.timeout)
                .keep_alive_while_idle(keepalive.keep_alive_while_idle)
                // HTTP level keepalive
                .http2_keep_alive_interval(keepalive.http2_keepalive);
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
            builder = builder.timeout(self.request_timeout);
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
    fn load_tls_config(&self) -> Result<Option<rustls::ClientConfig>, ConfigError> {
        TLSSetting::load_rustls_config(&self.tls_setting)
            .map_err(|e| ConfigError::TLSSettingError(e.to_string()))
    }

    /// Creates the channel with the appropriate connector (proxy or direct)
    fn create_channel_with_connector(
        &self,
        uri: Uri,
        builder: tonic::transport::Endpoint,
        http_connector: HttpConnector,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        // Create the appropriate connection type
        let connection = self.create_connection(uri, http_connector)?;

        // Apply TLS and create the channel
        self.create_channel_from_connection(builder, connection, tls_config)
    }

    /// Creates the appropriate connection type based on proxy configuration
    fn create_connection(
        &self,
        uri: Uri,
        http_connector: HttpConnector,
    ) -> Result<ConnectionType, ConfigError> {
        // Check if this host should bypass the proxy
        if let Some(intercept) = self.proxy.should_use_proxy(uri.to_string()) {
            // Use proxy for this host
            self.create_proxy_connection(intercept, http_connector)
        } else {
            // Skip proxy for this host, use direct connection
            Ok(ConnectionType::Direct(http_connector))
        }
    }

    /// Creates a proxy connection
    fn create_proxy_connection(
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
                create_connector!(builder, connector, tls_config)
            }
            ConnectionType::ProxyHttp(tunnel) => {
                create_connector!(builder, tunnel, tls_config)
            }
            ConnectionType::ProxyHttps(tunnel) => {
                create_connector!(builder, tunnel, tls_config)
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
    fn apply_auth_and_headers(
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
        + use<>,
        ConfigError,
    > {
        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                create_auth_service!(self, basic, header_map, channel)
            }
            AuthenticationConfig::Bearer(bearer) => {
                create_auth_service!(self, bearer, header_map, channel)
            }
            AuthenticationConfig::Jwt(jwt) => {
                create_auth_service!(self, jwt, header_map, channel)
            }
            AuthenticationConfig::None => Ok(tower::ServiceBuilder::new()
                .layer(SetRequestHeaderLayer::new(header_map))
                .service(channel)
                .boxed()),
        }
    }

    /// Warns if authentication is enabled without TLS
    fn warn_insecure_auth(&self) {
        if self.tls_setting.insecure {
            warn!("Auth is enabled without TLS. This is not recommended.");
        }
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
        let mut channel = client.to_channel();
        assert!(channel.is_err());

        // Set the endpoint
        client.endpoint = "http://localhost:8080".to_string();
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set the tls settings
        client.tls_setting.insecure = true;
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set the tls settings
        client.tls_setting = {
            let mut tls = TLSSetting::default();
            tls.config.ca_file = Some(format!("{}/testdata/grpc/{}", test_path, "ca.crt"));
            tls.insecure = false;
            tls
        };
        debug!("{}/testdata/{}", test_path, "ca.crt");
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set keepalive settings
        client.keepalive = Some(KeepaliveConfig::default());
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set rate limit settings
        client.rate_limit = Some("100/10".to_string());
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set rate limit settings wrong
        client.rate_limit = Some("100".to_string());
        channel = client.to_channel();
        assert!(channel.is_err());

        // reset config
        client.rate_limit = None;

        // Set timeout settings
        client.request_timeout = Duration::from_secs(10);
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set buffer size settings
        client.buffer_size = Some(1024);
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set origin settings
        client.origin = Some("http://example.com".to_string());
        channel = client.to_channel();
        assert!(channel.is_ok());

        // set additional header to add to the request
        client
            .headers
            .insert("X-Test".to_string(), "test".to_string());
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set proxy settings
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080");
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set proxy with authentication
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set proxy with headers
        let mut proxy_headers = HashMap::new();
        proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("http://proxy.example.com:8080").with_headers(proxy_headers);
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set HTTPS proxy settings
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080");
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set HTTPS proxy with authentication
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set HTTPS proxy with headers
        let mut https_proxy_headers = HashMap::new();
        https_proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("https://proxy.example.com:8080").with_headers(https_proxy_headers);
        channel = client.to_channel();
        assert!(channel.is_ok());
    }

    #[test]
    fn test_client_config_with_proxy() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
        let client = ClientConfig::with_endpoint("http://localhost:8080").with_proxy(proxy.clone());
        assert_eq!(client.proxy, proxy);
    }
}

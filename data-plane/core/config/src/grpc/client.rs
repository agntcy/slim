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
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tonic::codegen::{Body, Bytes, StdError};
use tonic::transport::{Channel, Uri};
use tracing::warn;

use super::compression::CompressionType;
use super::errors::ConfigError;
use super::headers_middleware::SetRequestHeaderLayer;
use crate::auth::ClientAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::bearer::Config as BearerAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
use crate::component::configuration::{Configuration, ConfigurationError};
use crate::tls::{client::TlsClientConfig as TLSSetting, common::RustlsConfigLoader};

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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProxyConfig {
    /// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
    pub url: String,

    /// Optional username for proxy authentication
    pub username: Option<String>,

    /// Optional password for proxy authentication
    pub password: Option<String>,

    /// Headers to send with proxy requests
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// List of hosts that should bypass the proxy.
    /// Based on hyper-utils matcher: https://github.com/hyperium/hyper-util/blob/master/src/client/proxy/matcher.rs
    #[serde(default)]
    pub no_proxy: Option<String>,
}

impl Clone for ProxyConfig {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            headers: self.headers.clone(),
            no_proxy: self.no_proxy.clone(),
        }
    }
}

impl PartialEq for ProxyConfig {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.username == other.username
            && self.password == other.password
            && self.headers == other.headers
            && self.no_proxy == other.no_proxy
    }
}

impl ProxyConfig {
    /// Creates a new proxy configuration with the given URL
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            username: None,
            password: None,
            headers: HashMap::new(),
            no_proxy: None,
        }
    }

    /// Sets the proxy authentication credentials
    pub fn with_auth(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }

    /// Sets additional headers for proxy requests
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Sets the no_proxy list - hosts that should bypass the proxy
    pub fn with_no_proxy(mut self, no_proxy: impl Into<String>) -> Self {
        self.no_proxy = Some(no_proxy.into());
        self
    }

    /// Checks if the given host should bypass the proxy based on no_proxy rules
    pub fn should_use_proxy(&self, uri: &str) -> Option<Intercept> {
        if let Some(no_proxy) = self.no_proxy.as_ref() {
            if no_proxy.is_empty() {
                return None;
            }
        } else {
            return None;
        }

        let no_proxy = self.no_proxy.as_ref().unwrap();

        // matcher builder
        let builder = Matcher::builder()
            .http(self.url.clone())
            .https(self.url.clone())
            .no(no_proxy.clone());

        let matcher = builder.build();

        // Convert string uri into http::Uri
        let dst = uri.parse::<http::Uri>().unwrap();

        // Check if this should bypass the proxy
        matcher.intercept(&dst)
    }
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
    pub proxy: Option<ProxyConfig>,

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
            proxy: None,
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
        Self {
            proxy: Some(proxy),
            ..self
        }
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
        let mut header_map = HeaderMap::new();
        for (key, value) in &self.headers {
            let k: HeaderName = key.parse().map_err(|_| {
                ConfigError::HeaderParseError(format!("error parsing header key {}", key))
            })?;
            let v: HeaderValue = value.parse().map_err(|_| {
                ConfigError::HeaderParseError(format!("error parsing header value {}", key))
            })?;

            header_map.insert(k, v);
        }
        Ok(header_map)
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
        match &self.proxy {
            Some(proxy_config) => self.create_channel_with_proxy(
                uri,
                builder,
                http_connector,
                tls_config,
                proxy_config,
            ),
            None => self.create_direct_channel(builder, http_connector, tls_config),
        }
    }

    /// Creates a channel with proxy configuration
    fn create_channel_with_proxy(
        &self,
        uri: Uri,
        builder: tonic::transport::Endpoint,
        http_connector: HttpConnector,
        tls_config: Option<rustls::ClientConfig>,
        proxy_config: &ProxyConfig,
    ) -> Result<Channel, ConfigError> {
        // Extract host from endpoint to check no_proxy rules
        let endpoint_host = uri.host().unwrap_or("");

        // Check if this host should bypass the proxy
        if let Some(intercept) = proxy_config.should_use_proxy(endpoint_host) {
            // Use proxy for this host
            let tunnel = self.create_proxy_tunnel(intercept, http_connector, proxy_config)?;
            self.apply_tls_to_tunnel_connector(builder, tunnel, tls_config)
        } else {
            // Skip proxy for this host, use direct connection
            self.create_direct_channel(builder, http_connector, tls_config)
        }
    }

    /// Creates a proxy tunnel connector
    fn create_proxy_tunnel(
        &self,
        intercept: Intercept,
        http_connector: HttpConnector,
        proxy_config: &ProxyConfig,
    ) -> Result<Tunnel<HttpConnector>, ConfigError> {
        let mut tunnel = Tunnel::new(intercept.uri().clone(), http_connector);

        // Set proxy authentication if provided
        if let (Some(username), Some(password)) = (&proxy_config.username, &proxy_config.password) {
            let auth_value = BASE64_STANDARD.encode(format!("{}:{}", username, password));
            let auth_header =
                HeaderValue::from_str(&format!("Basic {}", auth_value)).map_err(|_| {
                    ConfigError::HeaderParseError("Invalid proxy auth credentials".to_string())
                })?;
            tunnel = tunnel.with_auth(auth_header);
        }

        // Set custom headers for proxy requests
        if !proxy_config.headers.is_empty() {
            let proxy_headers = self.parse_proxy_headers(&proxy_config.headers)?;
            tunnel = tunnel.with_headers(proxy_headers);
        }

        Ok(tunnel)
    }

    /// Parses proxy headers
    fn parse_proxy_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> Result<HeaderMap, ConfigError> {
        let mut proxy_headers = HeaderMap::new();
        for (key, value) in headers {
            let header_name = HeaderName::from_str(key).map_err(|_| {
                ConfigError::HeaderParseError(format!("Invalid proxy header name: {}", key))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|_| {
                ConfigError::HeaderParseError(format!("Invalid proxy header value: {}", value))
            })?;
            proxy_headers.insert(header_name, header_value);
        }
        Ok(proxy_headers)
    }

    /// Creates a direct channel without proxy
    fn create_direct_channel(
        &self,
        builder: tonic::transport::Endpoint,
        http_connector: HttpConnector,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        match tls_config {
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
                    .service(http_connector);

                Ok(builder.connect_with_connector_lazy(connector))
            }
            None => Ok(builder.connect_with_connector_lazy(http_connector)),
        }
    }

    /// Applies TLS configuration to a tunnel connector for proxy usage
    fn apply_tls_to_tunnel_connector(
        &self,
        builder: tonic::transport::Endpoint,
        tunnel: Tunnel<HttpConnector>,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        match tls_config {
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
                    .service(tunnel);

                Ok(builder.connect_with_connector_lazy(connector))
            }
            None => Ok(builder.connect_with_connector_lazy(tunnel)),
        }
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
                let auth_layer = basic
                    .get_client_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                self.warn_insecure_auth();

                Ok(tower::ServiceBuilder::new()
                    .layer(SetRequestHeaderLayer::new(header_map))
                    .layer(auth_layer)
                    .service(channel)
                    .boxed())
            }
            AuthenticationConfig::Bearer(bearer) => {
                let auth_layer = bearer
                    .get_client_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                self.warn_insecure_auth();

                Ok(tower::ServiceBuilder::new()
                    .layer(SetRequestHeaderLayer::new(header_map))
                    .layer(auth_layer)
                    .service(channel)
                    .boxed())
            }
            AuthenticationConfig::Jwt(jwt) => {
                let auth_layer = jwt
                    .get_client_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                self.warn_insecure_auth();

                Ok(tower::ServiceBuilder::new()
                    .layer(SetRequestHeaderLayer::new(header_map))
                    .layer(auth_layer)
                    .service(channel)
                    .boxed())
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
        client.proxy = Some(ProxyConfig::new("http://proxy.example.com:8080"));
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set proxy with authentication
        client.proxy =
            Some(ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass"));
        channel = client.to_channel();
        assert!(channel.is_ok());

        // Set proxy with headers
        let mut proxy_headers = HashMap::new();
        proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            Some(ProxyConfig::new("http://proxy.example.com:8080").with_headers(proxy_headers));
        channel = client.to_channel();
        assert!(channel.is_ok());
    }

    #[test]
    fn test_proxy_config() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080");
        assert_eq!(proxy.url, "http://proxy.example.com:8080");
        assert_eq!(proxy.username, None);
        assert_eq!(proxy.password, None);
        assert!(proxy.headers.is_empty());

        let proxy_with_auth = proxy.with_auth("user", "pass");
        assert_eq!(proxy_with_auth.username, Some("user".to_string()));
        assert_eq!(proxy_with_auth.password, Some("pass".to_string()));

        let mut headers = HashMap::new();
        headers.insert("X-Custom".to_string(), "value".to_string());
        let proxy_with_headers =
            ProxyConfig::new("http://proxy.example.com:8080").with_headers(headers.clone());
        assert_eq!(proxy_with_headers.headers, headers);
    }

    #[test]
    fn test_proxy_no_proxy_functionality() {
        let no_proxy_list = [
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            ".internal.com".to_string(),
            ".example.org".to_string(),
            "direct.access.com".to_string(),
        ];

        let proxy = ProxyConfig::new("http://proxy.example.com:8080")
            .with_no_proxy(no_proxy_list.join(","));

        assert_eq!(proxy.no_proxy, Some(no_proxy_list.join(",")));

        // Test exact matches
        assert!(proxy.should_use_proxy("http://localhost").is_none());
        assert!(proxy.should_use_proxy("http://127.0.0.1").is_none());
        assert!(proxy.should_use_proxy("http://direct.access.com").is_none());

        // Test wildcard matching
        assert!(proxy.should_use_proxy("https://api.internal.com").is_none());
        assert!(proxy.should_use_proxy("http://sub.internal.com").is_none());
        assert!(proxy.should_use_proxy("http://internal.com").is_none());

        // Test non-matching hosts
        assert!(proxy.should_use_proxy("http://google.com").is_some());
        assert!(proxy.should_use_proxy("http://api.external.com").is_some());
        assert!(proxy.should_use_proxy("http://192.168.1.1").is_some());
    }

    #[test]
    fn test_proxy_no_proxy_edge_cases() {
        // Test empty no_proxy list
        let proxy_empty = ProxyConfig::new("http://proxy.example.com:8080");
        assert!(proxy_empty.should_use_proxy("http://localhost").is_none());
        assert!(
            proxy_empty
                .should_use_proxy("http://anything.com")
                .is_none()
        );
    }

    #[test]
    fn test_client_config_with_proxy() {
        let proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
        let client = ClientConfig::with_endpoint("http://localhost:8080").with_proxy(proxy.clone());
        assert_eq!(client.proxy, Some(proxy));
    }

    #[test]
    fn test_client_config_with_no_proxy() {
        let no_proxy_list = "localhost,.internal.com";
        let proxy = ProxyConfig::new("http://proxy.example.com:8080").with_no_proxy(no_proxy_list);

        let client = ClientConfig::with_endpoint("http://localhost:8080").with_proxy(proxy);

        // Test that the client config includes the no_proxy configuration
        if let Some(ref proxy_config) = client.proxy {
            assert_eq!(proxy_config.no_proxy, Some(no_proxy_list.to_string()));
            assert!(proxy_config.should_use_proxy("https://localhost").is_none());
            assert!(
                proxy_config
                    .should_use_proxy("http://api.internal.com")
                    .is_none()
            );
            assert!(
                proxy_config
                    .should_use_proxy("http://external.com")
                    .is_some()
            );
        } else {
            panic!("Proxy configuration should be present");
        }
    }
}

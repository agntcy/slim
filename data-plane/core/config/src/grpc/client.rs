// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_str::deserialize_duration;
use std::time::Duration;
use std::{collections::HashMap, str::FromStr};
use tower::ServiceExt;

use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper_rustls;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::Deserialize;
use tonic::codegen::{Body, Bytes, StdError};
use tonic::transport::{Channel, Uri};
use tracing::warn;

use super::compression::CompressionType;
use super::errors::ConfigError;
use super::headers_middleware::SetRequestHeaderLayer;
use crate::auth::ClientAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::bearer::Config as BearerAuthenticationConfig;
use crate::component::configuration::{Configuration, ConfigurationError};
use crate::tls::{client::TlsClientConfig as TLSSetting, common::RustlsConfigLoader};

/// Keepalive configuration for the client.
/// This struct contains the keepalive time for TCP and HTTP2,
/// the timeout duration for the keepalive, and whether to permit
/// keepalive without an active stream.
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct KeepaliveConfig {
    /// The duration of the keepalive time for TCP
    #[serde(
        default = "default_tcp_keepalive",
        deserialize_with = "deserialize_duration"
    )]
    pub tcp_keepalive: Duration,

    /// The duration of the keepalive time for HTTP2
    #[serde(
        default = "default_http2_keepalive",
        deserialize_with = "deserialize_duration"
    )]
    pub http2_keepalive: Duration,

    /// The timeout duration for the keepalive
    #[serde(default = "default_timeout", deserialize_with = "deserialize_duration")]
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
#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// Bearer authentication configuration.
    Bearer(BearerAuthenticationConfig),
    /// None
    #[default]
    None,
}

/// Struct for the client configuration.
/// This struct contains the endpoint, origin, compression type, rate limit,
/// TLS settings, keepalive settings, timeout settings, buffer size settings,
/// headers, and auth settings.
/// The client configuration can be converted to a tonic channel.
#[derive(Debug, Deserialize, Clone, PartialEq)]
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

    /// Timeout for the connection.
    #[serde(
        default = "default_connect_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub connect_timeout: Duration,

    /// Timeout per request.
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub request_timeout: Duration,

    /// ReadBufferSize.
    pub buffer_size: Option<usize>,

    /// The headers associated with gRPC requests.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Auth configuration for outgoing RPCs.
    #[serde(default)]
    #[serde(with = "serde_yaml::with::singleton_map")]
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
            "ClientConfig {{ endpoint: {}, origin: {:?}, compression: {:?}, rate_limit: {:?}, tls_setting: {:?}, keepalive: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, auth: {:?} }}",
            self.endpoint,
            self.origin,
            self.compression,
            self.rate_limit,
            self.tls_setting,
            self.keepalive,
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
        // Make sure the endpoint is set and is valid
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        // channel builder
        let uri =
            Uri::from_str(&self.endpoint).map_err(|e| ConfigError::UriParseError(e.to_string()))?;
        let builder = Channel::builder(uri);

        // HTTP2 connector. We need this to be able to use directly a rustls config
        // cf. https://github.com/hyperium/tonic/issues/1615
        let mut http = HttpConnector::new();

        // NOTE(msardara): we might want to make these configurable as well.
        http.enforce_http(false);
        http.set_nodelay(false);

        // set the connection timeout
        match self.connect_timeout.as_secs() {
            0 => http.set_connect_timeout(None),
            _ => http.set_connect_timeout(Some(self.connect_timeout)),
        }

        // set the buffer size
        let builder = match self.buffer_size {
            Some(size) => builder.buffer_size(size),
            None => builder,
        };

        // set keepalive settings
        let builder = match &self.keepalive {
            Some(keepalive) => {
                // TCP level keepalive
                http.set_keepalive(Some(keepalive.tcp_keepalive));

                builder
                    .keep_alive_timeout(keepalive.timeout)
                    .keep_alive_while_idle(keepalive.keep_alive_while_idle)
                    // HTTP level keepalive
                    .http2_keep_alive_interval(keepalive.http2_keepalive)
            }
            None => builder,
        };

        // set origin settings
        let builder = match &self.origin {
            Some(origin) => {
                let uri = Uri::from_str(origin.as_str())
                    .map_err(|e| ConfigError::UriParseError(e.to_string()))?;

                builder.origin(uri)
            }
            None => builder,
        };

        let builder = match &self.rate_limit {
            Some(rate_limit) => {
                let (limit, duration) = parse_rate_limit(rate_limit)
                    .map_err(|e| ConfigError::RateLimitParseError(e.to_string()))?;
                builder.rate_limit(limit, duration)
            }
            None => builder,
        };

        // set the request timeout
        let builder = match self.request_timeout.as_secs() {
            0 => builder,
            _ => builder.timeout(self.request_timeout),
        };

        // set header to http connector
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

        // TLS configuration
        let tls_config = TLSSetting::load_rustls_config(&self.tls_setting)
            .map_err(|e| ConfigError::TLSSettingError(e.to_string()))?;

        let channel = match tls_config {
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
                    .service(http);

                builder.connect_with_connector_lazy(connector)
            }
            None => builder.connect_with_connector_lazy(http),
        };

        // Auth configuration
        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                let auth_layer = basic
                    .get_client_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                // If auth is enabled without TLS, print a warning
                if self.tls_setting.insecure {
                    warn!("Auth is enabled without TLS. This is not recommended.");
                }

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

                // If auth is enabled without TLS, print a warning
                if self.tls_setting.insecure {
                    warn!("Auth is enabled without TLS. This is not recommended.");
                }

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
    }
}

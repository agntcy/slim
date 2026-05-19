// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub use crate::client::{
    AuthenticationConfig, BackoffConfig, ClientConfig, KeepaliveConfig, TransportChannel,
    is_valid_uuid_v4,
};

use std::collections::HashMap;
#[cfg(target_family = "unix")]
use std::error::Error as StdErrorTrait;
#[cfg(target_family = "unix")]
use std::path::PathBuf;
use std::str::FromStr;
#[cfg(target_family = "unix")]
use std::sync::Arc;

use base64::prelude::*;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::proxy::matcher::Intercept;
#[cfg(target_family = "unix")]
use hyper_util::rt::TokioIo;
use rustls_pki_types::ServerName;
#[cfg(target_family = "unix")]
use tokio::net::UnixStream;
use tonic::codegen::{Body, Bytes, StdError};
use tonic::transport::{Channel, Uri};
use tower::ServiceExt;
#[cfg(target_family = "unix")]
use tower::service_fn;
use tracing::warn;

use crate::auth::ClientAuthenticator;
use crate::errors::ConfigError;
use crate::grpc::headers_middleware::SetRequestHeaderLayer;
use crate::grpc::proxy::ProxyConfig;
use crate::tls::common::RustlsConfigLoader;
use crate::transport::TransportProtocol;

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
/// Supports both lazy and eager connection modes.
macro_rules! create_connector {
    ($builder:expr, $base_connector:expr, $tls_config:expr, $server_name:expr, $lazy:expr) => {
        match ($tls_config, $lazy) {
            (Some(tls), true) => {
                let connector = tower::ServiceBuilder::new()
                    .layer_fn(move |s| {
                        https_connector(s, &tls, $server_name.map(|s| s.to_string()))
                    })
                    .service($base_connector);
                Ok($builder.connect_with_connector_lazy(connector))
            }
            (Some(tls), false) => {
                let connector = tower::ServiceBuilder::new()
                    .layer_fn(move |s| {
                        https_connector(s, &tls, $server_name.map(|s| s.to_string()))
                    })
                    .service($base_connector);
                let ret = $builder.connect_with_connector(connector).await?;
                Ok(ret)
            }
            (None, true) => Ok($builder.connect_with_connector_lazy($base_connector)),
            (None, false) => {
                let ret = $builder.connect_with_connector($base_connector).await?;
                Ok(ret)
            }
        }
    };
}

/// Macro to create authenticated service layers for auth types that don't need initialization
macro_rules! create_auth_service_no_init {
    ($self:expr, $auth_config:expr, $header_map:expr, $channel:expr) => {{
        let auth_layer = $auth_config.get_client_layer()?;

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
        let mut auth_layer = $auth_config.get_client_layer()?;

        // Initialize the auth layer
        auth_layer.initialize().await?;

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

impl ClientConfig {
    /// Build a gRPC channel from this configuration. Crate-private; external
    /// callers should use the polymorphic [`ClientConfig::to_channel`].
    pub(crate) async fn to_grpc_channel(
        &self,
    ) -> Result<
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
        ConfigError,
    > {
        self.build_grpc_channel(false).await
    }

    /// Lazy variant of [`Self::to_grpc_channel`]. Test-only.
    #[cfg(test)]
    pub(crate) async fn to_grpc_channel_lazy(
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
        self.build_grpc_channel(true).await
    }

    /// Internal implementation for channel creation with optional lazy flag.
    async fn build_grpc_channel(
        &self,
        lazy: bool,
    ) -> Result<
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
        ConfigError,
    > {
        if self.transport == TransportProtocol::Websocket {
            return Err(ConfigError::GrpcChannelUnsupportedTransport);
        }

        // Validate endpoint
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        // Parse headers
        let header_map = self.parse_headers()?;

        let uri = self.parse_endpoint_uri()?;

        let channel = if uri.scheme_str() == Some("unix") {
            self.connect_unix_channel(uri, lazy).await?
        } else if uri.scheme_str() == Some("http") || uri.scheme_str() == Some("https") {
            self.connect_tcp_channel(uri, lazy).await?
        } else {
            return Err(ConfigError::InvalidEndpointScheme);
        };

        // Apply authentication and headers
        self.apply_auth_and_headers(channel, header_map).await
    }

    /// Parses the endpoint string into a URI for TCP/HTTP, Unix domain socket endpoints.
    pub(crate) fn parse_endpoint_uri(&self) -> Result<Uri, ConfigError> {
        // Special case for the unix scheme because it doesn't have an
        // authority in the URI and the Uri parser doesn't like this today,
        // so we build our own URI with a fake localhost authority.
        if self.endpoint.starts_with("unix://") {
            let path = &self.endpoint[7..];
            if path.is_empty() {
                return Err(ConfigError::UnixSocketMissingPath);
            }

            let uri = Uri::builder()
                .scheme("unix")
                .authority("localhost")
                .path_and_query(path)
                .build()
                .map_err(ConfigError::UnixSocketInvalidPath)?;
            return Ok(uri);
        }
        Ok(Uri::from_str(&self.endpoint)?)
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
                .http2_keep_alive_interval(keepalive.http2_keepalive.into())
                .keep_alive_while_idle(keepalive.keep_alive_while_idle);
        }

        // set timeouts
        if self.connect_timeout.as_secs() != 0 {
            builder = builder.connect_timeout(self.connect_timeout.into());
        }
        if self.request_timeout.as_secs() != 0 {
            builder = builder.timeout(self.request_timeout.into());
        }

        // set rate limit
        if let Some(rate_limit) = &self.rate_limit {
            let (limit, duration) = parse_rate_limit(rate_limit)?;
            builder = builder.rate_limit(limit, duration);
        }

        // set origin / authority override
        if let Some(origin) = &self.origin {
            let uri = Uri::from_str(origin)?;
            builder = builder.origin(uri);
        }

        Ok(builder)
    }

    /// Parses headers from the configuration
    fn parse_headers(&self) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(&self.headers)
    }

    /// Generic helper to parse a HashMap<String, String> into HeaderMap
    fn parse_header_map(headers: &HashMap<String, String>) -> Result<HeaderMap, ConfigError> {
        let mut header_map = HeaderMap::new();
        for (key, value) in headers {
            let header_name = HeaderName::from_str(key)?;
            let header_value = HeaderValue::from_str(value)?;
            header_map.insert(header_name, header_value);
        }
        Ok(header_map)
    }

    #[cfg(target_family = "unix")]
    fn map_transport_error(err: tonic::transport::Error) -> ConfigError {
        let mut source: Option<&(dyn StdErrorTrait + 'static)> = Some(&err);
        while let Some(err_ref) = source {
            if let Some(io_err) = err_ref.downcast_ref::<std::io::Error>() {
                let cloned = std::io::Error::new(io_err.kind(), io_err.to_string());
                return ConfigError::UnixSocketConnect(cloned);
            }
            source = err_ref.source();
        }

        ConfigError::from(err)
    }

    /// Helper to create basic auth header for proxy authentication
    fn create_proxy_auth_header(
        username: &str,
        password: &str,
    ) -> Result<HeaderValue, ConfigError> {
        let auth_value = BASE64_STANDARD.encode(format!("{}:{}", username, password));
        Ok(HeaderValue::from_str(&format!("Basic {}", auth_value))?)
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
        let tls = self.tls_setting.load_rustls_config().await?;
        Ok(tls)
    }

    #[cfg(target_family = "unix")]
    pub(crate) async fn connect_unix_channel(
        &self,
        uri: Uri,
        lazy: bool,
    ) -> Result<Channel, ConfigError> {
        if !self.tls_setting.insecure {
            // TLS handshakes are unnecessary over local UDS and currently unsupported
            return Err(ConfigError::UnixSocketTlsUnsupported);
        }

        let path = uri.path();
        let socket_path = Arc::new(PathBuf::from(path));
        let builder = self.create_channel_builder(uri)?;

        let make_connector = || {
            let path = socket_path.clone();
            service_fn(move |_uri: Uri| {
                let path = path.clone();
                async move { UnixStream::connect(path.as_path()).await.map(TokioIo::new) }
            })
        };

        if lazy {
            Ok(builder.connect_with_connector_lazy(make_connector()))
        } else {
            self.retry_connect(|| {
                let builder = builder.clone();
                let connector = make_connector();
                let path = socket_path.clone();
                async move {
                    tracing::debug!(
                        socket_path = %path.display(),
                        "Attempting to create gRPC channel over Unix domain socket"
                    );
                    builder
                        .connect_with_connector(connector)
                        .await
                        .map_err(Self::map_transport_error)
                }
            })
            .await
        }
    }

    #[cfg(not(target_family = "unix"))]
    pub(crate) async fn connect_unix_channel(
        &self,
        _uri: Uri,
        _lazy: bool,
    ) -> Result<Channel, ConfigError> {
        Err(ConfigError::UnixSocketUnsupported)
    }

    pub(crate) async fn connect_tcp_channel(
        &self,
        uri: Uri,
        lazy: bool,
    ) -> Result<Channel, ConfigError> {
        let http_connector = self.create_http_connector()?;
        let builder = self.create_channel_builder(uri.clone())?;
        let tls_config = self.load_tls_config().await?;

        if lazy {
            let connection = self.create_connection(uri, http_connector).await?;
            self.create_channel_from_connection(builder, connection, tls_config, true)
                .await
        } else {
            self.retry_connect(|| {
                let uri = uri.clone();
                let builder = builder.clone();
                let http_connector = http_connector.clone();
                let tls_config = tls_config.clone();
                async move {
                    tracing::debug!(%uri, "Attempting to create gRPC channel");
                    self.create_channel_with_connector(uri, builder, http_connector, tls_config)
                        .await
                }
            })
            .await
        }
    }

    /// Creates the channel with the appropriate connector (proxy or direct)
    /// Creates a channel with the provided connector and TLS configuration.
    async fn create_channel_with_connector(
        &self,
        uri: Uri,
        builder: tonic::transport::Endpoint,
        http_connector: HttpConnector,
        tls_config: Option<rustls::ClientConfig>,
    ) -> Result<Channel, ConfigError> {
        let connection = self.create_connection(uri, http_connector).await?;
        self.create_channel_from_connection(builder, connection, tls_config, false)
            .await
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

        tracing::info!(%proxy_uri, "Creating proxy tunnel");

        // Check if the proxy URL uses HTTPS
        if proxy_uri.scheme_str() == Some("https") {
            let proxy_tls_config = self.proxy.tls_setting.load_rustls_config().await?.unwrap();

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
    async fn create_channel_from_connection(
        &self,
        builder: tonic::transport::Endpoint,
        connection: ConnectionType,
        tls_config: Option<rustls::ClientConfig>,
        lazy: bool,
    ) -> Result<Channel, ConfigError> {
        match connection {
            ConnectionType::Direct(connector) => {
                create_connector!(
                    builder,
                    connector,
                    tls_config,
                    self.server_name.as_deref(),
                    lazy
                )
            }
            ConnectionType::ProxyHttp(tunnel) => {
                create_connector!(
                    builder,
                    tunnel,
                    tls_config,
                    self.server_name.as_deref(),
                    lazy
                )
            }
            ConnectionType::ProxyHttps(tunnel) => {
                create_connector!(
                    builder,
                    tunnel,
                    tls_config,
                    self.server_name.as_deref(),
                    lazy
                )
            }
        }
    }

    /// Parses proxy headers
    fn parse_proxy_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(headers)
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
        >
        + Send
        + Clone
        + 'static
        + use<>,
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
            #[cfg(not(target_family = "windows"))]
            AuthenticationConfig::Spire(spire) => {
                create_auth_service_with_init!(self, spire, header_map, channel)
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

/// Parse the rate limit string into a limit and a duration.
/// The rate limit string should be in the format of <limit>/<duration>,
/// with duration expressed in seconds.
fn parse_rate_limit(rate_limit: &str) -> Result<(u64, std::time::Duration), ConfigError> {
    let parts: Vec<&str> = rate_limit.split('/').collect();

    if parts.len() != 2 {
        // Invalid format: expected <limit>/<duration>
        return Err(ConfigError::Unknown);
    }

    let limit = parts[0].parse::<u64>()?;
    let duration = std::time::Duration::from_secs(parts[1].parse::<u64>()?);

    Ok((limit, duration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BackoffConfig, KeepaliveConfig};
    use crate::tls::client::TlsClientConfig as TLSSetting;
    use crate::tls::common::CaSource;
    use hyper_util::rt::TokioIo;
    use std::time::Duration;
    use tower::service_fn;
    use tracing_test::traced_test;

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

    #[test]
    fn test_parse_endpoint_uri_http() {
        let client = ClientConfig::with_endpoint("http://localhost:1234");
        let uri = client.parse_endpoint_uri().expect("valid http uri");
        assert_eq!(uri.scheme_str(), Some("http"));
        assert_eq!(
            uri.authority().map(|auth| auth.as_str()),
            Some("localhost:1234")
        );
    }

    #[test]
    fn test_parse_endpoint_uri_unix() {
        let client = ClientConfig::with_endpoint("unix://tmp/slim.sock");
        let uri = client.parse_endpoint_uri().expect("valid unix uri");
        assert_eq!(uri.scheme_str(), Some("unix"));
        assert_eq!(uri.authority().map(|auth| auth.as_str()), Some("localhost"));
        assert_eq!(uri.path(), "tmp/slim.sock");
    }

    #[test]
    fn test_parse_endpoint_uri_unix_missing_path() {
        let client = ClientConfig::with_endpoint("unix://");
        let err = client.parse_endpoint_uri().expect_err("missing unix path");
        assert!(matches!(err, ConfigError::UnixSocketMissingPath));
    }

    #[tokio::test]
    async fn test_connect_tcp_channel_lazy_ok() {
        let client = ClientConfig::with_endpoint("http://127.0.0.1:0");
        let uri = client.parse_endpoint_uri().expect("valid http uri");
        let channel = client.connect_tcp_channel(uri, true).await;
        assert!(channel.is_ok());
    }

    #[tokio::test]
    async fn test_connect_tcp_channel_non_lazy_error() {
        let mut client = ClientConfig::with_endpoint("http://127.0.0.1:0")
            .with_connect_timeout(Duration::from_millis(50));
        client.backoff = BackoffConfig::new_fixed_interval(Duration::from_millis(0), 1);

        let uri = client.parse_endpoint_uri().expect("valid http uri");
        let err = client
            .connect_tcp_channel(uri, false)
            .await
            .expect_err("expected connect error");
        assert!(matches!(err, ConfigError::TransportError(_)));
    }

    #[cfg(target_family = "unix")]
    #[tokio::test]
    async fn test_connect_unix_channel_lazy_ok() {
        let mut client = ClientConfig::with_endpoint("unix:///tmp/slim-test.sock");
        client.tls_setting.insecure = true;

        let uri = client.parse_endpoint_uri().expect("valid unix uri");
        let channel = client.connect_unix_channel(uri, true).await;
        assert!(channel.is_ok());
    }

    #[cfg(target_family = "unix")]
    #[tokio::test]
    async fn test_connect_unix_channel_non_lazy_error() {
        let mut client = ClientConfig::with_endpoint("unix:///tmp/slim-missing.sock");
        client.tls_setting.insecure = true;
        client.backoff = BackoffConfig::new_fixed_interval(Duration::from_millis(0), 1);

        let uri = client.parse_endpoint_uri().expect("valid unix uri");
        let err = client
            .connect_unix_channel(uri, false)
            .await
            .expect_err("expected unix socket connect error");
        assert!(matches!(err, ConfigError::UnixSocketConnect(_)));
    }

    #[cfg(not(target_family = "unix"))]
    #[tokio::test]
    async fn test_connect_unix_channel_unsupported() {
        let client = ClientConfig::with_endpoint("unix:///tmp/slim.sock");
        let uri = client.parse_endpoint_uri().expect("valid unix uri");
        let err = client
            .connect_unix_channel(uri, true)
            .await
            .expect_err("expected unix socket unsupported");
        assert!(matches!(err, ConfigError::UnixSocketUnsupported));
    }

    #[cfg(target_family = "unix")]
    #[tokio::test]
    async fn test_map_transport_error_maps_io() {
        let endpoint = tonic::transport::Endpoint::from_static("http://localhost");
        let connector = service_fn(|_uri: Uri| async move {
            Err::<TokioIo<tokio::io::DuplexStream>, std::io::Error>(std::io::Error::other("boom"))
        });
        let err = endpoint
            .connect_with_connector(connector)
            .await
            .expect_err("expected connect error");
        let mapped = ClientConfig::map_transport_error(err);
        assert!(matches!(mapped, ConfigError::UnixSocketConnect(_)));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_to_grpc_channel() {
        let test_path: &str = env!("CARGO_MANIFEST_DIR");

        // create a new client config
        let mut client = ClientConfig::default();

        // as the endpoint is missing, this should fail
        let mut channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_err());

        // Set the endpoint
        client.endpoint = "http://localhost:8080".to_string();
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set the tls settings
        client.tls_setting.insecure = true;
        channel = client.to_grpc_channel_lazy().await;
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

        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set keepalive settings
        client.keepalive = Some(KeepaliveConfig::default());
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set rate limit settings
        client.rate_limit = Some("100/10".to_string());
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set rate limit settings wrong
        client.rate_limit = Some("100".to_string());
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_err());

        // reset config
        client.rate_limit = None;

        // Set timeout settings
        client.request_timeout = Duration::from_secs(10).into();
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set buffer size settings
        client.buffer_size = Some(1024);
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set origin settings
        client.origin = Some("http://example.com".to_string());
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // set additional header to add to the request
        client
            .headers
            .insert("X-Test".to_string(), "test".to_string());
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set proxy settings
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080");
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set proxy with authentication
        client.proxy = ProxyConfig::new("http://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set proxy with headers
        let mut proxy_headers = std::collections::HashMap::new();
        proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("http://proxy.example.com:8080").with_headers(proxy_headers);
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy settings
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080");
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy with authentication
        client.proxy = ProxyConfig::new("https://proxy.example.com:8080").with_auth("user", "pass");
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());

        // Set HTTPS proxy with headers
        let mut https_proxy_headers = std::collections::HashMap::new();
        https_proxy_headers.insert("X-Proxy-Header".to_string(), "value".to_string());
        client.proxy =
            ProxyConfig::new("https://proxy.example.com:8080").with_headers(https_proxy_headers);
        channel = client.to_grpc_channel_lazy().await;
        assert!(channel.is_ok());
    }

    #[tokio::test]
    async fn test_to_grpc_channel_rejects_websocket_transport() {
        let client = ClientConfig::with_endpoint("ws://localhost:46357")
            .with_transport(TransportProtocol::Websocket);
        let channel = client.to_grpc_channel_lazy().await;
        assert!(matches!(
            channel,
            Err(ConfigError::GrpcChannelUnsupportedTransport)
        ));
    }
}

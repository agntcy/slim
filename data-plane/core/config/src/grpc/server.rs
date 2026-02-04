// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;
use duration_string::DurationString;
use futures::FutureExt;
use futures::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::future::Future;
#[cfg(target_family = "unix")]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::{net::SocketAddr, str::FromStr, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(target_family = "unix")]
use tokio::net::UnixListener;
#[cfg(target_family = "unix")]
use tokio_stream::wrappers::UnixListenerStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::server::TcpIncoming;
use tower_http::BoxError;
use tracing::debug;

use super::errors::ConfigError;
use crate::auth::ServerAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
use crate::component::configuration::Configuration;
use slim_auth::metadata::MetadataMap;

use crate::tls::{common::RustlsConfigLoader, server::TlsServerConfig as TLSSetting};

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

/// Enum holding one configuration for the client.
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// JWT authentication configuration.
    Jwt(JwtAuthenticationConfig),
    /// None
    #[default]
    None,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, JsonSchema)]
pub struct ServerConfig {
    /// Endpoint is the address to listen on.
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
        }
    }
}

fn default_http2_only() -> bool {
    true
}

/// Display implementation for ServerConfig
/// This is used to print the ServerConfig in a human-readable format.
impl std::fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ServerConfig {{ endpoint: {}, tls_setting: {}, http2_only: {}, max_frame_size: {:?}, max_concurrent_streams: {:?}, max_header_list_size: {:?}, read_buffer_size: {:?}, write_buffer_size: {:?}, keepalive: {:?}, auth: {:?}, metadata: {:?} }}",
            self.endpoint,
            self.tls_setting,
            self.http2_only,
            self.max_frame_size,
            self.max_concurrent_streams,
            self.max_header_list_size,
            self.read_buffer_size,
            self.write_buffer_size,
            self.keepalive,
            self.auth,
            self.metadata
        )
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

impl Configuration for ServerConfig {
    type Error = ConfigError;

    fn validate(&self) -> Result<(), Self::Error> {
        // Validate the client configuration
        self.tls_setting.validate()?;

        Ok(())
    }
}

/// ServerFuture is a type alias for a boxed future that returns a Result<(), tonic::transport::Error>.
type ServerFuture = Pin<Box<dyn Future<Output = Result<(), tonic::transport::Error>> + Send>>;

/// Convert ServerConfig to IncomingServerConfig
/// This function takes a ServerConfig and a service and returns a ServerFuture.
/// The ServerFuture is a boxed future that returns a Result<(), tonic::transport::Error>.
/// The ServerFuture is created by creating a new TcpIncoming and then creating a new Server.
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

    #[cfg(target_family = "unix")]
    fn parse_unix_socket_path(endpoint: &str) -> Result<PathBuf, ConfigError> {
        let path = endpoint.strip_prefix("unix://").unwrap_or(endpoint);

        let without_query = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
        let path_part = without_query
            .split_once('#')
            .map(|(p, _)| p)
            .unwrap_or(without_query);

        if path_part.is_empty() {
            return Err(ConfigError::UnixSocketMissingPath);
        }

        Ok(PathBuf::from(path_part))
    }

    fn create_server_builder(&self) -> tonic::transport::Server {
        let builder: tonic::transport::Server =
            tonic::transport::Server::builder().accept_http1(false);

        let builder = match self.max_concurrent_streams {
            Some(max_concurrent_streams) => {
                builder.concurrency_limit_per_connection(max_concurrent_streams as usize)
            }
            None => builder,
        };

        let builder = match self.max_frame_size {
            Some(max_frame_size) => builder.max_frame_size(max_frame_size * 1024 * 1024),
            None => builder,
        };

        let builder = match self.max_header_list_size {
            Some(max_header_list_size) => builder.http2_max_header_list_size(max_header_list_size),
            None => builder,
        };

        let builder = builder.http2_keepalive_interval(Some(self.keepalive.time.into()));
        let builder = builder.http2_keepalive_timeout(Some(self.keepalive.timeout.into()));

        builder.max_connection_age(self.keepalive.max_connection_age.into())
    }

    async fn serve_with_incoming<S, I, IO, IE>(
        &self,
        svc: &[S],
        incoming: I,
    ) -> Result<ServerFuture, ConfigError>
    where
        S: tower_service::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            >
            + tonic::server::NamedService
            + Clone
            + Send
            + 'static
            + Sync,
        S::Future: Send + 'static,
        I: Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: AsyncRead + AsyncWrite + tonic::transport::server::Connected + Unpin + Send + 'static,
        IE: Into<BoxError> + Send + 'static,
    {
        let mut builder = self.create_server_builder();

        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                let auth_layer = basic.get_server_layer()?;

                let mut builder = builder.layer(auth_layer);

                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::Jwt(jwt) => {
                // Build the authentication layer and perform its async initialization
                let mut auth_layer = <JwtAuthenticationConfig as ServerAuthenticator<
                    http::Response<tonic::body::Body>,
                >>::get_server_layer(jwt)?;

                auth_layer.initialize().await?;

                let mut builder = builder.layer(auth_layer);

                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::None => {
                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                Ok(router.serve_with_incoming(incoming).boxed())
            }
        }
    }

    pub async fn to_server_future<S>(&self, svc: &[S]) -> Result<ServerFuture, ConfigError>
    where
        S: tower_service::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            >
            + tonic::server::NamedService
            + Clone
            + Send
            + 'static
            + Sync,
        S::Future: Send + 'static,
    {
        if svc.is_empty() {
            return Err(ConfigError::MissingServices);
        }

        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        #[cfg(target_family = "unix")]
        if self.endpoint.starts_with("unix://") {
            if !self.tls_setting.insecure {
                // For local Unix domain sockets we currently require insecure=true
                return Err(ConfigError::UnixSocketTlsUnsupported);
            }

            let socket_path = Self::parse_unix_socket_path(self.endpoint.as_str())?;

            // Best-effort cleanup of any stale socket file
            let _ = std::fs::remove_file(&socket_path);

            let listener = UnixListener::bind(&socket_path)?;
            let incoming = UnixListenerStream::new(listener);

            return self.serve_with_incoming(svc, incoming).await;
        }

        #[cfg(not(target_family = "unix"))]
        if self.endpoint.starts_with("unix://") {
            return Err(ConfigError::UnixSocketUnsupported);
        }

        let addr = SocketAddr::from_str(self.endpoint.as_str())?;

        // Async TLS configuration load (may involve SPIFFE operations)
        let tls_config = self.tls_setting.load_rustls_config().await?;
        let incoming = TcpIncoming::bind(addr)?;

        match tls_config {
            Some(tls_config) => {
                let incoming = tonic_tls::rustls::TlsIncoming::new(incoming, Arc::new(tls_config));
                self.serve_with_incoming(svc, incoming).await
            }
            None => self.serve_with_incoming(svc, incoming).await,
        }
    }

    pub async fn run_server<S>(
        &self,
        svc: &[S],
        drain_rx: drain::Watch,
    ) -> Result<tokio_util::sync::CancellationToken, ConfigError>
    where
        S: tower_service::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            >
            + tonic::server::NamedService
            + Clone
            + Send
            + 'static
            + Sync,
        S::Future: Send + 'static,
    {
        debug!(%self, "server configured: setting it up");
        let server_future = self.to_server_future(svc).await?;

        // create a new cancellation token
        let token = CancellationToken::new();
        let token_clone = token.clone();

        // spawn server acceptor in a new task
        tokio::spawn(async move {
            debug!("starting server main loop");
            let shutdown = drain_rx.signaled();

            tokio::select! {
                res = server_future => {
                    match res {
                        Ok(_) => {
                            debug!("server shutdown");
                        }
                        Err(e) => {
                            tracing::error!(error = %e.chain(), "server error");
                        }
                    }
                }
                _ = shutdown => {
                    debug!("shutting down server");
                }
                _ = token.cancelled() => {
                    debug!("cancellation token triggered: shutting down server");
                }
            }
        });

        Ok(token_clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::{Empty, helloworld::greeter_server::GreeterServer};
    use crate::tls::common::TlsSource;
    use serde_json;

    static TEST_DATA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/grpc");

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

    #[tokio::test]
    async fn test_to_incoming_server_config() {
        let mut server_config = ServerConfig::default();
        let empty_service = Arc::new(Empty::new());

        // no endpoint - should return an error
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        // Make sure the error is a ConfigError::MissingEndpoint
        assert!(ret.is_err_and(|e| { e.to_string().contains("missing grpc endpoint") }));

        // set the endpoint in the config. Now it shouhld fail because of the invalid endpoint
        server_config.endpoint = "0.0.0.0:123456".to_string();
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        // Make sure we got an EndpointParse error
        assert!(ret.is_err_and(|e| { matches!(e, ConfigError::EndpointParse(_)) }));

        // set a valid endpoint in the config. Now it should fail because of the missing cert/key files for tls
        server_config.endpoint = "0.0.0.0:12345".to_string();
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        assert!(ret.is_err_and(|e| { matches!(e, ConfigError::TlsConfig(_)) }));

        // set the tls setting to insecure. Now it should return a server future
        server_config.tls_setting.insecure = true;
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        assert!(ret.is_ok());

        // drop it, as we have a server listening on the port now
        drop(ret.unwrap());

        // Set insecure to false and configure certificate/key via TlsSource::File (updated API)
        server_config.tls_setting.insecure = false;
        server_config.tls_setting.config.source = TlsSource::File {
            cert: format!("{}/server.crt", TEST_DATA_PATH),
            key: format!("{}/server.key", TEST_DATA_PATH),
        };
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        assert!(ret.is_ok());
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

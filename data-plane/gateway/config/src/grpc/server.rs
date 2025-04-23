// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::{net::SocketAddr, str::FromStr, time::Duration};

use duration_str::deserialize_duration;
use futures::{FutureExt, TryStreamExt};
use serde::Deserialize;
use tonic::transport::server::TcpIncoming;

use super::errors::ConfigError;
use crate::auth::ServerAuthenticator;
use crate::auth::basic::Config as BasicAuthenticationConfig;
use crate::auth::bearer::Config as BearerAuthenticationConfig;
use crate::component::configuration::{Configuration, ConfigurationError};
use crate::tls::{common::RustlsConfigLoader, server::TlsServerConfig as TLSSetting};

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct KeepaliveServerParameters {
    /// max_connection_idle sets the time after which an idle connection is closed.
    #[serde(
        default = "default_max_connection_idle",
        deserialize_with = "deserialize_duration"
    )]
    max_connection_idle: Duration,

    /// max_connection_age sets the maximum amount of time a connection may exist before it will be closed.
    #[serde(
        default = "default_max_connection_age",
        deserialize_with = "deserialize_duration"
    )]
    max_connection_age: Duration,

    /// max_connection_age_grace is an additional time given after MaxConnectionAge before closing the connection.
    #[serde(
        default = "default_max_connection_age_grace",
        deserialize_with = "deserialize_duration"
    )]
    max_connection_age_grace: Duration,

    /// Time sets the frequency of the keepalive ping.
    #[serde(default = "default_time", deserialize_with = "deserialize_duration")]
    time: Duration,

    /// Timeout sets the amount of time the server waits for a keepalive ping ack.
    #[serde(default = "default_timeout", deserialize_with = "deserialize_duration")]
    timeout: Duration,
}

/// Enum holding one configuration for the client.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationConfig {
    /// Basic authentication configuration.
    Basic(BasicAuthenticationConfig),
    /// Bearer authentication configuration.
    Bearer(BearerAuthenticationConfig),
    /// None
    None,
}

impl Default for AuthenticationConfig {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
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
    #[serde(with = "serde_yaml::with::singleton_map")]
    pub auth: AuthenticationConfig,
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

fn default_max_connection_idle() -> Duration {
    Duration::from_secs(3600)
}

fn default_max_connection_age() -> Duration {
    Duration::from_secs(2 * 3600)
}

fn default_max_connection_age_grace() -> Duration {
    Duration::from_secs(5 * 60)
}

fn default_time() -> Duration {
    Duration::from_secs(2 * 60)
}

fn default_timeout() -> Duration {
    Duration::from_secs(20)
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
            "ServerConfig {{ endpoint: {}, tls_setting: {}, http2_only: {}, max_frame_size: {:?}, max_concurrent_streams: {:?}, max_header_list_size: {:?}, read_buffer_size: {:?}, write_buffer_size: {:?}, keepalive: {:?}, auth: {:?} }}",
            self.endpoint,
            self.tls_setting,
            self.http2_only,
            self.max_frame_size,
            self.max_concurrent_streams,
            self.max_header_list_size,
            self.read_buffer_size,
            self.write_buffer_size,
            self.keepalive,
            self.auth
        )
    }
}

impl Configuration for ServerConfig {
    fn validate(&self) -> Result<(), ConfigurationError> {
        // Validate the client configuration
        self.tls_setting.validate()
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

    pub fn to_server_future<S>(&self, svc: &[S]) -> Result<ServerFuture, ConfigError>
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
        // Make sure at least one service is provided
        if svc.is_empty() {
            return Err(ConfigError::MissingServices);
        }

        // Check if the endpoint is missing
        if self.endpoint.is_empty() {
            return Err(ConfigError::MissingEndpoint);
        }

        // make sure endpoint is valid
        let addr = SocketAddr::from_str(self.endpoint.as_str())
            .map_err(|e| ConfigError::EndpointParseError(e.to_string()))?;

        // create a new TcpIncoming
        let incoming =
            TcpIncoming::bind(addr).map_err(|e| ConfigError::TcpIncomingError(e.to_string()))?;

        // Create initial server config
        let builder: tonic::transport::Server =
            tonic::transport::Server::builder().accept_http1(false);

        // Set max number of concurrent streams per connection
        let builder = match self.max_concurrent_streams {
            Some(max_concurrent_streams) => {
                builder.concurrency_limit_per_connection(max_concurrent_streams as usize)
            }
            None => builder,
        };

        // Set max size of messages accepted by the server
        let builder = match self.max_frame_size {
            Some(max_frame_size) => builder.max_frame_size(max_frame_size * 1024 * 1024),
            None => builder,
        };

        // Set max header list size
        let builder = match self.max_header_list_size {
            Some(max_header_list_size) => builder.http2_max_header_list_size(max_header_list_size),
            None => builder,
        };

        // Set keepalive parameters
        let builder = builder.http2_keepalive_interval(Some(self.keepalive.time));
        let builder = builder.http2_keepalive_timeout(Some(self.keepalive.timeout));

        // Set max connection age
        let mut builder = builder.max_connection_age(self.keepalive.max_connection_age);

        // TLS configuration
        let tls_config = TLSSetting::load_rustls_config(&self.tls_setting)
            .map_err(|e| ConfigError::TLSSettingError(e.to_string()))?;

        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                let auth_layer = basic
                    .get_server_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                let mut builder = builder.layer(auth_layer);

                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                if let Some(tls_config) = tls_config {
                    let incoming = tonic_tls::rustls::incoming(incoming, Arc::new(tls_config))
                        .map_err(|e| ConfigError::TcpIncomingError(e.to_string()));

                    // Return the server future with the TLS configuration
                    return Ok(router.serve_with_incoming(incoming).boxed());
                };

                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::Bearer(bearer) => {
                let auth_layer = bearer
                    .get_server_layer()
                    .map_err(|e| ConfigError::AuthConfigError(e.to_string()))?;

                let mut builder = builder.layer(auth_layer);

                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                if let Some(tls_config) = tls_config {
                    let incoming = tonic_tls::rustls::incoming(incoming, Arc::new(tls_config))
                        .map_err(|e| ConfigError::TcpIncomingError(e.to_string()));

                    // Return the server future with the TLS configuration
                    return Ok(router.serve_with_incoming(incoming).boxed());
                };

                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::None => {
                let mut router = builder.add_service(svc[0].clone());
                for s in svc.iter().skip(1) {
                    router = builder.add_service(s.clone());
                }

                if let Some(tls_config) = tls_config {
                    let incoming = tonic_tls::rustls::incoming(incoming, Arc::new(tls_config))
                        .map_err(|e| ConfigError::TcpIncomingError(e.to_string()));

                    // Return the server future with the TLS configuration
                    return Ok(router.serve_with_incoming(incoming).boxed());
                };

                Ok(router.serve_with_incoming(incoming).boxed())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::{Empty, helloworld::greeter_server::GreeterServer};

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
        let ret = server_config.to_server_future(&[GreeterServer::from_arc(empty_service.clone())]);
        // Make sure the error is a ConfigError::MissingEndpoint
        assert!(ret.is_err_and(|e| { e.to_string().contains("missing grpc endpoint") }));

        // set the endpoint in the config. Now it shouhld fail because of the invalid endpoint
        server_config.endpoint = "0.0.0.0:123456".to_string();
        let ret = server_config.to_server_future(&[GreeterServer::from_arc(empty_service.clone())]);
        assert!(ret.is_err_and(|e| { e.to_string().contains("error parsing grpc endpoint") }));

        // set a valid endpoint in the config. Now it should fail because of the missing cert/key files for tls
        server_config.endpoint = "0.0.0.0:12345".to_string();
        let ret = server_config.to_server_future(&[GreeterServer::from_arc(empty_service.clone())]);
        assert!(ret.is_err_and(|e| { e.to_string().contains("tls setting error") }));

        // set the tls setting to insecure. Now it should return a server future
        server_config.tls_setting.insecure = true;
        let ret = server_config.to_server_future(&[GreeterServer::from_arc(empty_service.clone())]);
        assert!(ret.is_ok());

        // drop it, as we have a server listening on the port now
        drop(ret.unwrap());

        // Set insecure to false and set the path to the cert and key files
        server_config.tls_setting.insecure = false;
        server_config.tls_setting.config.cert_file = Some(format!("{}/server.crt", TEST_DATA_PATH));
        server_config.tls_setting.config.key_file = Some(format!("{}/server.key", TEST_DATA_PATH));
        let ret = server_config.to_server_future(&[GreeterServer::from_arc(empty_service.clone())]);
        assert!(ret.is_ok());
    }
}

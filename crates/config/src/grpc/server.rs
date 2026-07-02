// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub use crate::server::{AuthenticationConfig, KeepaliveServerParameters, ServerConfig};

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::{net::SocketAddr, str::FromStr};

use display_error_chain::ErrorChainExt;
use futures::FutureExt;
use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;
use tonic::service::Routes;
use tonic::transport::server::TcpIncoming;
use tower_http::BoxError;
use tracing::debug;

#[cfg(target_family = "unix")]
use {std::path::PathBuf, tokio::net::UnixListener, tokio_stream::wrappers::UnixListenerStream};

use crate::auth::ServerAuthenticator;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig as SpireAuthConfig;
use crate::errors::ConfigError;
use crate::tls::common::RustlsConfigLoader;
use crate::transport::TransportProtocol;

/// Boxed future returned by [`ServerConfig::to_server_future`].
pub type ServerFuture = Pin<Box<dyn Future<Output = Result<(), tonic::transport::Error>> + Send>>;

fn routes_from_slice<S>(svc: &[S]) -> Routes
where
    S: tower_service::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<tonic::body::Body>,
            Error = Infallible,
        > + tonic::server::NamedService
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    let mut routes = Routes::new(svc[0].clone());
    for s in svc.iter().skip(1) {
        routes = routes.add_service(s.clone());
    }
    routes
}

impl ServerConfig {
    /// Build the gRPC server future that drives the underlying tonic server.
    ///
    /// Returns [`ConfigError::GrpcServerUnsupportedTransport`] when invoked on
    /// a config whose `transport != Grpc`.
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
        self.to_server_future_with_routes(routes_from_slice(svc))
            .await
    }

    /// `Routes`-based variant of [`Self::to_server_future`]. This is the
    /// canonical entry point — the slice-based overload is kept as a thin
    /// compat wrapper used by the gRPC-only `run_grpc_server` API.
    pub async fn to_server_future_with_routes(
        &self,
        routes: Routes,
    ) -> Result<ServerFuture, ConfigError> {
        if self.resolved_transport() == TransportProtocol::Websocket {
            return Err(ConfigError::GrpcServerUnsupportedTransport);
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

            let socket_path = parse_unix_socket_path(self.endpoint.as_str())?;

            // Best-effort cleanup of any stale socket file
            let _ = std::fs::remove_file(&socket_path);

            let listener = UnixListener::bind(&socket_path)?;
            let incoming = UnixListenerStream::new(listener);

            return self.serve_with_incoming(routes, incoming).await;
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
                self.serve_with_incoming(routes, incoming).await
            }
            None => self.serve_with_incoming(routes, incoming).await,
        }
    }

    /// Spawn the gRPC server and return a [`CancellationToken`] that can be
    /// used to stop it. The server is also driven by the supplied `drain`
    /// watch for cooperative shutdown.
    ///
    /// Generic over a slice of tonic services for backwards compatibility;
    /// the unified entry point is [`Self::run_server`] (uses [`Routes`] via
    /// the [`crate::ServerHandler`] trait).
    pub async fn run_grpc_server<S>(
        &self,
        svc: &[S],
        drain_rx: drain::Watch,
    ) -> Result<CancellationToken, ConfigError>
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
        self.run_grpc_server_with_routes(routes_from_slice(svc), drain_rx)
            .await
    }

    /// `Routes`-based variant of [`Self::run_grpc_server`]. Used internally
    /// by [`Self::run_server`].
    pub async fn run_grpc_server_with_routes(
        &self,
        routes: Routes,
        drain_rx: drain::Watch,
    ) -> Result<CancellationToken, ConfigError> {
        debug!(%self, "server configured: setting it up");
        let server_future = self.to_server_future_with_routes(routes).await?;

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

        let builder = match self.initial_stream_window_size {
            Some(sz) => builder.initial_stream_window_size(sz),
            None => builder,
        };

        let builder = match self.initial_connection_window_size {
            Some(sz) => builder.initial_connection_window_size(sz),
            None => builder,
        };

        let builder = builder.http2_keepalive_interval(Some(self.keepalive.time.into()));
        let builder = builder.http2_keepalive_timeout(Some(self.keepalive.timeout.into()));

        builder.max_connection_age(self.keepalive.max_connection_age.into())
    }

    async fn serve_with_incoming<I, IO, IE>(
        &self,
        routes: Routes,
        incoming: I,
    ) -> Result<ServerFuture, ConfigError>
    where
        I: Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: AsyncRead + AsyncWrite + tonic::transport::server::Connected + Unpin + Send + 'static,
        IE: Into<BoxError> + Send + 'static,
    {
        let mut builder = self.create_server_builder();

        match &self.auth {
            AuthenticationConfig::Basic(basic) => {
                let auth_layer = basic.get_server_layer()?;
                let router = builder.layer(auth_layer).add_routes(routes);
                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::Jwt(jwt) => {
                // Build the authentication layer and perform its async initialization
                let mut auth_layer = <JwtAuthenticationConfig as ServerAuthenticator<
                    http::Response<tonic::body::Body>,
                >>::get_server_layer(jwt)?;

                auth_layer.initialize().await?;

                let router = builder.layer(auth_layer).add_routes(routes);
                Ok(router.serve_with_incoming(incoming).boxed())
            }
            #[cfg(not(target_family = "windows"))]
            AuthenticationConfig::Spire(spire) => {
                let mut auth_layer = <SpireAuthConfig as ServerAuthenticator<
                    http::Response<tonic::body::Body>,
                >>::get_server_layer(spire)?;

                auth_layer.initialize().await?;

                let router = builder.layer(auth_layer).add_routes(routes);
                Ok(router.serve_with_incoming(incoming).boxed())
            }
            AuthenticationConfig::None => {
                let router = builder.add_routes(routes);
                Ok(router.serve_with_incoming(incoming).boxed())
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::{Empty, helloworld::greeter_server::GreeterServer};
    use crate::tls::common::TlsSource;

    static TEST_DATA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/grpc");

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

        // set the endpoint in the config. Now it should fail because of the invalid endpoint
        server_config.endpoint = "0.0.0.0:123456".to_string();
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service.clone())])
            .await;
        assert!(ret.is_err_and(|e| { matches!(e, ConfigError::EndpointParse(_)) }));

        // set a valid endpoint. Should fail because of missing cert/key files for tls
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

        // Set insecure to false and configure certificate/key via TlsSource::File
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

    #[tokio::test]
    async fn test_to_server_future_rejects_websocket_transport() {
        let empty_service = Arc::new(Empty::new());
        let server_config = ServerConfig::with_endpoint("ws://0.0.0.0:12345");
        let ret = server_config
            .to_server_future(&[GreeterServer::from_arc(empty_service)])
            .await;
        assert!(matches!(
            ret,
            Err(ConfigError::GrpcServerUnsupportedTransport)
        ));
    }
}

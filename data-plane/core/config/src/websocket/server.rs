// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use fastwebsockets::upgrade;
use http_body_util::Empty;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::grpc::errors::ConfigError;
use crate::server::ServerConfig;
use crate::tls::common::RustlsConfigLoader;
use crate::transport::TransportProtocol;

use super::common::{
    ServerHandshakeAuth, UpgradedWebSocket, WebSocketEndpoint, build_server_handshake_auth,
};

pub struct AcceptedWebSocketConnection {
    pub websocket: UpgradedWebSocket,
    pub remote_addr: Option<SocketAddr>,
    pub local_addr: Option<SocketAddr>,
}

pub type OnAcceptedWebSocket = Arc<
    dyn Fn(AcceptedWebSocketConnection) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

impl ServerConfig {
    pub async fn run_websocket_server(
        &self,
        drain_rx: drain::Watch,
        on_accepted: OnAcceptedWebSocket,
    ) -> Result<CancellationToken, ConfigError> {
        if self.transport != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketServerUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;
        let listener = TcpListener::bind(endpoint.socket_address()).await?;

        let tls_config = self.tls_setting.load_rustls_config().await?;
        let tls_acceptor = match (endpoint.secure, tls_config) {
            (true, Some(config)) => Some(TlsAcceptor::from(Arc::new(config))),
            (true, None) => return Err(ConfigError::WebSocketTlsConfiguration),
            (false, Some(_)) => return Err(ConfigError::WebSocketTlsConfiguration),
            (false, None) => None,
        };

        let auth = build_server_handshake_auth(self);
        let expected_path = endpoint.path.clone();

        let cancellation_token = CancellationToken::new();
        let cancel_clone = cancellation_token.clone();

        tokio::spawn(async move {
            let mut drain_signal = std::pin::pin!(drain_rx.signaled());

            loop {
                tokio::select! {
                    _ = &mut drain_signal => {
                        debug!("websocket server shutting down on drain");
                        break;
                    }
                    _ = cancel_clone.cancelled() => {
                        debug!("websocket server shutting down on cancellation token");
                        break;
                    }
                    accepted = listener.accept() => {
                        let (stream, remote_addr) = match accepted {
                            Ok(val) => val,
                            Err(err) => {
                                warn!(error = %err, "websocket accept error");
                                continue;
                            }
                        };

                        let local_addr = stream.local_addr().ok();
                        let auth = auth.clone();
                        let expected_path = expected_path.clone();
                        let on_accepted = on_accepted.clone();

                        if let Some(acceptor) = tls_acceptor.clone() {
                            tokio::spawn(async move {
                                let stream = match acceptor.accept(stream).await {
                                    Ok(stream) => stream,
                                    Err(err) => {
                                        warn!(error = %err, "websocket TLS accept error");
                                        return;
                                    }
                                };

                                serve_connection(
                                    stream,
                                    auth,
                                    expected_path,
                                    on_accepted,
                                    remote_addr,
                                    local_addr,
                                )
                                .await;
                            });
                        } else {
                            tokio::spawn(async move {
                                serve_connection(
                                    stream,
                                    auth,
                                    expected_path,
                                    on_accepted,
                                    remote_addr,
                                    local_addr,
                                )
                                .await;
                            });
                        }
                    }
                }
            }
        });

        Ok(cancellation_token)
    }
}

async fn serve_connection<S>(
    stream: S,
    auth: ServerHandshakeAuth,
    expected_path: String,
    on_accepted: OnAcceptedWebSocket,
    remote_addr: SocketAddr,
    local_addr: Option<SocketAddr>,
) where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = TokioIo::new(stream);
    let service = service_fn(move |mut request: Request<Incoming>| {
        let auth = auth.clone();
        let expected_path = expected_path.clone();
        let on_accepted = on_accepted.clone();

        async move {
            if request.uri().path() != expected_path {
                return Ok::<Response<Empty<Bytes>>, Infallible>(response_with_status(
                    StatusCode::NOT_FOUND,
                ));
            }

            if !upgrade::is_upgrade_request(&request) {
                return Ok::<Response<Empty<Bytes>>, Infallible>(response_with_status(
                    StatusCode::BAD_REQUEST,
                ));
            }

            if !auth.authorize(&request).await {
                return Ok::<Response<Empty<Bytes>>, Infallible>(response_with_status(
                    StatusCode::UNAUTHORIZED,
                ));
            }

            match upgrade::upgrade(&mut request) {
                Ok((response, future)) => {
                    tokio::spawn(async move {
                        match future.await {
                            Ok(websocket) => {
                                on_accepted(AcceptedWebSocketConnection {
                                    websocket,
                                    remote_addr: Some(remote_addr),
                                    local_addr,
                                })
                                .await;
                            }
                            Err(err) => {
                                warn!(error = %err, "websocket upgrade error");
                            }
                        }
                    });

                    Ok::<Response<Empty<Bytes>>, Infallible>(response)
                }
                Err(err) => {
                    warn!(error = %err, "websocket upgrade rejected");
                    Ok::<Response<Empty<Bytes>>, Infallible>(response_with_status(
                        StatusCode::BAD_REQUEST,
                    ))
                }
            }
        }
    });

    let connection = http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades();

    if let Err(err) = connection.await {
        debug!(error = %err, "websocket HTTP connection closed with error");
    }
}

fn response_with_status(status: StatusCode) -> Response<Empty<Bytes>> {
    Response::builder()
        .status(status)
        .body(Empty::new())
        .expect("valid websocket HTTP response")
}

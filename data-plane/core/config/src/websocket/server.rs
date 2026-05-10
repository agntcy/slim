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

use super::common::{UpgradedWebSocket, WebSocketEndpoint, authorize_server_handshake};

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

        let auth = self.auth.clone();
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
    auth: crate::server::AuthenticationConfig,
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

            if !authorize_server_handshake(&auth, &request).await {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::auth::basic::Config as BasicAuthConfig;
    use crate::auth::jwt::{Claims, Config as JwtConfig, JwtKey};
    use crate::server::AuthenticationConfig;
    use crate::tls::server::TlsServerConfig;
    use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};

    fn hs256_key(secret: &str) -> Key {
        Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data(secret.to_string()),
        }
    }

    fn websocket_upgrade_request(path: &str, auth_header: Option<&str>) -> String {
        let mut request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Connection: Upgrade\r\n\
             Upgrade: websocket\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n"
        );
        if let Some(value) = auth_header {
            request.push_str(&format!("Authorization: {value}\r\n"));
        }
        request.push_str("\r\n");
        request
    }

    async fn run_serve_connection_case(
        auth: AuthenticationConfig,
        expected_path: &str,
        raw_request: &str,
    ) -> String {
        let (mut client_io, server_io) = tokio::io::duplex(8 * 1024);
        let on_accepted: OnAcceptedWebSocket = Arc::new(|_accepted| Box::pin(async move {}));
        let remote_addr = SocketAddr::from(([127, 0, 0, 1], 51000));

        let mut serve_task = tokio::spawn(serve_connection(
            server_io,
            auth,
            expected_path.to_string(),
            on_accepted,
            remote_addr,
            None,
        ));

        client_io
            .write_all(raw_request.as_bytes())
            .await
            .expect("write request");

        let mut buffer = [0_u8; 2048];
        let n = tokio::time::timeout(Duration::from_secs(2), client_io.read(&mut buffer))
            .await
            .expect("timed out waiting for response")
            .expect("read response");
        let response = String::from_utf8_lossy(&buffer[..n]).to_string();

        drop(client_io);
        if tokio::time::timeout(Duration::from_secs(2), &mut serve_task)
            .await
            .is_err()
        {
            serve_task.abort();
        }

        response
    }

    #[test]
    fn response_with_status_sets_http_status() {
        let response = response_with_status(StatusCode::UNAUTHORIZED);
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn run_websocket_server_rejects_non_websocket_transport() {
        let config = ServerConfig::with_endpoint("127.0.0.1:0");
        let (_signal, watch) = drain::channel();
        let on_accepted: OnAcceptedWebSocket = Arc::new(|_accepted| Box::pin(async move {}));

        let err = config
            .run_websocket_server(watch, on_accepted)
            .await
            .expect_err("must reject grpc transport");
        assert!(matches!(
            err,
            ConfigError::WebSocketServerUnsupportedTransport
        ));
    }

    #[tokio::test]
    async fn run_websocket_server_rejects_wss_without_tls_config() {
        let config = ServerConfig::with_endpoint("wss://127.0.0.1:0")
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let (_signal, watch) = drain::channel();
        let on_accepted: OnAcceptedWebSocket = Arc::new(|_accepted| Box::pin(async move {}));

        let err = config
            .run_websocket_server(watch, on_accepted)
            .await
            .expect_err("must reject missing tls config for wss");
        assert!(matches!(err, ConfigError::WebSocketTlsConfiguration));
    }

    #[tokio::test]
    async fn run_websocket_server_rejects_ws_with_tls_configured() {
        let testdata_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/grpc");
        let tls = TlsServerConfig::new().with_cert_and_key_file(
            &format!("{testdata_dir}/server.crt"),
            &format!("{testdata_dir}/server.key"),
        );
        let config = ServerConfig::with_endpoint("ws://127.0.0.1:0")
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(tls);
        let (_signal, watch) = drain::channel();
        let on_accepted: OnAcceptedWebSocket = Arc::new(|_accepted| Box::pin(async move {}));

        let err = config
            .run_websocket_server(watch, on_accepted)
            .await
            .expect_err("must reject tls for ws");
        assert!(matches!(err, ConfigError::WebSocketTlsConfiguration));
    }

    #[tokio::test]
    async fn serve_connection_returns_not_found_for_wrong_path() {
        let response = run_serve_connection_case(
            AuthenticationConfig::None,
            "/ws",
            "GET /other HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 404"));
    }

    #[tokio::test]
    async fn serve_connection_returns_bad_request_for_non_upgrade_request() {
        let response = run_serve_connection_case(
            AuthenticationConfig::None,
            "/ws",
            "GET /ws HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 400"));
    }

    #[tokio::test]
    async fn serve_connection_returns_unauthorized_for_invalid_basic_credentials() {
        // codeql[rust/hard-coded-cryptographic-value]
        let response = run_serve_connection_case(
            AuthenticationConfig::Basic(BasicAuthConfig::new("alice", "secret")),
            "/ws",
            &websocket_upgrade_request("/ws", Some("Basic YWxpY2U6d3Jvbmc=")),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 401"));
    }

    #[tokio::test]
    async fn serve_connection_returns_unauthorized_for_invalid_jwt_config() {
        let response = run_serve_connection_case(
            AuthenticationConfig::Jwt(JwtConfig::new(
                Claims::default(),
                Duration::from_secs(60),
                JwtKey::Encoding(hs256_key("secret")),
            )),
            "/ws",
            &websocket_upgrade_request("/ws", Some("Bearer token")),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 401"));
    }

    #[tokio::test]
    async fn serve_connection_returns_switching_protocols_for_valid_upgrade() {
        let response = run_serve_connection_case(
            AuthenticationConfig::None,
            "/ws",
            &websocket_upgrade_request("/ws", None),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 101"));
    }
}

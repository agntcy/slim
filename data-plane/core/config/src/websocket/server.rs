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

use crate::errors::ConfigError;
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::basic::Config as BasicConfig;
    use crate::client::ClientConfig;
    use crate::server::AuthenticationConfig as ServerAuthConfig;
    use crate::tls::client::TlsClientConfig;
    use crate::tls::server::TlsServerConfig;
    use std::net::TcpListener as StdTcpListener;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream as TokioTcpStream;

    fn available_port() -> u16 {
        StdTcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("local_addr")
            .port()
    }

    /// Poll for server readiness with exponential backoff (max ~2s) to avoid
    /// flaky sleep-based readiness checks.
    async fn wait_for_server_ready(addr: &str, max_attempts: u32) -> bool {
        for attempt in 0..max_attempts {
            if TokioTcpStream::connect(addr).await.is_ok() {
                return true;
            }
            let backoff = Duration::from_millis(25 * (1 + attempt as u64).min(10));
            tokio::time::sleep(backoff).await;
        }
        false
    }

    fn noop_on_accepted() -> OnAcceptedWebSocket {
        Arc::new(|_| Box::pin(async {}))
    }

    async fn start_ws_server(server_conf: ServerConfig) -> CancellationToken {
        let port = server_conf
            .endpoint
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .expect("port");
        let (signal, watch) = drain::channel();
        // Keep the signal alive for the lifetime of the test; dropping it
        // would immediately drain the server.
        std::mem::forget(signal);
        let token = server_conf
            .run_websocket_server(watch, noop_on_accepted())
            .await
            .expect("server start");
        assert!(
            wait_for_server_ready(&format!("127.0.0.1:{port}"), 40).await,
            "server did not become ready in time",
        );
        token
    }

    #[tokio::test]
    async fn test_websocket_server_starts() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());

        let token = start_ws_server(cfg).await;
        token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_rejects_non_websocket_transport() {
        let port = available_port();
        // Default transport is gRPC.
        let cfg = ServerConfig::with_endpoint(&format!("127.0.0.1:{port}"));
        let (signal, watch) = drain::channel();
        std::mem::forget(signal);
        let res = cfg.run_websocket_server(watch, noop_on_accepted()).await;
        assert!(matches!(
            res,
            Err(ConfigError::WebSocketServerUnsupportedTransport)
        ));
    }

    #[tokio::test]
    async fn test_websocket_server_rejects_invalid_endpoint() {
        let cfg = ServerConfig::with_endpoint("not-a-ws-uri")
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let (signal, watch) = drain::channel();
        std::mem::forget(signal);
        let res = cfg.run_websocket_server(watch, noop_on_accepted()).await;
        assert!(res.is_err());
    }

    async fn raw_http_request(addr: &str, request: &str) -> String {
        let mut stream = TokioTcpStream::connect(addr).await.expect("tcp connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        stream.flush().await.expect("flush");

        let mut response = Vec::with_capacity(512);
        let mut buf = [0u8; 256];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, stream.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    response.extend_from_slice(&buf[..n]);
                    if response.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        String::from_utf8_lossy(&response).to_string()
    }

    #[tokio::test]
    async fn test_websocket_server_404_on_unknown_path() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let token = start_ws_server(cfg).await;

        let req = format!(
            "GET /wrong/path HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n",
        );
        let resp = raw_http_request(&format!("127.0.0.1:{port}"), &req).await;
        assert!(
            resp.starts_with("HTTP/1.1 404"),
            "expected 404, got: {resp:?}"
        );

        token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_400_when_not_upgrade() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let token = start_ws_server(cfg).await;

        // Correct path, but no Upgrade header.
        let req = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",);
        let resp = raw_http_request(&format!("127.0.0.1:{port}"), &req).await;
        assert!(
            resp.starts_with("HTTP/1.1 400"),
            "expected 400, got: {resp:?}"
        );

        token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_401_on_failed_basic_auth() {
        let port = available_port();
        // pragma: allowlist secret
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure())
            .with_auth(ServerAuthConfig::Basic(BasicConfig::new("user", "pass")));
        let token = start_ws_server(cfg).await;

        // Correct path & upgrade headers but no Authorization.
        let req = format!(
            "GET / HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n",
        );
        let resp = raw_http_request(&format!("127.0.0.1:{port}"), &req).await;
        assert!(
            resp.starts_with("HTTP/1.1 401"),
            "expected 401, got: {resp:?}"
        );

        token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_full_handshake_via_client() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let token = start_ws_server(cfg).await;

        let client_cfg = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure());

        let channel =
            tokio::time::timeout(Duration::from_secs(5), client_cfg.to_websocket_channel())
                .await
                .expect("handshake timed out")
                .expect("handshake failed");

        assert!(channel.remote_addr.is_some());
        assert!(channel.local_addr.is_some());

        token.cancel();
    }

    #[tokio::test]
    async fn test_websocket_server_cancellation_stops_listener() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure());
        let token = start_ws_server(cfg).await;

        token.cancel();

        // Wait for listener to actually stop accepting.
        for _ in 0..40 {
            if TokioTcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("listener did not stop after cancellation");
    }
}

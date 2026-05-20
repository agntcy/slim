// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use fastwebsockets::upgrade;
use http_body_util::Empty;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use slim_auth::jwt::VerifierJwt;
use slim_auth::jwt_middleware::ValidateJwtLayer;
use slim_auth::metadata::MetadataMap;
#[cfg(not(target_family = "windows"))]
use slim_auth::spire::SpireIdentityManager;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tower::util::BoxCloneService;
use tower::{ServiceBuilder, service_fn};
#[allow(deprecated)]
use tower_http::auth::require_authorization::Basic;
use tower_http::validate_request::ValidateRequestHeaderLayer;
use tracing::{debug, warn};

use crate::auth::ServerAuthenticator;
use crate::auth::jwt::Config as JwtAuthenticationConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig as SpireAuthConfig;
use crate::errors::ConfigError;
use crate::server::{AuthenticationConfig as ServerAuthConfig, ServerConfig};
use crate::tls::common::RustlsConfigLoader;
use crate::transport::TransportProtocol;
use crate::websocket::query_token_layer::QueryTokenToAuthHeaderLayer;

use super::common::{UpgradedWebSocket, WebSocketEndpoint};

/// Maximum time allowed for the TLS handshake to complete after accepting a
/// TCP connection. Prevents a silent or malicious client from pinning an
/// accept task indefinitely.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum time allowed for a client to complete the HTTP request and
/// WebSocket upgrade. Once the upgrade succeeds the connection is no longer
/// bound by this timeout.
const HTTP_UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard ceiling on the number of in-flight accept tasks when the
/// configured `max_concurrent_streams` is `None`. Prevents an unbounded
/// connection storm from exhausting memory / file descriptors on
/// misconfigured servers.
const DEFAULT_MAX_WEBSOCKET_CONNECTIONS: usize = 1024;

/// Unified server-side stream: either a plain TCP stream or a TLS-wrapped
/// TCP stream. Lets [`serve_connection`] be invoked with a single concrete
/// type regardless of whether TLS is enabled.
enum MaybeTlsStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl AsyncRead for MaybeTlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Both inner types are `Unpin`, so projecting through `&mut *self`
        // is safe without `pin-project`.
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTlsStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTlsStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTlsStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTlsStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

pub struct AcceptedWebSocketConnection {
    pub websocket: UpgradedWebSocket,
    pub remote_addr: Option<SocketAddr>,
    pub local_addr: Option<SocketAddr>,
}

pub type OnAcceptedWebSocket = Arc<
    dyn Fn(AcceptedWebSocketConnection) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// Resolved auth layers selected by [`ServerAuthConfig`]. The variant is
/// built once at server startup and cloned into each accepted connection so
/// the layer stack is composed at the point [`http1::Builder::serve_connection_with_upgrades`]
/// is invoked. Each variant resolves to a different concrete tower
/// `Service` type, which is why dispatch happens via `match` inside
/// [`serve_connection`] rather than through a single boxed service.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum AuthKind {
    None,
    Basic(#[allow(deprecated)] ValidateRequestHeaderLayer<Basic<Empty<Bytes>>>),
    Jwt(ValidateJwtLayer<MetadataMap, VerifierJwt>),
    #[cfg(not(target_family = "windows"))]
    Spire(ValidateJwtLayer<MetadataMap, SpireIdentityManager>),
}

async fn build_auth_kind(config: &ServerConfig) -> Result<AuthKind, ConfigError> {
    match &config.auth {
        ServerAuthConfig::None => Ok(AuthKind::None),
        ServerAuthConfig::Basic(basic) => {
            let layer = <_ as ServerAuthenticator<Empty<Bytes>>>::get_server_layer(basic)?;
            Ok(AuthKind::Basic(layer))
        }
        ServerAuthConfig::Jwt(jwt) => {
            let mut layer = <JwtAuthenticationConfig as ServerAuthenticator<
                Response<Empty<Bytes>>,
            >>::get_server_layer(jwt)?;
            layer.initialize().await?;
            Ok(AuthKind::Jwt(layer))
        }
        #[cfg(not(target_family = "windows"))]
        ServerAuthConfig::Spire(spire) => {
            let mut layer =
                <SpireAuthConfig as ServerAuthenticator<Response<Empty<Bytes>>>>::get_server_layer(
                    spire,
                )?;
            layer.initialize().await?;
            Ok(AuthKind::Spire(layer))
        }
    }
}

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
            (true, None) => return Err(ConfigError::WebSocketServerTlsMissing),
            (false, Some(_)) => return Err(ConfigError::WebSocketServerTlsUnexpected),
            (false, None) => None,
        };

        let auth_kind = build_auth_kind(self).await?;
        let expected_path = endpoint.path.clone();

        // Cap the number of concurrent accept tasks
        let max_connections = self
            .max_concurrent_streams
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_WEBSOCKET_CONNECTIONS)
            .max(1);
        let connection_semaphore = Arc::new(Semaphore::new(max_connections));

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

                        // Acquire a slot before spawning. If the cap is
                        // exhausted, drop the connection rather than
                        // queueing it up
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(
                                    max_connections,
                                    %remote_addr,
                                    "websocket connection cap reached; rejecting client"
                                );
                                drop(stream);
                                continue;
                            }
                        };

                        let local_addr = stream.local_addr().ok();
                        let auth_kind = auth_kind.clone();
                        let expected_path = expected_path.clone();
                        let on_accepted = on_accepted.clone();
                        let tls_acceptor = tls_acceptor.clone();

                        tokio::spawn(async move {
                            let stream = match tls_acceptor {
                                Some(acceptor) => {
                                    // Bound TLS handshake duration so a
                                    // silent/malicious client cannot hold a
                                    // task forever.
                                    match tokio::time::timeout(
                                        TLS_HANDSHAKE_TIMEOUT,
                                        acceptor.accept(stream),
                                    )
                                    .await
                                    {
                                        Ok(Ok(stream)) => MaybeTlsStream::Tls(Box::new(stream)),
                                        Ok(Err(err)) => {
                                            warn!(error = %err, "websocket TLS accept error");
                                            // permit dropped here, slot released
                                            return;
                                        }
                                        Err(_) => {
                                            warn!(
                                                timeout = ?TLS_HANDSHAKE_TIMEOUT,
                                                "websocket TLS handshake timed out"
                                            );
                                            return;
                                        }
                                    }
                                }
                                None => MaybeTlsStream::Plain(stream),
                            };

                            // Ownership of `permit` is moved into
                            // `serve_connection`, which in turn hands it
                            // to the spawned websocket task so the slot
                            // stays reserved for the lifetime of the
                            // active websocket (not just the upgrade).
                            serve_connection(
                                stream,
                                auth_kind,
                                expected_path,
                                on_accepted,
                                remote_addr,
                                local_addr,
                                permit,
                            )
                            .await;
                        });
                    }
                }
            }
        });

        Ok(cancellation_token)
    }
}

async fn serve_connection<S>(
    stream: S,
    auth_kind: AuthKind,
    expected_path: String,
    on_accepted: OnAcceptedWebSocket,
    remote_addr: SocketAddr,
    local_addr: Option<SocketAddr>,
    permit: OwnedSemaphorePermit,
) where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = TokioIo::new(stream);

    // Tracks whether the WebSocket upgrade succeeded. Used to enforce a
    // timeout on the *upgrade* portion of the connection only; once the
    // upgrade is complete the websocket itself may stay open indefinitely.
    let upgrade_done = Arc::new(AtomicBool::new(false));
    let upgrade_done_service = upgrade_done.clone();

    // The connection permit is moved into the upgrade task on a successful
    // upgrade so that the semaphore slot is held for the entire websocket
    // lifetime. If the upgrade never happens (404/400/401/timeout), the
    // permit is dropped here when `serve_connection` returns.
    let permit_slot = Arc::new(parking_lot::Mutex::new(Some(permit)));
    let permit_slot_service = permit_slot.clone();

    let inner = service_fn(move |mut request: Request<Incoming>| {
        let expected_path = expected_path.clone();
        let on_accepted = on_accepted.clone();
        let upgrade_done = upgrade_done_service.clone();
        let permit_slot = permit_slot_service.clone();

        let fut: Pin<Box<dyn Future<Output = Result<Response<Empty<Bytes>>, Infallible>> + Send>> =
            Box::pin(async move {
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

                match upgrade::upgrade(&mut request) {
                    Ok((response, future)) => {
                        // Mark the upgrade as successful so the connection
                        // future is no longer bound by `HTTP_UPGRADE_TIMEOUT`.
                        upgrade_done.store(true, Ordering::SeqCst);
                        // Transfer the semaphore permit out of the accept
                        // task and into the websocket task: the connection
                        // cap must bound *active websockets*, not just the
                        // brief upgrade phase.
                        let permit = permit_slot.lock().take();
                        tokio::spawn(async move {
                            let _permit = permit;
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
            });
        fut
    });

    // Bound the HTTP/WS upgrade phase so a client that opens a TCP/TLS
    // connection but never sends a valid upgrade request cannot pin this
    // task. The timeout only applies until `upgrade_done` flips: once the
    // upgrade succeeds the websocket IO is hijacked into its own task and
    // the remaining `serve_connection` future may resolve at any time
    // without being killed by the upgrade timeout.
    let upgrade_deadline = tokio::time::sleep(HTTP_UPGRADE_TIMEOUT);
    tokio::pin!(upgrade_deadline);

    // Each auth variant produces a different concrete tower `Service` type
    // (the static Layer stacking precludes a single variable), so we
    // type-erase each variant into a `BoxCloneService` with identical
    // signatures and then hand it to hyper via `TowerToHyperService`. The
    // dispatch mirrors the gRPC server's auth-layer construction
    // (`grpc/server.rs`).
    let svc: BoxCloneService<Request<Incoming>, Response<Empty<Bytes>>, Infallible> =
        match auth_kind {
            AuthKind::None => BoxCloneService::new(inner),
            AuthKind::Basic(layer) => {
                BoxCloneService::new(ServiceBuilder::new().layer(layer).service(inner))
            }
            AuthKind::Jwt(layer) => {
                // QueryTokenToAuthHeaderLayer promotes `?token=<jwt>` to an
                // `Authorization: Bearer <jwt>` header so browser clients —
                // which cannot set custom headers on `new WebSocket(url)` —
                // can still authenticate. Existing headers win over query.
                BoxCloneService::new(
                    ServiceBuilder::new()
                        .layer(QueryTokenToAuthHeaderLayer::new())
                        .layer(layer)
                        .service(inner),
                )
            }
            #[cfg(not(target_family = "windows"))]
            AuthKind::Spire(layer) => BoxCloneService::new(
                ServiceBuilder::new()
                    .layer(QueryTokenToAuthHeaderLayer::new())
                    .layer(layer)
                    .service(inner),
            ),
        };

    let connection = http1::Builder::new()
        .serve_connection(io, TowerToHyperService::new(svc))
        .with_upgrades();
    tokio::pin!(connection);

    tokio::select! {
        biased;
        result = &mut connection => {
            if let Err(err) = result {
                debug!(error = %err, "websocket HTTP connection closed with error");
            }
        }
        _ = &mut upgrade_deadline, if !upgrade_done.load(Ordering::SeqCst) => {
            warn!(
                timeout = ?HTTP_UPGRADE_TIMEOUT,
                "websocket HTTP upgrade timed out"
            );
        }
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
        let test_user = format!("user-{}", std::process::id());
        let test_pass = format!("pw-{}-{}", std::process::id(), port);
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure())
            .with_auth(ServerAuthConfig::Basic(BasicConfig::new(
                &test_user, &test_pass,
            )));
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

        assert!(channel.remote_addr().is_some());
        // local_addr is unavailable when driving the upgrade via hyper-util's
        // legacy Client — its HttpInfo extension only exposes remote_addr.
        assert!(channel.local_addr().is_none());

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

    #[tokio::test]
    async fn test_websocket_server_enforces_max_connections() {
        let port = available_port();
        let cfg = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_settings(TlsServerConfig::insecure())
            .with_max_concurrent_streams(Some(1));

        let on_accepted: OnAcceptedWebSocket = Arc::new(|_| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
            })
        });

        let (signal, watch) = drain::channel();
        std::mem::forget(signal);
        let token = cfg
            .run_websocket_server(watch, on_accepted)
            .await
            .expect("server start");

        let client_cfg = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure())
            .with_connect_timeout(Duration::from_secs(5))
            .with_backoff(crate::client::BackoffConfig::new_fixed_interval(
                Duration::from_millis(0),
                1,
            ));

        let mut first = None;
        for _ in 0..40 {
            match tokio::time::timeout(Duration::from_secs(5), client_cfg.to_websocket_channel())
                .await
            {
                Ok(Ok(ch)) => {
                    first = Some(ch);
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let _hold_first = first.expect("first handshake never succeeded");

        tokio::time::sleep(Duration::from_millis(200)).await;

        let second =
            tokio::time::timeout(Duration::from_secs(3), client_cfg.to_websocket_channel()).await;
        let inner = second.expect("second handshake outer timeout");
        assert!(
            inner.is_err(),
            "second handshake must fail when max_concurrent_streams cap is reached"
        );

        token.cancel();
    }
}

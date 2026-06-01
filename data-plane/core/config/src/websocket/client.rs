// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bytes::Bytes;
use fastwebsockets::{Role, WebSocket, handshake};
use http::header::{CONNECTION, HOST, ORIGIN, UPGRADE, USER_AGENT};
use http::header::{HeaderName, HeaderValue};
use http::uri::Authority;
use http::{Request, Response, StatusCode};
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpInfo;
use hyper_util::rt::{TokioExecutor, TokioIo};
use sha1::{Digest, Sha1};
use tower::{ServiceBuilder, ServiceExt};
use tracing::warn;

use crate::auth::ClientAuthenticator;
use crate::client::{AuthenticationConfig as ClientAuthConfig, ClientConfig};
use crate::errors::ConfigError;
use crate::transport::TransportProtocol;
use crate::transport_common::{Alpn, ProxyTunnel, build_https_connector};

use super::common::{UpgradedWebSocket, WebSocketEndpoint};

/// The error type emitted by hyper-util's legacy Client. Auth and bearer→query
/// layers preserve `S::Error`, so the whole stack produces this single type.
type ClientError = hyper_util::client::legacy::Error;

#[derive(Clone)]
pub struct WebSocketClientChannel {
    inner: Arc<WebSocketClientChannelInner>,
}

struct WebSocketClientChannelInner {
    /// The upgraded websocket. Wrapped in `Mutex<Option<_>>` because the
    /// underlying `fastwebsockets::WebSocket` can only be `.split()` once;
    /// callers consume it via [`WebSocketClientChannel::take_websocket`].
    websocket: parking_lot::Mutex<Option<UpgradedWebSocket>>,
    local_addr: Option<SocketAddr>,
    remote_addr: Option<SocketAddr>,
}

impl WebSocketClientChannel {
    pub(crate) fn new(
        websocket: UpgradedWebSocket,
        local_addr: Option<SocketAddr>,
        remote_addr: Option<SocketAddr>,
    ) -> Self {
        Self {
            inner: Arc::new(WebSocketClientChannelInner {
                websocket: parking_lot::Mutex::new(Some(websocket)),
                local_addr,
                remote_addr,
            }),
        }
    }

    /// Take ownership of the underlying upgraded websocket. Returns `None`
    /// if it has already been taken — the websocket can only be split into
    /// read/write halves once.
    pub fn take_websocket(&self) -> Option<UpgradedWebSocket> {
        self.inner.websocket.lock().take()
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.inner.local_addr
    }

    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.inner.remote_addr
    }
}

impl ClientConfig {
    /// Build a WebSocket channel. Crate-private; external callers should use
    /// [`ClientConfig::to_channel`].
    pub(crate) async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        if self.resolved_transport() != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketClientUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;

        self.retry_connect(|| self.connect_websocket_once(&endpoint))
            .await
    }

    async fn connect_websocket_once(
        &self,
        endpoint: &WebSocketEndpoint,
    ) -> Result<WebSocketClientChannel, ConfigError> {
        // Build the upgrade request. We use http:// / https:// scheme since
        // hyper rejects ws:// / wss://.
        let request_uri = endpoint.http_request_uri()?;
        let request = build_handshake_request(self, endpoint, request_uri)?;
        let sent_key = request
            .headers()
            .get("Sec-WebSocket-Key")
            .cloned()
            .expect("Sec-WebSocket-Key set by build_handshake_request");

        // Single budget covering connector setup + TLS + proxy CONNECT + HTTP
        // request. The retry layer above counts the *attempt*, not each leg.
        let total_timeout: Duration = self.connect_timeout.into();
        let mut response: Response<Incoming> = self
            .build_and_send(endpoint, request, total_timeout)
            .await?;

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(ConfigError::WebSocketHandshakeStatus(response.status()));
        }
        verify_sec_websocket_accept(&response, sent_key.as_bytes())?;

        // HttpInfo is attached to the response by hyper-util's legacy Client.
        // Pull remote_addr off before we consume the response via upgrade::on.
        let remote_addr = response
            .extensions()
            .get::<HttpInfo>()
            .map(|info: &HttpInfo| info.remote_addr());

        let upgraded = hyper::upgrade::on(&mut response)
            .await
            .map_err(|e| ConfigError::WebSocketConnection(std::io::Error::other(e)))?;

        let websocket = WebSocket::after_handshake(TokioIo::new(upgraded), Role::Client);

        Ok(WebSocketClientChannel::new(
            websocket,
            // hyper-util's HttpInfo only exposes remote_addr().
            None,
            remote_addr,
        ))
    }

    /// Build the connector chain, layer auth + bearer→query on top, and send
    /// the upgrade request. Each `match` arm calls into the generic
    /// [`Self::send_via`] / [`Self::send_authed`] so the concrete connector
    /// type is monomorphized end-to-end — no boxing required.
    async fn build_and_send(
        &self,
        endpoint: &WebSocketEndpoint,
        request: Request<Empty<Bytes>>,
        timeout: Duration,
    ) -> Result<Response<Incoming>, ConfigError> {
        let base = self.create_http_connector()?;
        let tls = if endpoint.secure {
            self.load_tls_config().await?
        } else {
            None
        };
        if endpoint.secure && tls.is_none() {
            return Err(ConfigError::WebSocketServerTlsMissing);
        }

        // hyper-util's proxy `Matcher` needs an http/https URI; the WS one
        // built with ws:// / wss:// schemes is rejected.
        let target_for_proxy = format!(
            "{}://{}{}",
            if endpoint.secure { "https" } else { "http" },
            endpoint.authority,
            endpoint.path,
        );
        let proxy_hit = self.proxy.should_use_proxy(&target_for_proxy);

        let server_name = self
            .server_name
            .clone()
            .or_else(|| self.origin.as_deref().and_then(host_from_authority))
            .unwrap_or_else(|| endpoint.host.clone());

        match (proxy_hit, tls) {
            (None, None) => self.send_via(build_client(base), request, timeout).await,
            (None, Some(tls)) => {
                let https = build_https_connector(base, tls, Some(server_name), Alpn::Http1);
                self.send_via(build_client(https), request, timeout).await
            }
            (Some(intercept), None) => match self
                .build_proxy_tunnel(intercept, base, Alpn::Http1)
                .await?
            {
                ProxyTunnel::Http(t) => self.send_via(build_client(t), request, timeout).await,
                ProxyTunnel::Https(t) => self.send_via(build_client(t), request, timeout).await,
            },
            (Some(intercept), Some(tls)) => match self
                .build_proxy_tunnel(intercept, base, Alpn::Http1)
                .await?
            {
                ProxyTunnel::Http(t) => {
                    let https = build_https_connector(t, tls, Some(server_name), Alpn::Http1);
                    self.send_via(build_client(https), request, timeout).await
                }
                ProxyTunnel::Https(t) => {
                    let https = build_https_connector(t, tls, Some(server_name), Alpn::Http1);
                    self.send_via(build_client(https), request, timeout).await
                }
            },
        }
    }

    /// Initialize the auth layer (if any), wrap `client` with it, and dispatch
    /// the request. Generic over the connector type so no type erasure is
    /// needed.
    async fn send_via<C>(
        &self,
        client: Client<C, Empty<Bytes>>,
        request: Request<Empty<Bytes>>,
        timeout: Duration,
    ) -> Result<Response<Incoming>, ConfigError>
    where
        C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
    {
        match &self.auth {
            ClientAuthConfig::None => run_send(client.oneshot(request), timeout).await,
            ClientAuthConfig::Basic(basic) => {
                let layer = basic.get_client_layer()?;
                self.warn_insecure_auth();
                let svc = ServiceBuilder::new().layer(layer).service(client);
                run_send(svc.oneshot(request), timeout).await
            }
            ClientAuthConfig::StaticJwt(jwt) => {
                let mut layer = jwt.get_client_layer()?;
                layer.initialize().await?;
                self.warn_insecure_auth();
                let svc = ServiceBuilder::new().layer(layer).service(client);
                run_send(svc.oneshot(request), timeout).await
            }
            ClientAuthConfig::Jwt(jwt) => {
                let mut layer = jwt.get_client_layer()?;
                layer.initialize().await?;
                self.warn_insecure_auth();
                let svc = ServiceBuilder::new().layer(layer).service(client);
                run_send(svc.oneshot(request), timeout).await
            }
            #[cfg(not(target_family = "windows"))]
            ClientAuthConfig::Spire(spire) => {
                let mut layer = spire.get_client_layer()?;
                layer.initialize().await?;
                self.warn_insecure_auth();
                let svc = ServiceBuilder::new().layer(layer).service(client);
                run_send(svc.oneshot(request), timeout).await
            }
        }
    }
}

async fn run_send<F>(send: F, timeout: Duration) -> Result<Response<Incoming>, ConfigError>
where
    F: std::future::Future<Output = Result<Response<Incoming>, ClientError>>,
{
    if timeout.is_zero() {
        send.await.map_err(box_send_err)
    } else {
        match tokio::time::timeout(timeout, send).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(box_send_err(e)),
            Err(_) => Err(ConfigError::WebSocketHandshakeTimeout),
        }
    }
}

fn build_client<C>(connector: C) -> Client<C, Empty<Bytes>>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    Client::builder(TokioExecutor::new())
        .pool_idle_timeout(Duration::from_secs(0))
        .pool_max_idle_per_host(0)
        .build(connector)
}

fn box_send_err<E>(err: E) -> ConfigError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ConfigError::WebSocketClientSend(Box::new(err))
}

fn build_handshake_request(
    config: &ClientConfig,
    endpoint: &WebSocketEndpoint,
    uri: http::Uri,
) -> Result<Request<Empty<Bytes>>, ConfigError> {
    const DEFAULT_USER_AGENT: &str = concat!("slim-websocket/", env!("CARGO_PKG_VERSION"));

    // RFC 7230 §5.4: Host MUST be the URI's authority with userinfo (and the
    // '@' delimiter) removed, and otherwise byte-identical.
    let host_header = strip_userinfo(&endpoint.authority);

    let mut request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(HOST, host_header)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Key", handshake::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .body(Empty::<Bytes>::new())
        .map_err(ConfigError::WebSocketRequest)?;

    let headers = request.headers_mut();

    if let Some(origin) = config.origin.as_deref() {
        headers.insert(ORIGIN, HeaderValue::from_str(origin)?);
    }

    // Iterate config.headers in sorted order: HashMap iteration is
    // non-deterministic, which would otherwise make request bytes vary
    // between attempts (and break any snapshot-style tests).
    let mut extra: Vec<(&String, &String)> = config.headers.iter().collect();
    extra.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in extra {
        let header_name = HeaderName::from_str(name)?;
        // Filter reserved headers
        if is_reserved_handshake_header(&header_name) {
            warn!(
                header = %header_name,
                "ignoring reserved websocket handshake header supplied via config.headers",
            );
            continue;
        }
        headers.insert(header_name, HeaderValue::from_str(value)?);
    }

    Ok(request)
}

fn is_reserved_handshake_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "upgrade"
            | "connection"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-accept"
            | "sec-websocket-extensions"
            | "sec-websocket-protocol"
    )
}

/// Return `authority` with any leading `userinfo@` removed. Preserves the
/// rest of the string verbatim (including absence-of-port), which is what
/// RFC 7230 §5.4 requires for the Host header and what host-based ingress
/// routing rules expect.
fn strip_userinfo(authority: &str) -> &str {
    match authority.rfind('@') {
        Some(pos) => &authority[pos + 1..],
        None => authority,
    }
}

/// Strip any `:port` suffix from an HTTP authority, returning just the host.
///
/// IPv6 hosts in URI authorities are bracketed (`[::1]:8080`); delegating to
/// [`http::uri::Authority`] handles that. Used to derive an SNI name from
/// the `Origin` header when configured.
fn host_from_authority(authority: &str) -> Option<String> {
    if authority.is_empty() {
        return None;
    }
    let parsed = Authority::from_str(authority).ok()?;
    let host = parsed.host();
    if host.is_empty() {
        return None;
    }
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    Some(unbracketed.to_string())
}

/// RFC 6455 §4.1: the server must echo `base64(sha1(client_key + GUID))` in
/// `Sec-WebSocket-Accept`. `fastwebsockets::WebSocket::after_handshake`
/// trusts the caller to have validated this, so we do it here.
fn verify_sec_websocket_accept<B>(
    response: &Response<B>,
    sent_key: &[u8],
) -> Result<(), ConfigError> {
    const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

    let header = response
        .headers()
        .get("Sec-WebSocket-Accept")
        .ok_or(ConfigError::WebSocketMissingAcceptHeader)?;
    let received = header
        .to_str()
        .map_err(|_| ConfigError::WebSocketAcceptMismatch)?;

    let mut hasher = Sha1::new();
    hasher.update(sent_key);
    hasher.update(GUID);
    let expected = BASE64_STANDARD.encode(hasher.finalize());

    if expected == received {
        Ok(())
    } else {
        Err(ConfigError::WebSocketAcceptMismatch)
    }
}

// =====================================================================
// Tests
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;

    use crate::tls::client::TlsClientConfig;
    use std::net::TcpListener;
    use std::time::Duration;

    fn available_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("local_addr")
            .port()
    }

    #[tokio::test]
    async fn test_websocket_client_invalid_endpoint_scheme() {
        let cfg = ClientConfig::with_endpoint("http://127.0.0.1:80")
            .with_tls_setting(TlsClientConfig::insecure());
        let result = cfg.to_websocket_channel().await;
        assert!(result.is_err(), "non-ws scheme must be rejected");
    }

    #[tokio::test]
    async fn test_websocket_client_connect_refused() {
        // Bind to grab a port, then drop the listener to guarantee it's closed.
        let port = available_port();
        let cfg = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_tls_setting(TlsClientConfig::insecure())
            .with_backoff(crate::client::BackoffConfig::new_fixed_interval(
                Duration::from_millis(0),
                1,
            ));

        let result = cfg.to_websocket_channel().await;
        assert!(result.is_err(), "connection to closed port should fail");
    }

    #[tokio::test]
    async fn test_websocket_client_connect_timeout() {
        let _env_guard = crate::test_env::PROXY_ENV_LOCK
            .lock()
            .expect("proxy env lock");

        // Isolate from shell/parallel tests that set HTTP_PROXY (see grpc::proxy tests).
        #[allow(clippy::disallowed_methods)]
        let saved_proxy_env = [
            ("http_proxy", std::env::var("http_proxy").ok()),
            ("HTTP_PROXY", std::env::var("HTTP_PROXY").ok()),
            ("https_proxy", std::env::var("https_proxy").ok()),
            ("HTTPS_PROXY", std::env::var("HTTPS_PROXY").ok()),
            ("all_proxy", std::env::var("all_proxy").ok()),
            ("ALL_PROXY", std::env::var("ALL_PROXY").ok()),
        ];
        for (key, _) in &saved_proxy_env {
            unsafe {
                std::env::remove_var(key);
            }
        }

        // RFC 5737 TEST-NET-1: guaranteed unroutable.
        let cfg = ClientConfig::with_endpoint("ws://192.0.2.1:9")
            .with_tls_setting(TlsClientConfig::insecure())
            .with_connect_timeout(Duration::from_millis(200))
            .with_backoff(crate::client::BackoffConfig::new_fixed_interval(
                Duration::from_millis(0),
                0,
            ));

        let start = std::time::Instant::now();
        let outer = tokio::time::timeout(Duration::from_secs(2), cfg.to_websocket_channel()).await;
        let elapsed = start.elapsed();

        assert!(outer.is_ok(), "configured connect_timeout was not honored");
        assert!(outer.unwrap().is_err(), "unroutable connect must fail");
        assert!(
            elapsed < Duration::from_secs(1),
            "connect_timeout was not honored (took {elapsed:?})"
        );

        for (key, original) in saved_proxy_env {
            unsafe {
                match original {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn test_host_from_authority_strips_port() {
        assert_eq!(
            host_from_authority("example.com:8443"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_host_from_authority_no_port() {
        assert_eq!(
            host_from_authority("example.com"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_host_from_authority_empty() {
        assert_eq!(host_from_authority(""), None);
    }

    #[test]
    fn test_host_from_authority_ipv6_with_port() {
        // IPv6 literal: brackets must not be split as a port separator.
        assert_eq!(
            host_from_authority("[::1]:8080"),
            Some("::1".to_string()),
            "IPv6 host must be extracted intact"
        );
    }

    #[test]
    fn test_host_from_authority_ipv6_without_port() {
        assert_eq!(
            host_from_authority("[2001:db8::1]"),
            Some("2001:db8::1".to_string()),
        );
    }

    #[test]
    fn test_is_reserved_handshake_header_matches_case_insensitively() {
        for name in [
            "host",
            "Host",
            "HOST",
            "upgrade",
            "connection",
            "sec-websocket-key",
            "Sec-WebSocket-Key",
            "sec-websocket-version",
            "sec-websocket-accept",
            "sec-websocket-extensions",
            "sec-websocket-protocol",
        ] {
            let h = HeaderName::from_str(name).expect("valid header name");
            assert!(
                is_reserved_handshake_header(&h),
                "{name} should be reserved"
            );
        }
    }

    #[test]
    fn test_is_reserved_handshake_header_allows_unrelated() {
        for name in ["authorization", "origin", "x-trace-id", "user-agent"] {
            let h = HeaderName::from_str(name).expect("valid header name");
            assert!(
                !is_reserved_handshake_header(&h),
                "{name} must not be reserved"
            );
        }
    }

    fn handshake_request_for(cfg: &ClientConfig) -> Request<Empty<Bytes>> {
        let endpoint = WebSocketEndpoint::parse(cfg.endpoint.as_str()).expect("endpoint");
        let request_uri = endpoint.http_request_uri().expect("uri");
        build_handshake_request(cfg, &endpoint, request_uri).expect("build")
    }

    #[test]
    fn test_handshake_request_sets_required_headers() {
        let cfg = ClientConfig::with_endpoint("ws://example.com:8080/p");
        let req = handshake_request_for(&cfg);
        let h = req.headers();
        assert_eq!(h.get(HOST).unwrap(), "example.com:8080");
        assert_eq!(h.get(UPGRADE).unwrap(), "websocket");
        assert_eq!(h.get(CONNECTION).unwrap(), "Upgrade");
        assert_eq!(h.get("Sec-WebSocket-Version").unwrap(), "13");
        assert!(h.get("Sec-WebSocket-Key").is_some());
        assert!(
            h.get(USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.starts_with("slim-websocket/"))
                .unwrap_or(false),
            "User-Agent should default to slim-websocket/<version>"
        );
    }

    #[test]
    fn test_handshake_request_host_omits_default_ws_port() {
        let cfg = ClientConfig::with_endpoint("ws://api.example.com/p");
        let req = handshake_request_for(&cfg);
        assert_eq!(req.headers().get(HOST).unwrap(), "api.example.com");
    }

    #[test]
    fn test_handshake_request_host_omits_default_wss_port() {
        let cfg = ClientConfig::with_endpoint("wss://api.example.com/p");
        let req = handshake_request_for(&cfg);
        assert_eq!(req.headers().get(HOST).unwrap(), "api.example.com");
    }

    #[test]
    fn test_handshake_request_host_preserves_explicit_port() {
        let cfg = ClientConfig::with_endpoint("ws://api.example.com:80/p");
        let req = handshake_request_for(&cfg);
        assert_eq!(req.headers().get(HOST).unwrap(), "api.example.com:80");
    }

    #[test]
    fn test_handshake_request_host_excludes_userinfo() {
        let cfg = ClientConfig::with_endpoint("ws://user:pw@example.com:8080/p");
        let req = handshake_request_for(&cfg);
        let host = req.headers().get(HOST).unwrap().to_str().unwrap();
        assert!(
            !host.contains('@'),
            "Host header must not carry userinfo: {host}"
        );
        assert_eq!(host, "example.com:8080");
    }

    #[test]
    fn test_handshake_request_host_for_ipv6() {
        let cfg = ClientConfig::with_endpoint("ws://[::1]:9000/p");
        let req = handshake_request_for(&cfg);
        assert_eq!(req.headers().get(HOST).unwrap(), "[::1]:9000");
    }

    #[test]
    fn test_strip_userinfo() {
        assert_eq!(strip_userinfo("example.com:8080"), "example.com:8080");
        assert_eq!(strip_userinfo("u@example.com"), "example.com");
        assert_eq!(strip_userinfo("u:p@example.com:443"), "example.com:443");
        assert_eq!(strip_userinfo("[::1]:9000"), "[::1]:9000");
        assert_eq!(strip_userinfo("a@b@example.com"), "example.com");
    }

    #[test]
    fn test_handshake_request_user_headers_cannot_clobber_reserved() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("Upgrade".to_string(), "evil".to_string());
        headers.insert("Connection".to_string(), "close".to_string());
        headers.insert(
            "Sec-WebSocket-Key".to_string(),
            "AAAAAAAAAAAAAAAAAAAAAA==".to_string(),
        );
        headers.insert("Sec-WebSocket-Version".to_string(), "8".to_string());
        headers.insert("Host".to_string(), "attacker.example".to_string());
        let cfg = ClientConfig::with_endpoint("ws://example.com:8080/p").with_headers(headers);
        let req = handshake_request_for(&cfg);
        let h = req.headers();
        assert_eq!(h.get(UPGRADE).unwrap(), "websocket");
        assert_eq!(h.get(CONNECTION).unwrap(), "Upgrade");
        assert_eq!(h.get("Sec-WebSocket-Version").unwrap(), "13");
        assert_eq!(h.get(HOST).unwrap(), "example.com:8080");
        let key = h.get("Sec-WebSocket-Key").unwrap().to_str().unwrap();
        assert_ne!(key, "AAAAAAAAAAAAAAAAAAAAAA==");
    }

    #[test]
    fn test_handshake_request_allows_user_supplied_non_reserved_headers() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("X-Trace-Id".to_string(), "abc-123".to_string());
        let cfg = ClientConfig::with_endpoint("ws://example.com:8080/p").with_headers(headers);
        let req = handshake_request_for(&cfg);
        assert_eq!(req.headers().get("X-Trace-Id").unwrap(), "abc-123");
    }

    #[test]
    fn test_verify_sec_websocket_accept_matches_rfc_example() {
        // RFC 6455 §1.3 worked example.
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let expected = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
        let response: Response<()> = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("Sec-WebSocket-Accept", expected)
            .body(())
            .unwrap();
        verify_sec_websocket_accept(&response, key.as_bytes()).expect("matches");
    }

    #[test]
    fn test_verify_sec_websocket_accept_missing_header() {
        let response: Response<()> = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(())
            .unwrap();
        assert!(matches!(
            verify_sec_websocket_accept(&response, b"key"),
            Err(ConfigError::WebSocketMissingAcceptHeader)
        ));
    }

    #[test]
    fn test_verify_sec_websocket_accept_wrong_digest() {
        let response: Response<()> = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("Sec-WebSocket-Accept", "AAAAAAAAAAAAAAAAAAAAAAAAAAA=")
            .body(())
            .unwrap();
        assert!(matches!(
            verify_sec_websocket_accept(&response, b"dGhlIHNhbXBsZSBub25jZQ=="),
            Err(ConfigError::WebSocketAcceptMismatch)
        ));
    }
}

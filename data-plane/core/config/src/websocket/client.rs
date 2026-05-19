// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use bytes::Bytes;
use fastwebsockets::handshake;
use http::header::{AUTHORIZATION, CONNECTION, HOST, ORIGIN, UPGRADE};
use http_body_util::Empty;
use hyper::Request;
use hyper::header::{HeaderName, HeaderValue};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::client::ClientConfig;
use crate::errors::ConfigError;
use crate::tls::common::RustlsConfigLoader;
use crate::transport::TransportProtocol;

use super::common::{
    ClientHandshakeAuth, UpgradedWebSocket, WebSocketEndpoint, build_client_handshake_auth,
};

pub struct WebSocketClientChannel {
    pub websocket: UpgradedWebSocket,
    pub local_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
}

impl ClientConfig {
    /// Build a WebSocket channel. Crate-private; external callers should use
    /// [`ClientConfig::to_channel`].
    pub(crate) async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        if self.transport != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketClientUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;
        let auth = build_client_handshake_auth(self).await?;

        let query_param = self
            .websocket_auth_query_param
            .as_deref()
            .zip(auth.bearer_token.as_deref());

        let request_uri = endpoint.request_uri(query_param)?;
        let request = build_handshake_request(self, &endpoint, request_uri, &auth)?;

        let stream = connect_tcp(self, &endpoint).await?;
        let local_addr = stream.local_addr().ok();
        let remote_addr = stream.peer_addr().ok();

        let websocket = if endpoint.secure {
            let tls_config = self.tls_setting.load_rustls_config().await?;
            let tls_config = tls_config.ok_or(ConfigError::WebSocketTlsConfiguration)?;
            let connector = TlsConnector::from(Arc::new(tls_config));

            // Pick the SNI / certificate verification name in this order:
            //   1. `server_name` — explicit SNI override, highest precedence.
            //   2. `origin` host part — if the caller spoofed the Host
            //      header, they almost certainly want certificate
            //      validation against the same name, otherwise the cert
            //      check fails on a name they didn't pick. We strip any
            //      `:port` because SNI takes only the hostname.
            //   3. `endpoint.host` — the actual connect target; last
            //      resort default.
            let server_name = self
                .server_name
                .as_deref()
                .or_else(|| self.origin.as_deref().and_then(host_from_authority))
                .unwrap_or(endpoint.host.as_str());
            let server_name =
                tokio_rustls::rustls::pki_types::ServerName::try_from(server_name.to_string())
                    .map_err(|_| ConfigError::WebSocketTlsConfiguration)?;

            let tls_stream = connector
                .connect(server_name, stream)
                .await
                .map_err(ConfigError::WebSocketConnection)?;

            handshake::client(&SpawnExecutor, request, tls_stream)
                .await
                .map_err(ConfigError::WebSocketHandshake)?
                .0
        } else {
            handshake::client(&SpawnExecutor, request, stream)
                .await
                .map_err(ConfigError::WebSocketHandshake)?
                .0
        };

        Ok(WebSocketClientChannel {
            websocket,
            local_addr,
            remote_addr,
        })
    }
}

fn build_handshake_request(
    config: &ClientConfig,
    endpoint: &WebSocketEndpoint,
    uri: http::Uri,
    auth: &ClientHandshakeAuth,
) -> Result<Request<Empty<Bytes>>, ConfigError> {
    let mut request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(HOST, endpoint.authority.as_str())
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header("Sec-WebSocket-Key", handshake::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::<Bytes>::new())
        .map_err(ConfigError::WebSocketRequest)?;

    let headers = request.headers_mut();

    if let Some(origin) = config.origin.as_deref() {
        headers.insert(ORIGIN, HeaderValue::from_str(origin)?);
    }

    if let Some(auth_header) = auth.authorization_header.as_deref() {
        headers.insert(AUTHORIZATION, HeaderValue::from_str(auth_header)?);
    }

    for (name, value) in &config.headers {
        headers.insert(HeaderName::from_str(name)?, HeaderValue::from_str(value)?);
    }

    Ok(request)
}

async fn connect_tcp(
    config: &ClientConfig,
    endpoint: &WebSocketEndpoint,
) -> Result<TcpStream, ConfigError> {
    let connect = TcpStream::connect(endpoint.socket_address());
    let timeout: std::time::Duration = config.connect_timeout.into();

    if timeout.is_zero() {
        return connect.await.map_err(ConfigError::WebSocketConnection);
    }

    match tokio::time::timeout(timeout, connect).await {
        Ok(result) => result.map_err(ConfigError::WebSocketConnection),
        Err(_) => Err(ConfigError::WebSocketConnection(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "websocket connect timeout",
        ))),
    }
}

struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::task::spawn(fut);
    }
}

/// Strip any `:port` suffix from an HTTP authority, returning just the host.
///
/// Used to derive an SNI name from the `Origin` header value (e.g.
/// `example.com:8443` -> `example.com`). Returns `None` for empty input so
/// callers can fall back to the next name in the chain.
fn host_from_authority(authority: &str) -> Option<&str> {
    let host = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    if host.is_empty() { None } else { Some(host) }
}

// =====================================================================
// Tests
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;

    use crate::tls::client::TlsClientConfig;
    use crate::transport::TransportProtocol;
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
    async fn test_websocket_client_wrong_transport() {
        // Default transport is gRPC, not websocket -> must error.
        let cfg = ClientConfig::with_endpoint("ws://127.0.0.1:1");
        let result = cfg.to_websocket_channel().await;
        assert!(matches!(
            result,
            Err(ConfigError::WebSocketClientUnsupportedTransport)
        ));
    }

    #[tokio::test]
    async fn test_websocket_client_invalid_endpoint_scheme() {
        let cfg = ClientConfig::with_endpoint("http://127.0.0.1:80")
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure());
        let result = cfg.to_websocket_channel().await;
        assert!(result.is_err(), "non-ws scheme must be rejected");
    }

    #[tokio::test]
    async fn test_websocket_client_connect_refused() {
        // Bind to grab a port, then drop the listener to guarantee it's closed.
        let port = available_port();
        let cfg = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure());

        let result = cfg.to_websocket_channel().await;
        assert!(result.is_err(), "connection to closed port should fail");
    }

    #[tokio::test]
    async fn test_websocket_client_connect_timeout() {
        // RFC 5737 TEST-NET-1: guaranteed unroutable.
        let cfg = ClientConfig::with_endpoint("ws://192.0.2.1:9")
            .with_transport(TransportProtocol::Websocket)
            .with_tls_setting(TlsClientConfig::insecure())
            .with_connect_timeout(Duration::from_millis(200));

        let start = std::time::Instant::now();
        let outer = tokio::time::timeout(Duration::from_secs(2), cfg.to_websocket_channel()).await;
        let elapsed = start.elapsed();

        assert!(outer.is_ok(), "configured connect_timeout was not honored");
        assert!(outer.unwrap().is_err(), "unroutable connect must fail");
        assert!(
            elapsed < Duration::from_secs(1),
            "connect_timeout was not honored (took {elapsed:?})"
        );
    }

    #[test]
    fn test_host_from_authority_strips_port() {
        assert_eq!(host_from_authority("example.com:8443"), Some("example.com"));
    }

    #[test]
    fn test_host_from_authority_no_port() {
        assert_eq!(host_from_authority("example.com"), Some("example.com"));
    }

    #[test]
    fn test_host_from_authority_empty() {
        assert_eq!(host_from_authority(""), None);
    }
}

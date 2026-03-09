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
use crate::grpc::errors::ConfigError;
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
    pub async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        if self.transport != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketClientUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;
        let auth = build_client_handshake_auth(self).await?;

        // TODO(hackeramitkumar): In query-param mode we should suppress Authorization headers
        // for browser-compat behavior and rely only on the configured query parameter.
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

            let server_name = self
                .server_name
                .as_deref()
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

    // TODO(hackeramitkumar): Skip this header when websocket_auth_query_param is active.
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;

    #[test]
    fn build_handshake_request_sets_required_and_optional_headers() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:8080/ws").expect("endpoint");
        let uri = endpoint.request_uri(None).expect("uri");

        let mut headers = HashMap::new();
        headers.insert("x-test-header".to_string(), "x-value".to_string());
        let config = ClientConfig::with_endpoint("ws://localhost:8080/ws")
            .with_transport(TransportProtocol::Websocket)
            .with_origin("https://example.com")
            .with_headers(headers);

        let auth = ClientHandshakeAuth {
            authorization_header: Some("Bearer test-token".to_string()),
            bearer_token: Some("test-token".to_string()),
        };

        let request = build_handshake_request(&config, &endpoint, uri, &auth).expect("request");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(request.uri().to_string(), "ws://localhost:8080/ws");

        let headers = request.headers();
        assert_eq!(
            headers.get(HOST).and_then(|v| v.to_str().ok()),
            Some("localhost:8080")
        );
        assert_eq!(
            headers.get(UPGRADE).and_then(|v| v.to_str().ok()),
            Some("websocket")
        );
        assert_eq!(
            headers.get(CONNECTION).and_then(|v| v.to_str().ok()),
            Some("upgrade")
        );
        assert_eq!(
            headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
            Some("Bearer test-token")
        );
        assert_eq!(
            headers.get(ORIGIN).and_then(|v| v.to_str().ok()),
            Some("https://example.com")
        );
        assert_eq!(
            headers.get("x-test-header").and_then(|v| v.to_str().ok()),
            Some("x-value")
        );
        assert!(headers.contains_key("Sec-WebSocket-Key"));
        assert_eq!(
            headers
                .get("Sec-WebSocket-Version")
                .and_then(|v| v.to_str().ok()),
            Some("13")
        );
    }

    #[test]
    fn build_handshake_request_rejects_invalid_origin_header() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:8080/ws").expect("endpoint");
        let uri = endpoint.request_uri(None).expect("uri");
        let config = ClientConfig::with_endpoint("ws://localhost:8080/ws")
            .with_transport(TransportProtocol::Websocket)
            .with_origin("https://example.com\ninvalid");

        let err = build_handshake_request(&config, &endpoint, uri, &ClientHandshakeAuth::default())
            .expect_err("invalid origin must fail");
        assert!(matches!(err, ConfigError::HeaderValueParse(_)));
    }

    #[test]
    fn build_handshake_request_rejects_invalid_custom_header_name() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:8080/ws").expect("endpoint");
        let uri = endpoint.request_uri(None).expect("uri");
        let mut headers = HashMap::new();
        headers.insert("bad header".to_string(), "value".to_string());
        let config = ClientConfig::with_endpoint("ws://localhost:8080/ws")
            .with_transport(TransportProtocol::Websocket)
            .with_headers(headers);

        let err = build_handshake_request(&config, &endpoint, uri, &ClientHandshakeAuth::default())
            .expect_err("invalid header name must fail");
        assert!(matches!(err, ConfigError::HeaderNameParse(_)));
    }

    #[test]
    fn build_handshake_request_rejects_invalid_auth_header() {
        let endpoint = WebSocketEndpoint::parse("ws://localhost:8080/ws").expect("endpoint");
        let uri = endpoint.request_uri(None).expect("uri");
        let config = ClientConfig::with_endpoint("ws://localhost:8080/ws")
            .with_transport(TransportProtocol::Websocket);
        let auth = ClientHandshakeAuth {
            authorization_header: Some("Bearer invalid\nvalue".to_string()),
            bearer_token: None,
        };

        let err = build_handshake_request(&config, &endpoint, uri, &auth)
            .expect_err("invalid auth header must fail");
        assert!(matches!(err, ConfigError::HeaderValueParse(_)));
    }

    #[tokio::test]
    async fn connect_tcp_with_zero_timeout_connects_successfully() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener bind");
        let port = listener.local_addr().expect("local addr").port();

        let accept_task = tokio::spawn(async move {
            let _ = listener.accept().await.expect("accept");
        });

        let endpoint =
            WebSocketEndpoint::parse(&format!("ws://127.0.0.1:{port}/ws")).expect("endpoint parse");
        let config = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}/ws"))
            .with_transport(TransportProtocol::Websocket)
            .with_connect_timeout(Duration::ZERO);

        let stream = connect_tcp(&config, &endpoint).await.expect("connect");
        assert!(stream.peer_addr().is_ok());
        drop(stream);
        accept_task.await.expect("accept task");
    }

    #[tokio::test]
    async fn to_websocket_channel_rejects_non_websocket_transport() {
        let config = ClientConfig::with_endpoint("http://127.0.0.1:12345");
        let err = match config.to_websocket_channel().await {
            Ok(_) => panic!("must reject grpc transport"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            ConfigError::WebSocketClientUnsupportedTransport
        ));
    }

    #[tokio::test]
    async fn to_websocket_channel_rejects_invalid_endpoint_scheme() {
        let config = ClientConfig::with_endpoint("http://127.0.0.1:12345")
            .with_transport(TransportProtocol::Websocket);
        let err = match config.to_websocket_channel().await {
            Ok(_) => panic!("must reject non ws scheme"),
            Err(err) => err,
        };
        assert!(matches!(err, ConfigError::InvalidWebSocketEndpointScheme));
    }
}

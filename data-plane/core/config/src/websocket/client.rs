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

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;

#[cfg(target_arch = "wasm32")]
use super::common::{WebSocketEndpoint, build_client_handshake_auth};
use crate::client::ClientConfig;
use crate::grpc::errors::ConfigError;
#[cfg(target_arch = "wasm32")]
use crate::transport::TransportProtocol;

#[cfg(target_arch = "wasm32")]
use gloo_net::websocket::futures::WebSocket;

#[cfg(target_arch = "wasm32")]
pub struct WebSocketClientChannel {
    pub websocket: WebSocket,
    pub local_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct WebSocketClientChannel {
    pub local_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
}

#[cfg(target_arch = "wasm32")]
impl ClientConfig {
    pub async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        if self.transport != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketClientUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;
        let auth = build_client_handshake_auth(self).await?;

        if auth.authorization_header.is_some() && self.websocket_auth_query_param.is_none() {
            return Err(ConfigError::WebSocketWasmAuthorizationHeaderUnsupported);
        }

        let query_param = self
            .websocket_auth_query_param
            .as_deref()
            .zip(auth.bearer_token.as_deref());
        let request_uri = endpoint.request_uri(query_param)?;

        let websocket = WebSocket::open(request_uri.to_string().as_str())
            .map_err(|err| ConfigError::WebSocketWasmConnection(err.to_string()))?;

        Ok(WebSocketClientChannel {
            websocket,
            local_addr: None,
            remote_addr: None,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientConfig {
    pub async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        let _ = self;
        Err(ConfigError::WebSocketWasmUnsupportedTarget)
    }
}

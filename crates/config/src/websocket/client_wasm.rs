// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser WebSocket client channel.
//!
//! Mirrors the native `websocket::client` surface (`WebSocketClientChannel` +
//! `ClientConfig::to_websocket_channel`) but opens the connection with
//! `gloo_net`'s JS `WebSocket`. The browser performs the TLS handshake for
//! `wss://`, so there is no TLS/proxy/auth plumbing here; any auth token is
//! expected to travel in the endpoint query string (browsers cannot set
//! handshake headers).

use std::cell::RefCell;
use std::net::SocketAddr;

use gloo_net::websocket::futures::WebSocket;

use crate::client::ClientConfig;
use crate::errors::ConfigError;
use crate::transport::TransportProtocol;

use super::common::WebSocketEndpoint;

/// Browser counterpart of the native `WebSocketClientChannel`.
///
/// The underlying `gloo_net` `WebSocket` is `!Send`/`!Sync` and can only be
/// split once, so it is held in a `RefCell<Option<_>>` and consumed via
/// [`WebSocketClientChannel::take_websocket`]. This matches the
/// take-once contract of the native channel.
pub struct WebSocketClientChannel {
    websocket: RefCell<Option<WebSocket>>,
}

impl WebSocketClientChannel {
    pub(crate) fn new(websocket: WebSocket) -> Self {
        Self {
            websocket: RefCell::new(Some(websocket)),
        }
    }

    /// Take ownership of the underlying socket. Returns `None` if it has
    /// already been taken.
    pub fn take_websocket(&self) -> Option<WebSocket> {
        self.websocket.borrow_mut().take()
    }

    /// Browser sockets do not expose the local socket address.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        None
    }

    /// Browser sockets do not expose the peer socket address.
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }
}

impl ClientConfig {
    /// Open a browser WebSocket channel. Crate-private; external callers should
    /// use [`ClientConfig::to_channel`].
    pub(crate) async fn to_websocket_channel(&self) -> Result<WebSocketClientChannel, ConfigError> {
        if self.resolved_transport() != TransportProtocol::Websocket {
            return Err(ConfigError::WebSocketClientUnsupportedTransport);
        }

        let endpoint = WebSocketEndpoint::parse(self.endpoint.as_str())?;
        // `request_uri()` keeps the ws:// / wss:// scheme and preserves any
        // existing query string (e.g. an auth token), which is what the browser
        // `WebSocket` constructor expects.
        let url = endpoint.request_uri()?.to_string();

        let websocket =
            WebSocket::open(&url).map_err(|e| ConfigError::WebSocketConnection(e.to_string()))?;

        Ok(WebSocketClientChannel::new(websocket))
    }
}

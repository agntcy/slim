// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-build configuration errors.
//!
//! The native [`super::ConfigError`] (in `errors/native.rs`) covers
//! gRPC/TLS/auth/native-WebSocket; the wasm build only needs the surface
//! exercised by `ClientConfig` validation and the gloo-net connect.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("missing endpoint")]
    MissingEndpoint,
    #[error("invalid endpoint scheme")]
    InvalidEndpointScheme,
    #[error("websocket transport requires endpoint scheme ws:// or wss://")]
    InvalidWebSocketEndpointScheme,
    #[error("websocket client builder requires websocket transport")]
    WebSocketClientUnsupportedTransport,
    #[error("URI parse error")]
    UriParse(#[from] http::uri::InvalidUri),
    #[error("websocket connection error: {0}")]
    WebSocketConnection(String),
}

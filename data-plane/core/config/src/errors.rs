// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::auth::ConfigAuthError;
use slim_auth::errors::AuthError;
use thiserror::Error;

/// Errors for Config.
/// This is a custom error type for handling configuration-related errors.
/// It is used to provide more context to the error messages.
#[derive(Error, Debug)]
pub enum ConfigError {
    // Configuration / required fields
    #[error("missing the grpc server service")]
    MissingServices,
    #[error("missing grpc endpoint")]
    MissingEndpoint,

    // Address / parsing
    #[error("endpoint parse error")]
    EndpointParse(#[from] std::net::AddrParseError),
    #[error("URI parse error")]
    UriParse(#[from] http::uri::InvalidUri),
    #[error("invalid endpoint scheme")]
    InvalidEndpointScheme,
    #[error("websocket transport requires endpoint scheme ws:// or wss://")]
    InvalidWebSocketEndpointScheme,
    #[error("websocket client builder requires websocket transport")]
    WebSocketClientUnsupportedTransport,
    #[error("websocket server builder requires websocket transport")]
    WebSocketServerUnsupportedTransport,
    #[error("websocket transport TLS configuration is invalid")]
    WebSocketTlsConfiguration,
    #[error("websocket server TLS is required but no TLS configuration was provided")]
    WebSocketServerTlsMissing,
    #[error("HTTPS proxy URL requires a TLS configuration but none was provided")]
    ProxyTlsMissing,
    #[error("websocket server TLS configuration is set but endpoint scheme is not wss://")]
    WebSocketServerTlsUnexpected,
    #[error("websocket server SNI / certificate name is invalid")]
    WebSocketInvalidServerName,
    #[error("websocket TLS handshake error")]
    WebSocketTlsHandshake(#[source] std::io::Error),
    #[error("websocket TLS handshake timed out")]
    WebSocketTlsHandshakeTimeout,
    #[error("websocket protocol upgrade timed out")]
    WebSocketHandshakeTimeout,
    #[error("websocket handshake returned non-101 status: {0}")]
    WebSocketHandshakeStatus(http::StatusCode),
    #[error("websocket handshake response missing Sec-WebSocket-Accept header")]
    WebSocketMissingAcceptHeader,
    #[error("websocket handshake response Sec-WebSocket-Accept did not match the sent key")]
    WebSocketAcceptMismatch,
    #[error("websocket client send error")]
    WebSocketClientSend(#[source] Box<dyn std::error::Error + Send + Sync>),

    // Network / transport
    #[error("transport error")]
    TransportError(#[from] tonic::transport::Error),
    #[error("gRPC channel builder does not support websocket transport")]
    GrpcChannelUnsupportedTransport,
    #[error("gRPC server builder does not support websocket transport")]
    GrpcServerUnsupportedTransport,
    #[error("bind error")]
    Bind(#[from] std::io::Error),
    #[error("websocket connection error")]
    WebSocketConnection(#[source] std::io::Error),
    #[error("websocket handshake error")]
    WebSocketHandshake(#[source] fastwebsockets::WebSocketError),
    #[error("websocket request error")]
    WebSocketRequest(#[source] http::Error),

    // Unix domain sockets
    #[error("unix domain sockets are unsupported on this platform")]
    UnixSocketUnsupported,
    #[error("unix domain socket endpoint requires a socket path")]
    UnixSocketMissingPath,
    #[error("unix domain sockets require tls.insecure=true")]
    UnixSocketTlsUnsupported,
    #[error("invalid unix domain socket path")]
    UnixSocketInvalidPath(#[source] http::Error),

    #[cfg(target_family = "unix")]
    #[error("failed to connect to unix domain socket")]
    UnixSocketConnect(#[source] std::io::Error),

    // Header parsing
    #[error("header name parse error")]
    HeaderNameParse(#[from] http::header::InvalidHeaderName),
    #[error("header value parse error")]
    HeaderValueParse(#[from] http::header::InvalidHeaderValue),

    // Rate limiting
    #[error("rate limit parse error")]
    RateLimitParse(#[from] std::num::ParseIntError),

    // TLS configuration
    #[error("TLS config error")]
    TlsConfig(#[from] crate::tls::errors::ConfigError),

    // Authentication
    #[error("auth error")]
    AuthError(#[from] AuthError),
    #[error("auth config error")]
    AuthConfigError(#[from] ConfigAuthError),

    // Resolution / operational
    #[error("resolution error")]
    ResolutionError,

    // Link negotiation
    #[error("link_id must be a valid UUID v4")]
    InvalidLinkId,

    // ServerHandler routing
    #[error("server handler does not provide gRPC routes, but transport is gRPC")]
    HandlerMissingGrpcSupport,
    #[error("server handler does not provide a websocket callback, but transport is websocket")]
    HandlerMissingWebSocketSupport,

    // Unknown / catch-all
    #[error("unknown error")]
    Unknown,
}

impl ConfigError {
    /// True if this error came from a transient transport-level failure that
    /// a connect attempt could plausibly recover from on retry (TCP refused,
    /// TLS handshake hiccup, websocket upgrade race, etc.). Used by the
    /// shared connect-retry helper to decide whether to keep retrying.
    ///
    /// Add new variants here when introducing a transport — that's the single
    /// switch that opts a new transport into the shared retry policy.
    pub fn is_retryable_connect_error(&self) -> bool {
        match self {
            // gRPC / tonic
            ConfigError::TransportError(_) => true,

            // Unix domain socket
            #[cfg(target_family = "unix")]
            ConfigError::UnixSocketConnect(_) => true,

            // WebSocket
            ConfigError::WebSocketConnection(_)
            | ConfigError::WebSocketTlsHandshake(_)
            | ConfigError::WebSocketTlsHandshakeTimeout
            | ConfigError::WebSocketHandshake(_)
            | ConfigError::WebSocketHandshakeTimeout
            | ConfigError::WebSocketHandshakeStatus(_)
            | ConfigError::WebSocketMissingAcceptHeader
            | ConfigError::WebSocketAcceptMismatch
            | ConfigError::WebSocketClientSend(_) => true,

            _ => false,
        }
    }
}

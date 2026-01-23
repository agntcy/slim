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

    // Network / transport
    #[error("transport error")]
    TransportError(#[from] tonic::transport::Error),
    #[error("bind error")]
    Bind(#[from] std::io::Error),

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

    // Unknown / catch-all
    #[error("unknown error")]
    Unknown,
}

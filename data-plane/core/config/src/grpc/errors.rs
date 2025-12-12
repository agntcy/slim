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
    #[error("endpoint parse error: {0}")]
    EndpointParse(#[from] std::net::AddrParseError),
    #[error("URI parse error: {0}")]
    UriParse(#[from] http::uri::InvalidUri),

    // Network / transport
    #[error("transport error: {0}")]
    TransportError(#[from] tonic::transport::Error),
    #[error("bind error: {0}")]
    Bind(#[from] std::io::Error),

    // Header parsing
    #[error("header name parse error: {0}")]
    HeaderNameParse(#[from] http::header::InvalidHeaderName),
    #[error("header value parse error: {0}")]
    HeaderValueParse(#[from] http::header::InvalidHeaderValue),

    // Rate limiting
    #[error("rate limit parse error: {0}")]
    RateLimitParse(#[from] std::num::ParseIntError),

    // TLS configuration
    #[error("TLS config error: {0}")]
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

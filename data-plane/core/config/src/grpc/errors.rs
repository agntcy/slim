// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;
use slim_auth::errors::AuthError;
use crate::auth::ConfigAuthError as ConfigAuthError;

/// Errors for Config.
/// This is a custom error type for handling configuration-related errors.
/// It is used to provide more context to the error messages.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("missing the grpc server service")]
    MissingServices,
    #[error("missing grpc endpoint")]
    MissingEndpoint,
    #[error("endpoint parse error: {0}")]
    EndpointParse(#[from] std::net::AddrParseError),
    #[error("tcp incoming error: {0}")]
    TcpIncoming(#[from] tonic::transport::Error),
    #[error("bind error: {0}")]
    Bind(#[from] std::io::Error),
    #[error("URI parse error: {0}")]
    UriParse(#[from] http::uri::InvalidUri),
    #[error("header name parse error: {0}")]
    HeaderNameParse(#[from] http::header::InvalidHeaderName),
    #[error("header value parse error: {0}")]
    HeaderValueParse(#[from] http::header::InvalidHeaderValue),
    #[error("rate limit parse error: {0}")]
    RateLimitParse(#[from] std::num::ParseIntError),
    #[error("TLS config error: {0}")]
    TlsConfig(#[from] crate::tls::common::ConfigError),
    #[error("auth error")]
    AuthError(#[from] AuthError),
    #[error("auth config error")]
    AuthConfigError(#[from] ConfigAuthError),
    #[error("resolution error")]
    ResolutionError,
    #[error("unknown error")]
    Unknown,
}

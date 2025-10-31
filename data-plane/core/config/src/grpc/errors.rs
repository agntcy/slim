// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

/// Errors for Config.
/// This is a custom error type for handling configuration-related errors.
/// It is used to provide more context to the error messages.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("missing the grpc server service")]
    MissingServices,
    #[error("missing grpc endpoint")]
    MissingEndpoint,
    #[error("error parsing grpc endpoint: {0}")]
    EndpointParseError(String),
    #[error("tcp incoming error: {0}")]
    TcpIncomingError(String),
    #[error("failed to parse uri: {0}]")]
    UriParseError(String),
    #[error("failed to parse headers: {0}")]
    HeaderParseError(String),
    #[error("failed to parse rate limit configuration: {0}")]
    RateLimitParseError(String),
    #[error("tls setting error: {0}")]
    TLSSettingError(String),
    #[error("auth config error: {0}")]
    AuthConfigError(String),
    #[error("resolution error")]
    ResolutionError,
    #[error("invalid uri: {0}")]
    InvalidUri(String),
    #[error("unknown error")]
    Unknown,
}

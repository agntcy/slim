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
    #[error("error parsing grpc endpoint")]
    EndpointParseError(String),
    #[error("tcp incoming error")]
    TcpIncomingError(String),
    #[error("failed to parse uri")]
    UriParseError(String),
    #[error("failed to parse headers")]
    HeaderParseError(String),
    #[error("failed to parse rate limit configuration")]
    RateLimitParseError(String),
    #[error("tls setting error: {0}")]
    TLSSettingError(String),
    #[error("auth config error: {0}")]
    AuthConfigError(String),
    #[error("resolution error")]
    ResolutionError,
    #[error("unknown error")]
    Unknown,
}

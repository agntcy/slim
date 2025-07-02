// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("config error: {0}")]
    ConfigError(String),

    #[error("configuration error: {0}")]
    ConfigurationError(String),

    #[error("token expired")]
    TokenExpired,

    #[error("token invalid: {0}")]
    TokenInvalid(String),

    #[error("signing error: {0}")]
    SigningError(String),

    #[error("get token error: {0}")]
    GetTokenError(String),

    #[error("verification error: {0}")]
    VerificationError(String),

    #[error("invalid header: {0}")]
    InvalidHeader(String),

    #[error("network error: {0}")]
    NetworkError(String),

    #[error("authorization error: {0}")]
    AuthorizationError(String),

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("fallback allow - continuing due to AuthZEN unavailability")]
    FallbackAllow,
}

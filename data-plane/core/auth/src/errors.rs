// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    // Configuration / generic
    #[error("config error: {0}")]
    ConfigError(String),

    // Time
    #[error("time error: {0}")]
    TimeError(#[from] std::time::SystemTimeError),

    // URL parsing
    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    // Header parsing
    #[error("invalid header: {0}")]
    InvalidHeader(String),
    #[error("invalid header name: {0}")]
    HeaderNameError(#[from] http::header::InvalidHeaderName),
    #[error("invalid header value: {0}")]
    HeaderValueError(#[from] http::header::InvalidHeaderValue),

    // File watcher
    #[error("file watcher error: {0}")]
    FileWatcherError(#[from] crate::file_watcher::FileWatcherError),

    // Token lifecycle
    #[error("get token error: {0}")]
    GetTokenError(String),
    #[error("token expired")]
    TokenExpired,
    #[error("token invalid: {0}")]
    TokenInvalid(String),
    #[error("OAuth2 token expired and refresh failed: {0}")]
    TokenRefreshFailed(String),

    // Signing / verification
    #[error("signing error: {0}")]
    SigningError(String),
    #[error("verification error: {0}")]
    VerificationError(String),

    // JWT / crypto
    #[error("JWT library error: {0}")]
    JwtLibraryError(#[from] jsonwebtoken_aws_lc::errors::Error),

    // HTTP / networking
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    // Serialization
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    // OAuth2 generic
    #[error("OAuth2 error: {0}")]
    OAuth2Error(String),
    #[error("OAuth2 request error: {0}")]
    OAuth2Request(Box<dyn std::error::Error + Send + Sync>),
    #[error("Token endpoint error: status {status}, body: {body}")]
    TokenEndpointError { status: u16, body: String },
    #[error("Invalid client credentials")]
    InvalidClientCredentials,
    #[error("Invalid issuer endpoint URL: {0}")]
    InvalidIssuerEndpoint(String),

    // Unsupported / operational
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),
    #[error("operation would block on async I/O; call async variant")]
    WouldBlockOn,
}

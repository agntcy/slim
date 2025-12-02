// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use spiffe::{SpiffeIdError, error::GrpcClientError, workload_api::x509_source::X509SourceError};
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
    #[error("JWT error: {0}")]
    JwtLibraryError(#[from] jsonwebtoken_aws_lc::errors::Error),

    // HTTP / networking
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    // JWKS / key resolution
    // (Do not use #[from] here to avoid conflicting From impls with existing HttpError/JsonError)
    #[error("failed to fetch JWKS: {source}")]
    JwksFetch { source: reqwest::Error },
    #[error("failed to read JWKS response: {source}")]
    JwksRead { source: reqwest::Error },
    #[error("failed to parse JWKS: {source}")]
    JwksParse { source: serde_json::Error },
    #[error("no suitable key found in JWKS for token header")]
    JwksNoSuitableKey,
    #[error("no cached JWKS for issuer: {issuer}")]
    JwksCacheMiss { issuer: String },
    #[error("openid discovery document missing jwks_uri field")]
    OidcDiscoveryMissingJwksUri,
    #[error("cached JWKS expired for issuer: {issuer}")]
    JwksCacheExpired { issuer: String },
    #[error("serde error while encoding audience: {source}")]
    SpiffeCustomClaimsSerialize { source: serde_json::Error },

    // SPIFFE / SPIRE integration
    #[error("spiffe error: {0}")]
    SpiffeError(#[from] SpiffeIdError),
    #[error("spiffe grpc error: {0}")]
    SpiffeGrpcError(#[from] GrpcClientError),
    #[error("spiffe workload api unavailable")]
    SpiffeWorkloadApiUnavailable,
    #[error("spiffe x509 dource error: {0}")]
    SpiffeX509SourceError(#[from] X509SourceError),
    #[error("jwt source not initialized")]
    SpiffeJwtSourceNotInitialized,
    #[error("missing jwt svid")]
    SpiffeJwtSvidMissing,
    #[error("missing jwt bundle")]
    SpiffeJwtBundleMissing,
    #[error("failed to fetch x509 SVID")]
    SpiffeX509SvidMissing,
    #[error("x509 source not initialized")]
    SpiffeX509SourceNotInitialized,
    #[error("x509 trust bundle not available: {0}")]
    SpiffeX509BundleMissing(String),
    #[error("error fetching x509 SVID: {0}")]
    SpiffeX509SvidFetch(String),
    #[error("error fetching x509 trust bundle: {0}")]
    SpiffeX509BundleFetch(String),
    #[error("jwt source closed")]
    SpiffeCustomAudiencesJwtSourceClosed,
    #[error("error fetching jwt svid with custom audiences")]
    SpiffeCustomAudiencesError,

    // Serialization
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64DecodeError(#[from] base64::DecodeError),

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

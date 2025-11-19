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

    // SPIFFE / SPIRE integration
    #[error("failed to connect to SPIFFE Workload API: {details}")]
    SpiffeWorkloadConnect { details: String },
    #[error("SPIFFE X509 source not initialized")]
    SpiffeX509SourceNotInitialized,
    #[error("SPIFFE JWT source not initialized")]
    SpiffeJwtSourceNotInitialized,
    #[error("failed to initialize SPIFFE X509 source: {details}")]
    SpiffeX509Init { details: String },
    #[error("failed to initialize SPIFFE JWT source: {details}")]
    SpiffeJwtInit { details: String },
    #[error("failed to get X509 SVID: {details}")]
    SpiffeX509SvidFetch { details: String },
    #[error("no X509 SVID available")]
    SpiffeX509SvidMissing,
    #[error("failed to get JWT SVID: {details}")]
    SpiffeJwtSvidFetch { details: String },
    #[error("no JWT SVID available")]
    SpiffeJwtSvidMissing,
    #[error("failed to fetch X509 bundle: {details}")]
    SpiffeX509BundleFetch { details: String },
    #[error("invalid trust domain {td}: {details}")]
    SpiffeTrustDomainInvalid { td: String, details: String },
    #[error("no X509 bundle for trust domain {td}")]
    SpiffeTrustDomainBundleMissing { td: String },
    #[error("SPIFFE JWT bundles not yet available")]
    SpiffeJwtBundlesUnavailable,
    #[error("failed to serialize custom claims: {source}")]
    SpiffeCustomClaimsSerialize { source: serde_json::Error },
    #[error("JWT source task has shut down")]
    SpiffeJwtSourceTaskShutdown,
    #[error("failed to receive response from JWT source: {details}")]
    SpiffeJwtSourceResponse { details: String },

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

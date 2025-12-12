// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use http::StatusCode;
use jsonwebtoken_aws_lc::jwk::KeyAlgorithm;
use spiffe::{
    JwtSvidError, SpiffeIdError, TrustDomain, error::GrpcClientError,
    workload_api::x509_source::X509SourceError,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    // JWT errors
    #[error("unsupported key algorithm: {0}")]
    JwtUnsupportedKeyAlgorithm(KeyAlgorithm),
    #[error("JWK does not contain the key algorithm (alg) field")]
    JwtMissingKeyAlgorithm,
    #[error("no private key available for signing")]
    JwtMissingPrivateKey,
    #[error("missing decoding key or autoresolve is disabled")]
    JwtMissingDecodingKeyOrKeyResolver,
    #[error("missing 'iss' in JWT claims")]
    JwtMissingIssuer,
    #[error("no key resolver available")]
    JwtNoKeyResolver,
    #[error("no static JWT token configured")]
    JwtNoStaticTokenConfigured,
    #[error("JWK format not supported for encoding (signing) keys")]
    JwtJwkFormatNotSupportedForEncoding,
    #[error("failed to fetch JWKS for issuer - status_code: {0}")]
    JwtFetchJwksFailed(StatusCode),
    #[error("StaticTokenProvider does not support custom claims")]
    JwtStaticUnsupportedCustomClaims,

    // OIDC/Oauth2 errors
    #[error("token_endpoint not found in discovery document")]
    OidcDiscoveryMissingTokenEndpoint,
    #[error("key not found: {0}")]
    OidcKeyNotFound(String),
    #[error("kid is missing and multiple keys are available")]
    OidcMissingKidWithMultipleKeys,
    #[error("OIDC Token Provider does not support custom claims")]
    OidcUnsupportedCustomClaims,
    #[error("OAuth2 request error: {0}")]
    OAuth2Request(Box<dyn std::error::Error + Send + Sync>),
    #[error("Token endpoint error: status {status}, body: {body}")]
    TokenEndpointError { status: u16, body: String },
    #[error("Invalid client credentials")]
    InvalidClientCredentials,

    // hmac
    #[error("hmac key is too short")]
    HmacKeyTooShort,
    #[error("hmac key is missing")]
    HmacKeyMissing,

    // Time
    #[error("time error: {0}")]
    TimeError(#[from] std::time::SystemTimeError),

    // URL parsing
    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    // Header parsing
    #[error("invalid header name: {0}")]
    HeaderNameError(#[from] http::header::InvalidHeaderName),
    #[error("invalid header value: {0}")]
    HeaderValueError(#[from] http::header::InvalidHeaderValue),

    // File watcher
    #[error("file watcher error: {0}")]
    FileWatcherError(#[from] crate::file_watcher::FileWatcherError),

    // Token lifecycle
    #[error("no token available")]
    GetTokenError,
    #[error("token invalid")]
    TokenInvalid2,
    #[error("token malformed")]
    TokenMalformed,
    #[error("token invalid: missing subject claim")]
    TokenInvalidMissingSub,
    #[error("token invalid: replay")]
    TokenInvalidReplay,
    #[error("token invalid: {0}")]
    JwtTokenInvalid(#[from] jsonwebtoken_aws_lc::errors::Error),
    #[error("token invalid - missing or invalid exp claim")]
    TokenInvalidMissingExp,

    // HTTP / networking
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    // JWKS / key resolution
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
    #[error("invalid JWT svid: {0}")]
    SpiffeInvalidJwtSvid(#[from] JwtSvidError),
    #[error("failed to fetch x509 SVID")]
    SpiffeX509SvidMissing,
    #[error("x509 source not initialized")]
    SpiffeX509SourceNotInitialized,
    #[error("x509 trust bundle not available: {0}")]
    SpiffeX509BundleMissing(TrustDomain),
    #[error("error fetching x509 SVID: {source}")]
    SpiffeX509SvidFetch {
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("error fetching x509 trust bundle: {source}")]
    SpiffeX509BundleFetch {
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("spire x509 empty certificate chain")]
    SpiffeX509EmptyCertChain,
    #[error("jwt source closed")]
    SpiffeCustomAudiencesJwtSourceClosed,
    #[error("error fetching jwt svid with custom audiences")]
    SpiffeCustomAudiencesError,

    // Serialization
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64DecodeError(#[from] base64::DecodeError),

    // Operational
    #[error("operation would block on async I/O; call async variant")]
    WouldBlockOn,
}

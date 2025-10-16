// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod basic;
pub mod jwt;
pub mod oidc;
pub mod spiffe;
pub mod static_jwt;

use slim_auth::errors::AuthError as SlimAuthError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("config error: {0}")]
    ConfigError(String),

    #[error("token expired")]
    TokenExpired,

    #[error("token invalid: {0}")]
    TokenInvalid(String),

    #[error("sign error: {0}")]
    SigningError(String),

    #[error("invalid header: {0}")]
    InvalidHeader(String),
}

impl From<SlimAuthError> for AuthError {
    fn from(err: SlimAuthError) -> Self {
        match err {
            SlimAuthError::ConfigError(msg) => AuthError::ConfigError(msg),
            SlimAuthError::TokenExpired => AuthError::TokenExpired,
            SlimAuthError::TokenInvalid(msg) => AuthError::TokenInvalid(msg),
            SlimAuthError::SigningError(msg) => AuthError::SigningError(msg),
            SlimAuthError::GetTokenError(msg) => {
                AuthError::ConfigError(format!("Get token error: {}", msg))
            }
            SlimAuthError::VerificationError(msg) => {
                AuthError::TokenInvalid(format!("Verification error: {}", msg))
            }
            SlimAuthError::InvalidHeader(msg) => AuthError::InvalidHeader(msg),
            SlimAuthError::JwtAwsLcError(e) => {
                AuthError::TokenInvalid(format!("JWT validation error: {}", e))
            }
            SlimAuthError::UnsupportedOperation(msg) => {
                AuthError::ConfigError(format!("Unsupported operation: {}", msg))
            }
            SlimAuthError::HttpError(e) => AuthError::ConfigError(format!("HTTP error: {}", e)),
            SlimAuthError::JsonError(e) => AuthError::ConfigError(format!("JSON error: {}", e)),
            SlimAuthError::OAuth2Error(msg) => {
                AuthError::ConfigError(format!("OAuth2 error: {}", msg))
            }
            SlimAuthError::TokenEndpointError { status, body } => {
                AuthError::ConfigError(format!("Token endpoint error ({}): {}", status, body))
            }
            SlimAuthError::InvalidClientCredentials => {
                AuthError::ConfigError("Invalid client credentials".to_string())
            }
            SlimAuthError::TokenRefreshFailed(msg) => {
                AuthError::ConfigError(format!("Token refresh failed: {}", msg))
            }
            SlimAuthError::InvalidIssuerEndpoint(msg) => {
                AuthError::ConfigError(format!("Invalid issuer endpoint: {}", msg))
            }
        }
    }
}

pub trait ClientAuthenticator {
    // associated types
    type ClientLayer;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError>;
}

pub trait ServerAuthenticator<Response: Default> {
    // associated types
    type ServerLayer;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError>;
}

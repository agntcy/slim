// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod basic;
pub mod jwt;
pub mod oidc;
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

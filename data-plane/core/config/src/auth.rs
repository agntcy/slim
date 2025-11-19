// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod basic;
pub mod jwt;
pub mod oidc;
#[cfg(not(target_family = "windows"))]
pub mod spire;
pub mod static_jwt;

use slim_auth::errors::AuthError as SlimAuthError;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigAuthError {
    // Configuration
    #[error("config error: {0}")]
    ConfigError(String),

    // Token lifecycle
    #[error("token expired")]
    TokenExpired,
    #[error("token invalid: {0}")]
    TokenInvalid(String),

    // Signing / headers
    #[error("sign error: {0}")]
    SigningError(String),
    #[error("invalid header: {0}")]
    InvalidHeader(String),

    // Propagated auth library errors
    #[error("internal auth error: {0}")]
    InternalError(#[from] SlimAuthError),
}

pub trait ClientAuthenticator {
    // associated types
    type ClientLayer;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, ConfigAuthError>;
}

pub trait ServerAuthenticator<Response: Default> {
    // associated types
    type ServerLayer;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, ConfigAuthError>;
}

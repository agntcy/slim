// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod basic;
pub mod jwt;
pub mod oidc;
#[cfg(not(target_family = "windows"))]
pub mod spire;
pub mod static_jwt;

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

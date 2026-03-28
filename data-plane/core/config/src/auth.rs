// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod basic;
#[cfg(feature = "native")]
pub mod identity;
#[cfg(feature = "native")]
pub mod jwt;
#[cfg(feature = "native")]
pub mod oidc;
#[cfg(all(
    not(target_family = "windows"),
    feature = "native"
))]
pub mod spire;
#[cfg(feature = "native")]
pub mod static_jwt;

#[cfg(feature = "native")]
use slim_auth::errors::AuthError as SlimAuthError;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigAuthError {
    // Configuration
    #[error("username cannot be empty")]
    AuthBasicEmptyUsername,
    #[error("password cannot be empty")]
    AuthBasicEmptyPassword,

    #[error("client id cannot be empty")]
    AuthOidcEmptyClientId,
    #[error("client secret cannot be empty")]
    AuthOidcEmptyClientSecret,

    // Propagated auth library errors
    #[cfg(feature = "native")]
    #[error("internal auth error")]
    AuthInternalError(#[from] SlimAuthError),

    // Verifier errors
    #[error("audience required")]
    AuthJwtAudienceRequired,
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

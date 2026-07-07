// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod app_auth;
pub mod basic;
pub mod identity;
pub mod jwt;
pub mod oidc;
#[cfg(not(target_family = "windows"))]
pub mod spire;
pub mod static_jwt;

pub use app_auth::AuthConfig;

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

    // App auth validation
    #[error("auth.secret cannot be empty for shared_secret")]
    AuthSecretEmpty,
    #[error("auth.socket_path must be set for spire")]
    AuthSpireSocketPathMissing,

    // Propagated auth library errors
    #[error("internal auth error")]
    AuthInternalError(#[from] SlimAuthError),

    // Verifier errors
    #[error("audience required")]
    AuthJwtAudienceRequired,

    // Identity config errors
    #[error("no identity provider configured")]
    IdentityProviderNotConfigured,
    #[error("no identity verifier configured")]
    IdentityVerifierNotConfigured,
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

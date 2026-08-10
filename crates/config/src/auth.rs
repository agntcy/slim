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

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Policy evaluated against JWT claims on every authenticated request.
///
/// YAML shape (externally tagged — use exactly one key):
/// ```yaml
/// policy:
///   rego: |
///     package slim.auth
///     default allow = false
///     allow if "admin" in input.claims.groups
///
/// policy:
///   rego_file: /etc/slim/auth.rego
///
/// policy:
///   cel: '"admin" in claims.groups'
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyConfig {
    /// Inline Rego policy. Must define `package slim.auth` with `default allow = false`.
    /// Claims available as `input.claims.*`.
    Rego(String),
    /// Path to a `.rego` file read at server startup.
    RegoFile(PathBuf),
    /// CEL expression that must evaluate to `true`.
    /// Claims available as `claims.*` (e.g. `"admin" in claims.groups`).
    Cel(String),
}

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

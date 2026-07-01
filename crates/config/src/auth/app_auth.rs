// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Application-level authentication configuration.
//!
//! [`AuthConfig`] provides a high-level enum that maps to the lower-level
//! [`IdentityProviderConfig`] and [`IdentityVerifierConfig`] types.

use serde::Deserialize;

use super::ConfigAuthError;
use super::identity::{IdentityProviderConfig, IdentityVerifierConfig};
#[cfg(not(target_family = "windows"))]
use super::spire::SpireConfig;

/// Authentication configuration for the SLIM app identity.
///
/// For `shared_secret`, an optional `id` can be provided. When omitted it
/// defaults to the `local-name` field of the manager configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthConfig {
    /// Shared secret authentication (symmetric key)
    SharedSecret {
        /// Identity id. Defaults to `local-name` when not provided.
        id: Option<String>,
        /// The shared secret value
        secret: String,
    },
    /// SPIRE-based identity (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    Spire(SpireConfig),
}

impl AuthConfig {
    /// Validate the auth configuration fields.
    pub fn validate(&self) -> Result<(), ConfigAuthError> {
        match self {
            AuthConfig::SharedSecret { secret, .. } => {
                if secret.is_empty() {
                    return Err(ConfigAuthError::AuthSecretEmpty);
                }
            }
            #[cfg(not(target_family = "windows"))]
            AuthConfig::Spire(spire_config) => {
                if spire_config.socket_path.is_none() {
                    return Err(ConfigAuthError::AuthSpireSocketPathMissing);
                }
            }
        }
        Ok(())
    }

    /// Convert to IdentityProviderConfig + IdentityVerifierConfig pair.
    /// For shared_secret, uses the explicit `id` if provided, otherwise
    /// falls back to `local_name`.
    pub fn to_identity_configs(
        &self,
        local_name: &str,
    ) -> (IdentityProviderConfig, IdentityVerifierConfig) {
        match self {
            AuthConfig::SharedSecret { id, secret } => {
                let identity_id = id.as_deref().unwrap_or(local_name).to_string();
                (
                    IdentityProviderConfig::SharedSecret {
                        id: identity_id.clone(),
                        data: secret.clone(),
                    },
                    IdentityVerifierConfig::SharedSecret {
                        id: identity_id,
                        data: secret.clone(),
                    },
                )
            }
            #[cfg(not(target_family = "windows"))]
            AuthConfig::Spire(spire_config) => (
                IdentityProviderConfig::Spire(spire_config.clone()),
                IdentityVerifierConfig::Spire(spire_config.clone()),
            ),
        }
    }
}

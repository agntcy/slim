// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};

use crate::auth::jwt::Config as JwtConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig;
use crate::auth::static_jwt::Config as StaticJwtConfig;
use crate::auth::ConfigAuthError;

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IdentityProviderConfig {
    SharedSecret {
        id: String,
        data: String,
    },
    StaticJwt(StaticJwtConfig),
    Jwt(JwtConfig),
    #[cfg(not(target_family = "windows"))]
    Spire(SpireConfig),
    #[default]
    None,
}

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IdentityVerifierConfig {
    SharedSecret {
        id: String,
        data: String,
    },
    Jwt(JwtConfig),
    #[cfg(not(target_family = "windows"))]
    Spire(SpireConfig),
    #[default]
    None,
}

impl IdentityProviderConfig {
    /// Build an [`AuthProvider`] from this configuration.
    pub fn build_auth_provider(&self) -> Result<AuthProvider, ConfigAuthError> {
        match self {
            Self::SharedSecret { id, data } => {
                Ok(AuthProvider::shared_secret_from_str(id, data)?)
            }
            Self::StaticJwt(jwt_config) => {
                let provider = jwt_config.build_static_token_provider()?;
                Ok(AuthProvider::static_token(provider))
            }
            Self::Jwt(jwt_config) => {
                let provider = jwt_config.get_provider()?;
                Ok(AuthProvider::jwt_signer(provider))
            }
            #[cfg(not(target_family = "windows"))]
            Self::Spire(spire_config) => {
                let manager = spire_config.create_provider()?;
                Ok(AuthProvider::spire(manager))
            }
            Self::None => Err(ConfigAuthError::IdentityProviderNotConfigured),
        }
    }
}

impl IdentityVerifierConfig {
    /// Build an [`AuthVerifier`] from this configuration.
    pub fn build_auth_verifier(&self) -> Result<AuthVerifier, ConfigAuthError> {
        match self {
            Self::SharedSecret { id, data } => {
                Ok(AuthVerifier::shared_secret_from_str(id, data)?)
            }
            Self::Jwt(jwt_config) => {
                let verifier = jwt_config.get_verifier()?;
                Ok(AuthVerifier::jwt_verifier(verifier))
            }
            #[cfg(not(target_family = "windows"))]
            Self::Spire(spire_config) => {
                let manager = spire_config.create_verifier()?;
                Ok(AuthVerifier::spire(manager))
            }
            Self::None => Err(ConfigAuthError::IdentityVerifierNotConfigured),
        }
    }
}

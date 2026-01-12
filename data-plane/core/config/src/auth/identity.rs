// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

use crate::auth::jwt::Config as JwtConfig;
#[cfg(not(target_family = "windows"))]
use crate::auth::spire::SpireConfig;
use crate::auth::static_jwt::Config as StaticJwtConfig;

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

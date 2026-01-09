// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

use crate::auth::jwt::Config as JwtConfig;
use crate::auth::static_jwt::Config as StaticJwtConfig;

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IdentityProviderConfig {
    SharedSecret {
        data: String,
    },
    StaticJwt(StaticJwtConfig),
    Jwt(JwtConfig),
    #[default]
    None,
}

#[derive(Default, Debug, Clone, Deserialize, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IdentityVerifierConfig {
    SharedSecret {
        data: String,
    },
    Jwt(JwtConfig),
    #[default]
    None,
}

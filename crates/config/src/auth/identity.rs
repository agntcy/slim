// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};

use crate::auth::ConfigAuthError;
use crate::auth::jwt::Config as JwtConfig;
use crate::auth::oidc::Config as OidcConfig;
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
    /// OIDC identity bound to the MLS signing key via DPoP (RFC 9449). The MLS
    /// identity becomes the IdP's `sub`, shared by every instance that user runs.
    /// Requires Keycloak >= 24 with DPoP enabled, or equivalent.
    Oidc(OidcConfig),
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
    /// Verifies tokens via JWKS, resolving the presented MLS key against `cnf.jkt`.
    Oidc(OidcConfig),
    #[default]
    None,
}

impl IdentityProviderConfig {
    /// Build an [`AuthProvider`] from this configuration.
    pub fn build_auth_provider(&self) -> Result<AuthProvider, ConfigAuthError> {
        match self {
            Self::SharedSecret { id, data } => Ok(AuthProvider::shared_secret_from_str(id, data)?),
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
            Self::Oidc(oidc_config) => {
                let provider = oidc_config.create_identity_provider()?;
                Ok(AuthProvider::oidc(provider))
            }
            Self::None => Err(ConfigAuthError::IdentityProviderNotConfigured),
        }
    }
}

impl IdentityVerifierConfig {
    /// Build an [`AuthVerifier`] from this configuration.
    pub fn build_auth_verifier(&self) -> Result<AuthVerifier, ConfigAuthError> {
        match self {
            Self::SharedSecret { id, data } => Ok(AuthVerifier::shared_secret_from_str(id, data)?),
            Self::Jwt(jwt_config) => {
                let verifier = jwt_config.get_verifier()?;
                Ok(AuthVerifier::jwt_verifier(verifier))
            }
            #[cfg(not(target_family = "windows"))]
            Self::Spire(spire_config) => {
                let manager = spire_config.create_verifier()?;
                Ok(AuthVerifier::spire(manager))
            }
            Self::Oidc(oidc_config) => {
                let verifier = oidc_config.create_verifier()?;
                Ok(AuthVerifier::oidc(verifier))
            }
            Self::None => Err(ConfigAuthError::IdentityVerifierNotConfigured),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selectable from a config file, tagged like every other variant.
    #[test]
    fn oidc_provider_config_deserializes_from_tagged_yaml() {
        let cfg: IdentityProviderConfig = serde_yaml::from_str(
            r#"
type: oidc
issuer_url: https://keycloak.example.com/realms/slim
client_id: slim-app
audience: slim
"#,
        )
        .unwrap();

        match cfg {
            IdentityProviderConfig::Oidc(oidc) => {
                assert_eq!(oidc.issuer_url, "https://keycloak.example.com/realms/slim");
                assert_eq!(oidc.client_id.as_deref(), Some("slim-app"));
                // A public client has no secret; this must round-trip as absent
                // rather than failing to parse.
                assert!(oidc.client_secret.is_none());
            }
            other => panic!("expected Oidc, got {other:?}"),
        }
    }

    #[test]
    fn oidc_verifier_config_deserializes_from_tagged_yaml() {
        let cfg: IdentityVerifierConfig = serde_yaml::from_str(
            r#"
type: oidc
issuer_url: https://keycloak.example.com/realms/slim
audience: slim
"#,
        )
        .unwrap();
        assert!(matches!(cfg, IdentityVerifierConfig::Oidc(_)));
    }

    /// Without an audience the build must fail, not accept every token.
    #[test]
    fn oidc_verifier_without_audience_fails_to_build() {
        let cfg = IdentityVerifierConfig::Oidc(OidcConfig::new(
            "https://keycloak.example.com/realms/slim",
        ));
        assert!(cfg.build_auth_verifier().is_err());
    }

    /// Nothing to authenticate with: no secret, and no login for this issuer.
    /// The issuer is deliberately unresolvable so a real
    /// `~/.slimctl/credentials.yaml` cannot be adopted and change the result.
    #[test]
    fn oidc_public_client_without_a_login_is_rejected() {
        let cfg = IdentityProviderConfig::Oidc(
            OidcConfig::new("https://no-such-issuer.invalid/realms/slim")
                .with_client_credentials("slim-app", ""),
        );
        assert!(matches!(
            cfg.build_auth_provider(),
            Err(ConfigAuthError::IdentityProviderNotConfigured)
        ));
    }

    /// A `client_secret` identity must be refused, not accepted. The
    /// client-credentials grant is not DPoP-bound, so its token carries no MLS
    /// key: such a provider builds and fetches tokens happily, then every peer
    /// rejects it with `PublicKeyNotFound` at the first group join. Failing here
    /// turns that into a startup error naming the alternatives.
    #[test]
    fn oidc_identity_rejects_a_client_secret() {
        let cfg = IdentityProviderConfig::Oidc(
            OidcConfig::new("https://no-such-issuer.invalid/realms/slim")
                .with_client_credentials("slim-app", "the-secret"),
        );
        assert!(matches!(
            cfg.build_auth_provider(),
            Err(ConfigAuthError::IdentityProviderNotConfigured)
        ));
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Application-level authentication configuration.
//!
//! [`AuthConfig`] provides a high-level enum that maps to the lower-level
//! [`IdentityProviderConfig`] and [`IdentityVerifierConfig`] types.

use serde::Deserialize;

use super::ConfigAuthError;
use super::identity::{IdentityProviderConfig, IdentityVerifierConfig};
use super::oidc::Config as OidcConfig;
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
    /// OIDC identity bound to the MLS signing key via DPoP, so it is shared
    /// across every instance that user runs.
    Oidc(OidcConfig),
}

impl AuthConfig {
    /// Return a copy with the identity `id` overridden.
    /// For SPIRE configs this is a no-op (identity comes from the workload).
    pub fn with_identity_id(self, id: String) -> Self {
        match self {
            AuthConfig::SharedSecret { secret, .. } => AuthConfig::SharedSecret {
                id: Some(id),
                secret,
            },
            #[cfg(not(target_family = "windows"))]
            AuthConfig::Spire(cfg) => AuthConfig::Spire(cfg),
            // Identity comes from the IdP's `sub`; nothing local to override.
            AuthConfig::Oidc(cfg) => AuthConfig::Oidc(cfg),
        }
    }

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
            AuthConfig::Oidc(oidc_config) => {
                if oidc_config.issuer_url.is_empty() {
                    return Err(ConfigAuthError::AuthOidcEmptyIssuerUrl);
                }
                if oidc_config.client_id.is_none() {
                    return Err(ConfigAuthError::AuthOidcEmptyClientId);
                }
                // One config drives both halves, and the verifier needs an audience
                // to validate `aud` against.
                if !oidc_config.can_verify() {
                    return Err(ConfigAuthError::AuthJwtAudienceRequired);
                }
                // A client-credentials token is never DPoP-bound (see
                // `OidcConfig::create_identity_provider`'s doc comment on why),
                // so it carries no MLS key and every peer would reject it at
                // the first group join. Catch the mistake here, at config-load
                // time, instead of at provider-construction time.
                if oidc_config
                    .client_secret
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
                {
                    return Err(ConfigAuthError::AuthOidcIdentityClientSecretNotAllowed);
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
            // `local_name` is unused: the identity is the IdP's `sub`.
            AuthConfig::Oidc(oidc_config) => (
                IdentityProviderConfig::Oidc(oidc_config.clone()),
                IdentityVerifierConfig::Oidc(oidc_config.clone()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_secret() {
        let cfg = AuthConfig::SharedSecret {
            id: None,
            secret: String::new(),
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigAuthError::AuthSecretEmpty)
        ));
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn validate_rejects_missing_spire_socket_path() {
        let cfg = AuthConfig::Spire(SpireConfig {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: vec![],
            trust_domains: vec![],
        });
        assert!(matches!(
            cfg.validate(),
            Err(ConfigAuthError::AuthSpireSocketPathMissing)
        ));
    }

    #[test]
    fn to_identity_configs_uses_local_name_when_id_is_none() {
        let cfg = AuthConfig::SharedSecret {
            id: None,
            secret: "my-secret-that-is-long-enough-ok".to_string(),
        };
        let (provider, verifier) = cfg.to_identity_configs("fallback-name");
        match provider {
            IdentityProviderConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "fallback-name");
            }
            _ => panic!("expected SharedSecret provider"),
        }
        match verifier {
            IdentityVerifierConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "fallback-name");
            }
            _ => panic!("expected SharedSecret verifier"),
        }
    }

    #[test]
    fn to_identity_configs_uses_explicit_id() {
        let cfg = AuthConfig::SharedSecret {
            id: Some("explicit-id".to_string()),
            secret: "my-secret-that-is-long-enough-ok".to_string(),
        };
        let (provider, _) = cfg.to_identity_configs("fallback-name");
        match provider {
            IdentityProviderConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "explicit-id");
            }
            _ => panic!("expected SharedSecret provider"),
        }
    }

    fn oidc_cfg() -> OidcConfig {
        OidcConfig::combined(
            "https://keycloak.example.com/realms/slim",
            "slim-app",
            "",
            "slim",
        )
    }

    #[test]
    fn oidc_validate_accepts_public_client_without_secret() {
        // Authorization code + PKCE clients are public by design; requiring a
        // secret here would reject the very flow DPoP identity depends on.
        assert!(AuthConfig::Oidc(oidc_cfg()).validate().is_ok());
    }

    #[test]
    fn oidc_validate_rejects_missing_issuer_client_id_and_audience() {
        let mut cfg = oidc_cfg();
        cfg.issuer_url = String::new();
        assert!(matches!(
            AuthConfig::Oidc(cfg).validate(),
            Err(ConfigAuthError::AuthOidcEmptyIssuerUrl)
        ));

        let mut cfg = oidc_cfg();
        cfg.client_id = None;
        assert!(matches!(
            AuthConfig::Oidc(cfg).validate(),
            Err(ConfigAuthError::AuthOidcEmptyClientId)
        ));

        // Without an audience the verifier half cannot validate `aud`.
        let mut cfg = oidc_cfg();
        cfg.audience = None;
        assert!(matches!(
            AuthConfig::Oidc(cfg).validate(),
            Err(ConfigAuthError::AuthJwtAudienceRequired)
        ));
    }

    /// A `client_secret` on an identity config must be caught at config-load
    /// time (`validate`), not left to surface later at provider-construction
    /// time (`OidcConfig::create_identity_provider`) — see its doc comment
    /// on why a client-credentials token can never be an identity's.
    #[test]
    fn oidc_validate_rejects_client_secret() {
        let mut cfg = oidc_cfg();
        cfg.client_secret = Some("a-client-secret".to_string());
        assert!(matches!(
            AuthConfig::Oidc(cfg).validate(),
            Err(ConfigAuthError::AuthOidcIdentityClientSecretNotAllowed)
        ));
    }

    #[test]
    fn oidc_to_identity_configs_ignores_local_name() {
        // The identity is the IdP's `sub`, so nothing local should leak in.
        let (provider, verifier) = AuthConfig::Oidc(oidc_cfg()).to_identity_configs("some-name");
        assert_eq!(provider, IdentityProviderConfig::Oidc(oidc_cfg()));
        assert_eq!(verifier, IdentityVerifierConfig::Oidc(oidc_cfg()));
    }

    #[test]
    fn oidc_with_identity_id_is_a_noop() {
        let cfg = AuthConfig::Oidc(oidc_cfg()).with_identity_id("ignored".to_string());
        match cfg {
            AuthConfig::Oidc(inner) => assert_eq!(inner, oidc_cfg()),
            _ => panic!("expected Oidc"),
        }
    }

    #[test]
    fn with_identity_id_overrides_existing_id() {
        let cfg = AuthConfig::SharedSecret {
            id: Some("old-id".to_string()),
            secret: "my-secret-that-is-long-enough-ok".to_string(),
        };
        let cfg = cfg.with_identity_id("new-id".to_string());
        match cfg {
            AuthConfig::SharedSecret { id, secret } => {
                assert_eq!(id, Some("new-id".to_string()));
                assert_eq!(secret, "my-secret-that-is-long-enough-ok");
            }
            #[cfg(not(target_family = "windows"))]
            _ => panic!("expected SharedSecret"),
        }
    }
}

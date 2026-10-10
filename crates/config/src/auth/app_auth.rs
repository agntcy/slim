// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Application-level authentication configuration.
//!
//! [`AuthConfig`] provides a high-level enum that maps to the lower-level
//! [`IdentityProviderConfig`] and [`IdentityVerifierConfig`] types.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use slim_auth::did_key::{ed25519_did_key, ed25519_public_key_from_pkcs8_pem};
use slim_auth::errors::AuthError as SlimAuthError;
use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat};

use super::ConfigAuthError;
use super::identity::{IdentityProviderConfig, IdentityVerifierConfig};
use super::jwt::{Claims, Config as JwtConfig, JwtKey};
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
    /// Self-issued JWT identity: the app signs EdDSA tokens with an Ed25519
    /// key, as its `did:key` (`iss` = `sub`), and accepts peers whose tokens
    /// verify against a JWKS of trusted public keys.
    Jwt {
        /// PKCS#8 PEM Ed25519 private key (`file:` path or inline `data:`).
        private_key: KeyData,
        /// JWKS of the public keys of trusted peers (`file:` path or inline
        /// `data:`).
        trusted_keys: KeyData,
    },
}

/// Lifetime of the tokens a `jwt` identity issues.
const JWT_TOKEN_LIFETIME: Duration = Duration::from_secs(3600);

/// The contents of `key`, or `None` if its file can't be read.
fn read_key_data(key: &KeyData) -> Option<String> {
    match key {
        KeyData::Data(data) => Some(data.clone()),
        KeyData::File(path) => std::fs::read_to_string(path).ok(),
    }
}

/// Whether a JWKS document has an entry for `public_key` (an OKP key whose
/// `x` is its base64url encoding). `true` when it doesn't parse as a JWKS,
/// which the verifier reports instead.
fn jwks_has_key(jwks: &str, public_key: &[u8]) -> bool {
    let Ok(jwks) = serde_json::from_str::<serde_json::Value>(jwks) else {
        return true;
    };
    let Some(keys) = jwks.get("keys").and_then(|k| k.as_array()) else {
        return true;
    };
    keys.iter().any(|key| {
        key.get("x")
            .and_then(|x| x.as_str())
            .and_then(|x| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(x.trim_end_matches('='))
                    .ok()
            })
            .is_some_and(|x| x == public_key)
    })
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
            // The identity is the key's did:key.
            jwt @ AuthConfig::Jwt { .. } => jwt,
        }
    }

    /// The app's `did:key`, for a `jwt` identity; `None` for the others.
    pub fn did_key(&self) -> Result<Option<String>, ConfigAuthError> {
        Ok(self.jwt_public_key()?.map(|key| ed25519_did_key(&key)))
    }

    /// The public key of a `jwt` identity's private key.
    fn jwt_public_key(&self) -> Result<Option<Vec<u8>>, ConfigAuthError> {
        let AuthConfig::Jwt { private_key, .. } = self else {
            return Ok(None);
        };
        let pem = match private_key {
            KeyData::Data(pem) => pem.clone(),
            KeyData::File(path) => std::fs::read_to_string(path).map_err(|source| {
                ConfigAuthError::AuthJwtPrivateKeyRead {
                    path: path.clone(),
                    source,
                }
            })?,
        };
        Ok(Some(ed25519_public_key_from_pkcs8_pem(&pem)?))
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
            AuthConfig::Jwt { trusted_keys, .. } => {
                let public_key = self.jwt_public_key()?.unwrap_or_default();
                // MLS checks the app's own credential -- its own token -- with
                // this verifier, so the app must trust its own key. A JWKS that
                // can't be read or parsed is left for the verifier to report.
                if let Some(jwks) = read_key_data(trusted_keys)
                    && !jwks_has_key(&jwks, &public_key)
                {
                    let did_key = ed25519_did_key(&public_key);
                    return Err(SlimAuthError::InvalidEd25519Key(format!(
                        "auth.trusted_keys has no entry for this app's own key ({did_key}): \
                         MLS verifies the app's own token against it, so add it"
                    ))
                    .into());
                }
            }
        }
        Ok(())
    }

    /// Convert to IdentityProviderConfig + IdentityVerifierConfig pair.
    /// For shared_secret, uses the explicit `id` if provided, otherwise
    /// falls back to `local_name`. Fails only for a `jwt` identity whose
    /// private key can't be read or isn't an Ed25519 key.
    pub fn to_identity_configs(
        &self,
        local_name: &str,
    ) -> Result<(IdentityProviderConfig, IdentityVerifierConfig), ConfigAuthError> {
        Ok(match self {
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
            AuthConfig::Jwt {
                private_key,
                trusted_keys,
            } => {
                let did_key = self.did_key()?;
                let signing_key = JwtKey::Encoding(Key {
                    algorithm: Algorithm::EdDSA,
                    format: KeyFormat::Pem,
                    key: private_key.clone(),
                });
                let verifying_keys = JwtKey::Decoding(Key {
                    algorithm: Algorithm::EdDSA,
                    format: KeyFormat::Jwks,
                    key: trusted_keys.clone(),
                });
                (
                    IdentityProviderConfig::Jwt(JwtConfig::new(
                        Claims::new(None, did_key.clone(), did_key, None),
                        JWT_TOKEN_LIFETIME,
                        signing_key,
                    )),
                    IdentityVerifierConfig::Jwt(JwtConfig::new(
                        Claims::default(),
                        JWT_TOKEN_LIFETIME,
                        verifying_keys,
                    )),
                )
            }
        })
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
        let (provider, verifier) = cfg.to_identity_configs("fallback-name").unwrap();
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
        let (provider, _) = cfg.to_identity_configs("fallback-name").unwrap();
        match provider {
            IdentityProviderConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "explicit-id");
            }
            _ => panic!("expected SharedSecret provider"),
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

    // RFC 8032 section 7.1, test 1.
    const ED25519_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const ED25519_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A `jwt` identity for the RFC 8032 key, trusting that same key.
    fn jwt_config() -> AuthConfig {
        use base64::Engine;
        let mut der = hex("302e020100300506032b657004220420");
        der.extend_from_slice(&hex(ED25519_SEED));
        let pem = slim_auth::utils::bytes_to_pem(
            &der,
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----\n",
        );
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hex(ED25519_PUBLIC));
        let jwks = serde_json::json!({"keys": [{
            "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig", "x": x,
        }]});
        AuthConfig::Jwt {
            private_key: KeyData::Data(pem),
            trusted_keys: KeyData::Data(jwks.to_string()),
        }
    }

    fn expected_did_key() -> String {
        slim_auth::did_key::ed25519_did_key(&hex(ED25519_PUBLIC))
    }

    #[test]
    fn jwt_identity_signs_as_its_did_key_and_verifies_against_the_jwks() {
        let (provider, verifier) = jwt_config()
            .to_identity_configs("ignored/local/name")
            .unwrap();
        let IdentityProviderConfig::Jwt(provider) = provider else {
            panic!("expected a JWT provider");
        };
        assert_eq!(provider.claims().issuer(), &Some(expected_did_key()));
        assert_eq!(provider.claims().subject(), &Some(expected_did_key()));
        let IdentityVerifierConfig::Jwt(verifier) = verifier else {
            panic!("expected a JWT verifier");
        };
        assert!(matches!(
            verifier.key(),
            JwtKey::Decoding(Key {
                algorithm: Algorithm::EdDSA,
                format: KeyFormat::Jwks,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn jwt_identity_tokens_verify_against_a_jwks_holding_its_key() {
        use slim_auth::traits::{TokenProvider, Verifier};
        crate::tls::provider::initialize_crypto_provider();
        let (provider, verifier) = jwt_config()
            .to_identity_configs("ignored/local/name")
            .unwrap();
        let mut provider = provider.build_auth_provider().unwrap();
        let mut verifier = verifier.build_auth_verifier().unwrap();
        provider.initialize().await.unwrap();
        verifier.initialize().await.unwrap();

        let token = provider.get_token().unwrap();
        let claims: serde_json::Value = verifier.get_claims(token).await.unwrap();
        assert_eq!(claims["sub"], expected_did_key());
        assert_eq!(claims["iss"], expected_did_key());
    }

    #[test]
    fn did_key_is_none_for_other_identities() {
        let cfg = AuthConfig::SharedSecret {
            id: None,
            secret: "my-secret-that-is-long-enough-ok".to_string(),
        };
        assert_eq!(cfg.did_key().unwrap(), None);
        assert_eq!(jwt_config().did_key().unwrap(), Some(expected_did_key()));
    }

    #[test]
    fn jwt_identity_with_an_unreadable_key_file_is_invalid() {
        let cfg = AuthConfig::Jwt {
            private_key: KeyData::File("/nonexistent/cm-ed25519.pem".to_string()),
            trusted_keys: KeyData::Data("{\"keys\": []}".to_string()),
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigAuthError::AuthJwtPrivateKeyRead { .. })
        ));
        assert!(cfg.to_identity_configs("org/ns/app").is_err());
    }

    /// The RFC 8032 identity with `trusted_keys` replaced.
    fn jwt_config_trusting(trusted_keys: &str) -> AuthConfig {
        let AuthConfig::Jwt { private_key, .. } = jwt_config() else {
            unreachable!()
        };
        AuthConfig::Jwt {
            private_key,
            trusted_keys: KeyData::Data(trusted_keys.to_string()),
        }
    }

    #[test]
    fn jwt_identity_that_trusts_its_own_key_is_valid() {
        assert!(jwt_config().validate().is_ok());
    }

    #[test]
    fn jwt_identity_must_trust_its_own_key() {
        // A valid JWKS, but only for some other key.
        let other = serde_json::json!({"keys": [{
            "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA",
            "x": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE",
        }]});
        let err = jwt_config_trusting(&other.to_string())
            .validate()
            .unwrap_err();
        let ConfigAuthError::AuthInternalError(SlimAuthError::InvalidEd25519Key(msg)) = err else {
            panic!("unexpected error: {err:?}");
        };
        assert!(msg.contains(&expected_did_key()), "{msg}");
    }

    #[test]
    fn an_unparseable_jwks_is_left_for_the_verifier() {
        assert!(jwt_config_trusting("not a jwks").validate().is_ok());
    }

    #[test]
    fn with_identity_id_keeps_a_jwt_identity() {
        let cfg = jwt_config().with_identity_id("new-id".to_string());
        assert_eq!(cfg.did_key().unwrap(), Some(expected_did_key()));
    }
}

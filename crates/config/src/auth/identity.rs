// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};

use crate::auth::ConfigAuthError;
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
    /// Signs with an Ed25519 key whose `did:key` is the token's `iss`/`sub`,
    /// and uses that key as the MLS signature key, so the `pubkey` claim
    /// matches what an `InsecureSelfCertifiedDid` verifier requires. Requires the
    /// `insecure-self-certified-did` Cargo feature.
    #[cfg(feature = "insecure-self-certified-did")]
    InsecureSelfCertifiedDid(JwtConfig),
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
    /// Accepts any JWT whose `iss`/`sub` is a `did:key:z...` Ed25519
    /// identifier, with no JWKS, allow-list, or live authority. Contract: key
    /// consistency only; admission is the application's job. The wrapped
    /// `JwtConfig`'s `key` field is ignored; the decoding key always comes
    /// from the token itself. Requires the `insecure-self-certified-did`
    /// Cargo feature to construct or deserialize.
    ///
    /// The application must:
    /// - Require the holder DID in the caller's credential presentation (e.g.
    ///   OID4VP) to equal the peer's `PEER_SUBJECT_KEY`, so a presentation
    ///   cannot be replayed over another caller's session.
    /// - Bind the presentation to a callee-supplied nonce and an `aud` naming
    ///   the callee.
    /// - Run no RPC or message handler before admission succeeds.
    /// - Apply rate and resource limits to callers not yet admitted, since
    ///   any key holder can open a session.
    #[cfg(feature = "insecure-self-certified-did")]
    InsecureSelfCertifiedDid(JwtConfig),
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
            #[cfg(feature = "insecure-self-certified-did")]
            Self::InsecureSelfCertifiedDid(jwt_config) => {
                let provider = jwt_config.get_self_certified_did_provider()?;
                Ok(AuthProvider::jwt_signer(provider))
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
            #[cfg(feature = "insecure-self-certified-did")]
            Self::InsecureSelfCertifiedDid(jwt_config) => {
                let verifier = jwt_config.get_self_certified_did_verifier()?;
                Ok(AuthVerifier::jwt_verifier(verifier))
            }
            Self::None => Err(ConfigAuthError::IdentityVerifierNotConfigured),
        }
    }
}

#[cfg(all(test, feature = "insecure-self-certified-did"))]
mod tests {
    use super::*;
    use crate::auth::jwt::{Claims, JwtKey};
    use crate::tls::provider::initialize_crypto_provider;
    use std::time::Duration;

    fn insecure_self_certified_did_config() -> IdentityVerifierConfig {
        // `key` is mandatory on `JwtConfig::new` but ignored by this variant;
        // `Autoresolve` is just a harmless placeholder.
        IdentityVerifierConfig::InsecureSelfCertifiedDid(JwtConfig::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Autoresolve,
        ))
    }

    #[test]
    fn serializes_with_the_insecure_type_tag() {
        let config = insecure_self_certified_did_config();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(
            json.get("type").and_then(|v| v.as_str()),
            Some("insecure_self_certified_did")
        );
    }

    #[test]
    fn round_trips_through_json() {
        let config = insecure_self_certified_did_config();
        let json = serde_json::to_string(&config).unwrap();
        let decoded: IdentityVerifierConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn builds_a_working_verifier() {
        initialize_crypto_provider();
        let config = insecure_self_certified_did_config();
        assert!(config.build_auth_verifier().is_ok());
    }
}

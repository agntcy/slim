// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::{
    ExtensionList, IdentityProvider,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use tracing::debug;

use slim_auth::errors::AuthError;
use slim_auth::traits::Verifier;

use crate::errors::MlsError;

#[derive(Clone)]
pub struct SlimIdentityProvider<V>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    identity_verifier: V,
}

impl<V> SlimIdentityProvider<V>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(identity_verifier: V) -> Self {
        Self { identity_verifier }
    }

    fn credential_token(signing_id: &SigningIdentity) -> Result<&str, MlsError> {
        let basic_cred = signing_id
            .credential
            .as_basic()
            .ok_or(MlsError::NotBasicCredential)?;
        Ok(std::str::from_utf8(&basic_cred.identifier)?)
    }

    /// Map a verifier error to an `MlsError`. A missing claim stays a typed
    /// `IdentityProviderError`, so callers can tell it from a bad token.
    fn auth_error(e: AuthError, context: &str) -> MlsError {
        match e {
            AuthError::PublicKeyNotFound | AuthError::SubjectNotFound => {
                MlsError::IdentityProviderError(e)
            }
            e => MlsError::verification_failed(format!("{context}: {e}")),
        }
    }

    /// Verify the token and return its subject, without checking key binding.
    fn resolve_subject(&self, signing_id: &SigningIdentity) -> Result<String, MlsError> {
        let token = Self::credential_token(signing_id)?;
        let subject = self
            .identity_verifier
            .try_get_subject(token)
            .map_err(|e| Self::auth_error(e, "could not get subject from token"))?;
        debug!(sub = %subject, "Extracted subject from token");
        Ok(subject)
    }

    /// Verify the token and that it is bound to the identity's signature key.
    ///
    /// The verifier decides how the token binds the key (e.g. a `pubkey` claim,
    /// DPoP confirmation). Without this check a token stolen from another member
    /// would pass for any key.
    pub fn verify_key_binding(&self, signing_id: &SigningIdentity) -> Result<(), MlsError> {
        let token = Self::credential_token(signing_id)?;
        self.identity_verifier
            .try_verify_key_binding(token, signing_id.signature_key.as_ref())
            .map_err(|e| match e {
                AuthError::PublicKeyMismatch {
                    subject,
                    expected,
                    found,
                } => {
                    tracing::error!(
                        %expected, %found, %subject, "public key mismatch",
                    );
                    MlsError::PublicKeyMismatch { expected, found }
                }
                e => Self::auth_error(e, "could not verify token"),
            })
    }
}

// The `IdentityProvider` trait from `mls-rs-core` is sync on native and async
// on wasm32. `#[maybe_async::maybe_async]` selects the matching form via the
// `is_sync` feature (enabled per-target in Cargo.toml).
#[maybe_async::maybe_async]
impl<V> IdentityProvider for SlimIdentityProvider<V>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    type Error = MlsError;

    async fn validate_member(
        &self,
        signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _context: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        debug!("Validating MLS group member identity");
        self.verify_key_binding(signing_identity)?;

        Ok(())
    }

    async fn validate_external_sender(
        &self,
        _signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        tracing::error!("validating external senders is not supported in SlimIdentityProvider");
        Err(MlsError::ExternalCommitNotSupported)
    }

    async fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        Ok(self.resolve_subject(signing_identity)?.into_bytes())
    }

    async fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        debug!("Validating identity succession");
        let pred_subject = self.resolve_subject(predecessor)?;
        let succ_subject = self.resolve_subject(successor)?;

        // Successor is valid if both identities have the same subject
        let is_valid = pred_subject == succ_subject;
        debug!(%is_valid, "Identity succession validation result");
        Ok(is_valid)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

#[cfg(test)]
mod tests {
    use mls_rs::identity::{SigningIdentity, basic::BasicCredential};
    use mls_rs_core::crypto::SignaturePublicKey;
    use serde::de::DeserializeOwned;
    use slim_auth::errors::AuthError;

    use super::*;

    /// Verifier for tokens that carry a subject but no `pubkey` claim, binding
    /// keys some other way (the key `b"bound"`).
    #[derive(Clone)]
    struct NoPubkeyClaim;

    impl Verifier for NoPubkeyClaim {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }

        async fn verify(&self, _token: impl AsRef<str> + Send) -> Result<(), AuthError> {
            Ok(())
        }

        fn try_verify(&self, _token: impl AsRef<str>) -> Result<(), AuthError> {
            Ok(())
        }

        async fn get_claims<Claims>(
            &self,
            token: impl AsRef<str> + Send,
        ) -> Result<Claims, AuthError>
        where
            Claims: DeserializeOwned + Send,
        {
            self.try_get_claims(token)
        }

        fn try_get_claims<Claims>(&self, _token: impl AsRef<str>) -> Result<Claims, AuthError>
        where
            Claims: DeserializeOwned + Send,
        {
            Ok(serde_json::from_value(
                serde_json::json!({ "sub": "alice" }),
            )?)
        }

        fn try_get_subject(&self, token: impl AsRef<str>) -> Result<String, AuthError> {
            slim_auth::traits::subject_from_claims(self, token)
        }

        fn try_verify_key_binding(
            &self,
            token: impl AsRef<str>,
            public_key: &[u8],
        ) -> Result<(), AuthError> {
            slim_auth::traits::verify_pubkey_claim_binding(self, token, public_key)
        }
    }

    fn identity(token: &str, key: &[u8]) -> SigningIdentity {
        SigningIdentity::new(
            BasicCredential::new(token.as_bytes().to_vec()).into_credential(),
            SignaturePublicKey::new(key.to_vec()),
        )
    }

    #[test]
    fn identity_and_succession_do_not_need_a_pubkey_claim() {
        let provider = SlimIdentityProvider::new(NoPubkeyClaim);
        let old = identity("token", b"key-1");
        let new = identity("token", b"key-2");

        assert_eq!(
            provider.identity(&old, &ExtensionList::default()).unwrap(),
            b"alice"
        );
        assert!(
            provider
                .valid_successor(&old, &new, &ExtensionList::default())
                .unwrap()
        );
    }

    #[test]
    fn a_missing_pubkey_claim_is_a_typed_error() {
        let provider = SlimIdentityProvider::new(NoPubkeyClaim);
        assert!(matches!(
            provider.verify_key_binding(&identity("token", b"key")),
            Err(MlsError::IdentityProviderError(
                AuthError::PublicKeyNotFound
            ))
        ));
    }
}

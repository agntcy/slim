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

    /// Verify the token and return its subject, without checking key binding.
    fn resolve_subject(&self, signing_id: &SigningIdentity) -> Result<String, MlsError> {
        let token = Self::credential_token(signing_id)?;
        let subject = self.identity_verifier.try_get_subject(token).map_err(|e| {
            MlsError::verification_failed(format!("could not get subject from token: {}", e))
        })?;
        debug!(sub = %subject, "Extracted subject from token");
        Ok(subject)
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
        let token = Self::credential_token(signing_identity)?;

        // The verifier decides how the token binds the signing public key
        // (e.g. `pubkey` claim, DPoP confirmation).
        self.identity_verifier
            .try_verify_key_binding(token, signing_identity.signature_key.as_ref())
            .map_err(|e| match e {
                AuthError::PublicKeyMismatch { expected, found } => {
                    tracing::error!(%expected, %found, "public key mismatch");
                    MlsError::PublicKeyMismatch { expected, found }
                }
                e => MlsError::verification_failed(format!("could not verify token: {}", e)),
            })?;

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

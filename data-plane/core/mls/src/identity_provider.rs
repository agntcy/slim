// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::{
    ExtensionList, IdentityProvider,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use tracing::debug;

use slim_auth::identity_claims::IdentityClaims;
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

    fn resolve_slim_identity(
        &self,
        signing_id: &SigningIdentity,
    ) -> Result<IdentityClaims, MlsError> {
        let basic_cred = signing_id
            .credential
            .as_basic()
            .ok_or(MlsError::NotBasicCredential)?;
        let credential_data = std::str::from_utf8(&basic_cred.identifier)?;

        // Verify token and extract claims (sync only — mls_build_async is not set)
        let claims: serde_json::Value = self
            .identity_verifier
            .try_get_claims(credential_data)
            .map_err(|e| {
                MlsError::verification_failed(format!("could not get claims from token: {}", e))
            })?;

        // Extract identity claims using the abstraction
        let identity_claims = IdentityClaims::from_json(&claims)?;

        debug!(
            claims = %claims,
            "Extracted public key from claims",
        );
        debug!(sub = %identity_claims.subject, "Extracted subject from claims");

        Ok(identity_claims)
    }

    fn verify_public_key_match(expected: &str, found: &str, subject: &str) -> Result<(), MlsError> {
        if found != expected {
            tracing::error!(
                expected = %expected, found = %found, subject = %subject, "public key mismatch",
            );
            return Err(MlsError::PublicKeyMismatch {
                expected: expected.to_string(),
                found: found.to_string(),
            });
        }
        Ok(())
    }
}

#[cfg_attr(not(mls_build_async), maybe_async::must_be_sync)]
#[cfg_attr(mls_build_async, maybe_async::must_be_async)]
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
        let identity_claims = self.resolve_slim_identity(signing_identity)?;

        // make sure the public key matches the signing identity's public key
        let signing_pubkey =
            IdentityClaims::encode_public_key(signing_identity.signature_key.as_ref());

        Self::verify_public_key_match(
            &signing_pubkey,
            &identity_claims.public_key,
            &identity_claims.subject,
        )?;

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
        let identity_claims = self.resolve_slim_identity(signing_identity)?;

        Ok(identity_claims.subject.into_bytes())
    }

    async fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        debug!("Validating identity succession");
        let pred_claims = self.resolve_slim_identity(predecessor)?;
        let succ_claims = self.resolve_slim_identity(successor)?;

        // Successor is valid if both identities have the same subject
        let is_valid = pred_claims.subject == succ_claims.subject;
        debug!(%is_valid, "Identity succession validation result");
        Ok(is_valid)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

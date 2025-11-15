// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use mls_rs::{
    ExtensionList, IdentityProvider,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use tracing::debug;

use slim_auth::{errors::AuthError, traits::Verifier};

use crate::errors::SlimIdentityError;
use crate::identity_claims::IdentityClaims;

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

    async fn resolve_slim_identity(
        &self,
        signing_id: &SigningIdentity,
    ) -> Result<IdentityClaims, SlimIdentityError> {
        let basic_cred = signing_id
            .credential
            .as_basic()
            .ok_or(SlimIdentityError::NotBasicCredential)?;

        let credential_data =
            std::str::from_utf8(&basic_cred.identifier).map_err(SlimIdentityError::InvalidUtf8)?;

        // Verify token and extract claims
        let claims: serde_json::Value = match self.identity_verifier.try_get_claims(credential_data)
        {
            Ok(claims) => claims,
            Err(AuthError::WouldBlockOn) => {
                // Fallback to async verification
                self.identity_verifier
                    .get_claims(credential_data)
                    .await
                    .map_err(|e| {
                        SlimIdentityError::VerificationFailed(format!(
                            "could not get claims from token: {}",
                            e
                        ))
                    })?
            }
            Err(e) => {
                return Err(SlimIdentityError::VerificationFailed(format!(
                    "could not get claims from token: {}",
                    e
                )));
            }
        };

        // Extract identity claims using the abstraction
        let identity_claims = IdentityClaims::from_json(&claims)?;

        debug!(
            "Extracted public key from claims: {}",
            identity_claims.public_key
        );
        debug!("Extracted subject from claims: {}", identity_claims.subject);

        Ok(identity_claims)
    }

    fn verify_public_key_match(
        expected: &str,
        found: &str,
        subject: &str,
    ) -> Result<(), SlimIdentityError> {
        if found != expected {
            tracing::error!(
                expected = %expected, found = %found, subject = %subject, "Public key mismatch",
            );
            return Err(SlimIdentityError::PublicKeyMismatch {
                expected: expected.to_string(),
                found: found.to_string(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl<V> IdentityProvider for SlimIdentityProvider<V>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    type Error = SlimIdentityError;

    async fn validate_member(
        &self,
        signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _context: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        debug!("Validating MLS group member identity");
        let identity_claims = self.resolve_slim_identity(signing_identity).await?;

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
        tracing::error!("Validating external senders is not supported in SlimIdentityProvider");
        Err(SlimIdentityError::ExternalCommitNotSupported)
    }

    async fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        let identity_claims = self.resolve_slim_identity(signing_identity).await?;

        Ok(identity_claims.subject.into_bytes())
    }

    async fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        debug!("Validating identity succession");
        let pred_claims = self.resolve_slim_identity(predecessor).await?;
        let succ_claims = self.resolve_slim_identity(successor).await?;

        // Successor is valid if both identities have the same subject
        let is_valid = pred_claims.subject == succ_claims.subject;
        debug!("Identity succession validation result: {}", is_valid);
        Ok(is_valid)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

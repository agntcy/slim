// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::SlimIdentityError;
use mls_rs::{
    ExtensionList, IdentityProvider,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use slim_auth::traits::Verifier;
use tracing::debug;

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
        self: &Self,
        signing_id: &SigningIdentity,
    ) -> Result<String, SlimIdentityError> {
        let basic_cred = signing_id
            .credential
            .as_basic()
            .ok_or(SlimIdentityError::NotBasicCredential)?;

        let credential_data =
            std::str::from_utf8(&basic_cred.identifier).map_err(SlimIdentityError::InvalidUtf8)?;

        // get id
        self.identity_verifier
            .try_get_id(credential_data)
            .map_err(|e| {
                SlimIdentityError::VerificationFailed(format!(
                    "Failed to extract identity from credential data: {}",
                    e
                ))
            })
    }
}

impl<V> IdentityProvider for SlimIdentityProvider<V>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    type Error = SlimIdentityError;

    fn validate_member(
        &self,
        signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _context: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        debug!("Validating MLS group member identity");
        self.resolve_slim_identity(signing_identity)
            .map(|_id| {})
            .map_err(|e| {
                SlimIdentityError::VerificationFailed(format!(
                    "Failed to validate member identity: {}",
                    e
                ))
            })
    }

    fn validate_external_sender(
        &self,
        signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        debug!("Validating external sender identity");
        self.resolve_slim_identity(signing_identity)
            .map(|_id| {})
            .map_err(|e| {
                SlimIdentityError::VerificationFailed(format!(
                    "Failed to validate member identity: {}",
                    e
                ))
            })
    }

    fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        debug!("Resolving identity from signing identity");
        let id = self.resolve_slim_identity(signing_identity)?;

        debug!("Resolved identity: {}", id);
        Ok(id.into_bytes())
    }

    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        debug!("Validating identity succession");
        let pred_identity = self.resolve_slim_identity(predecessor)?;
        let succ_identity = self.resolve_slim_identity(successor)?;

        let is_valid = pred_identity == succ_identity;

        debug!("Identity succession validation result: {}", is_valid);
        Ok(is_valid)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

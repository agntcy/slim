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

//TODO(zkacsand): what should we use for `.try_verify::<EmptyClaims>(&identity)`?
#[derive(serde::Deserialize)]
struct EmptyClaims {
    #[allow(dead_code)]
    exp: u64,
}

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
}

fn resolve_slim_identity(signing_id: &SigningIdentity) -> Result<String, SlimIdentityError> {
    let basic_cred = signing_id
        .credential
        .as_basic()
        .ok_or(SlimIdentityError::NotBasicCredential)?;

    let credential_data =
        std::str::from_utf8(&basic_cred.identifier).map_err(SlimIdentityError::InvalidUtf8)?;

    Ok(credential_data.to_string())
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
        let identity = resolve_slim_identity(signing_identity)?;

        self.identity_verifier
            .try_verify::<EmptyClaims>(&identity)
            .map_err(|e| SlimIdentityError::VerificationFailed(e.to_string()))?;

        Ok(())
    }

    fn validate_external_sender(
        &self,
        signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        debug!("Validating external sender identity");
        let identity = resolve_slim_identity(signing_identity)?;

        self.identity_verifier
            .try_verify::<EmptyClaims>(&identity)
            .map_err(|e| SlimIdentityError::ExternalSenderFailed(e.to_string()))?;

        Ok(())
    }

    fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        let identity = resolve_slim_identity(signing_identity)?;
        Ok(identity.into_bytes())
    }

    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        debug!("Validating identity succession");
        let pred_identity = resolve_slim_identity(predecessor)?;
        let succ_identity = resolve_slim_identity(successor)?;

        //TODO(zkacsand): we need to verify this with the verifier
        let is_valid = pred_identity == succ_identity;
        debug!("Identity succession validation result: {}", is_valid);
        Ok(is_valid)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

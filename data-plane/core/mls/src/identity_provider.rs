// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::{
    ExtensionList, IdentityProvider,
    error::IntoAnyError,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use slim_auth::traits::Verifier;

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

#[derive(Debug, thiserror::Error)]
#[error("Slim identity validation failed: {0}")]
pub struct SlimIdentityError(String);

impl IntoAnyError for SlimIdentityError {}

fn resolve_slim_identity<V>(
    signing_id: &SigningIdentity,
    _verifier: &V,
) -> Result<String, SlimIdentityError>
where
    V: Verifier + Send + Sync + Clone + 'static,
{
    let basic_cred = signing_id
        .credential
        .as_basic()
        .ok_or_else(|| SlimIdentityError("Not a basic credential".to_string()))?;

    let credential_data = std::str::from_utf8(&basic_cred.identifier)
        .map_err(|_| SlimIdentityError("Invalid UTF-8 in credential".to_string()))?;

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
        resolve_slim_identity(signing_identity, &self.identity_verifier).map(|_| ())
    }

    fn validate_external_sender(
        &self,
        _signing_identity: &SigningIdentity,
        _timestamp: Option<MlsTime>,
        _extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        // TODO: temporary workaround for Simple auth - this should validate the external sender
        Ok(())
    }

    fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        let identity = resolve_slim_identity(signing_identity, &self.identity_verifier)?;
        Ok(identity.into_bytes())
    }

    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        let pred_identity = resolve_slim_identity(predecessor, &self.identity_verifier)?;
        let succ_identity = resolve_slim_identity(successor, &self.identity_verifier)?;

        // for simple auth, consider successors valid if they have the same identity
        Ok(pred_identity == succ_identity)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

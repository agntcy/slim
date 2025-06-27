// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::{
    ExtensionList, IdentityProvider,
    error::IntoAnyError,
    identity::{CredentialType, SigningIdentity},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use slim_auth::jwt::Jwt;
use slim_auth::traits::{StandardClaims, Verifier};

#[derive(Clone)]
pub struct JwtIdentityProvider {
    jwt_verifier: Jwt<slim_auth::jwt::V>,
}

impl JwtIdentityProvider {
    pub fn new(jwt_verifier: Jwt<slim_auth::jwt::V>) -> Self {
        Self { jwt_verifier }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("JWT validation failed: {0}")]
pub struct JwtValidationError(String);

impl IntoAnyError for JwtValidationError {}

fn resolve_jwt_identity(
    signing_id: &SigningIdentity,
    jwt_verifier: &Jwt<slim_auth::jwt::V>,
    timestamp: Option<MlsTime>,
) -> Result<StandardClaims, JwtValidationError> {
    let basic_cred = signing_id
        .credential
        .as_basic()
        .ok_or_else(|| JwtValidationError("Not a basic credential".to_string()))?;

    let jwt_token = std::str::from_utf8(&basic_cred.identifier)
        .map_err(|_| JwtValidationError("Invalid UTF-8 in credential".to_string()))?;

    let claims: StandardClaims = jwt_verifier
        .try_verify(jwt_token)
        .map_err(|e| JwtValidationError(format!("JWT verification failed: {}", e)))?;

    if let Some(mls_time) = timestamp {
        let current_time = mls_time.seconds_since_epoch();

        if current_time > claims.exp {
            return Err(JwtValidationError("JWT token has expired".to_string()));
        }

        if let Some(nbf) = claims.nbf {
            if current_time < nbf {
                return Err(JwtValidationError("JWT token is not yet valid".to_string()));
            }
        }
    }

    Ok(claims)
}

impl IdentityProvider for JwtIdentityProvider {
    type Error = JwtValidationError;

    fn validate_member(
        &self,
        signing_identity: &SigningIdentity,
        timestamp: Option<MlsTime>,
        _context: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        resolve_jwt_identity(signing_identity, &self.jwt_verifier, timestamp).map(|_| ())
    }

    fn validate_external_sender(
        &self,
        signing_identity: &SigningIdentity,
        timestamp: Option<MlsTime>,
        _extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        resolve_jwt_identity(signing_identity, &self.jwt_verifier, timestamp).map(|_| ())
    }

    fn identity(
        &self,
        signing_identity: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        let claims = resolve_jwt_identity(signing_identity, &self.jwt_verifier, None)?;
        Ok(claims
            .sub
            .unwrap_or_else(|| "unknown".to_string())
            .into_bytes())
    }

    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _extensions: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        let pred_claims = resolve_jwt_identity(predecessor, &self.jwt_verifier, None)?;
        let succ_claims = resolve_jwt_identity(successor, &self.jwt_verifier, None)?;

        Ok(pred_claims.sub == succ_claims.sub && pred_claims.iss == succ_claims.iss)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

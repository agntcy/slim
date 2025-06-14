// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common traits for authentication mechanisms.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;

/// Standard JWT Claims structure that includes the registered claims
/// as specified in RFC 7519.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StandardClaims {
    /// Issuer (who issued the JWT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,

    /// Subject (whom the JWT is about)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,

    /// Audience (who the JWT is intended for)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,

    /// Expiration time (when the JWT expires)
    pub exp: u64,

    /// Issued at (when the JWT was issued)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,

    /// JWT ID (unique identifier for this JWT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,

    /// Not before (when the JWT starts being valid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,

    // Additional custom claims can be added by the user
    #[serde(flatten)]
    pub custom_claims: HashMap<String, serde_json::Value>,
}

pub trait Claimer {
    fn create_standard_claims(
        &self,
        custom_claims: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> StandardClaims;
}

/// Trait for verifying JWT tokens
#[async_trait]
pub trait Verifier: Claimer {
    /// Verifies the JWT token and returns the claims if valid.
    ///
    /// The `Claims` type parameter represents the expected structure of the JWT claims.
    async fn verify<Claims>(&mut self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;
}

pub trait SyncVerifier: Claimer {
    /// Try to verify the JWT token without async context and return the claims if valid.
    /// If an async operation is needed, an error is returned.
    fn try_verify<Claims>(&mut self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;
}

/// Trait for signing JWT claims
pub trait Signer: Claimer {
    /// Signs the claims and returns a JWT token.
    ///
    /// The `Claims` type parameter represents the structure of the JWT claims to be signed.
    fn sign<Claims>(&self, claims: &Claims) -> Result<String, AuthError>
    where
        Claims: Serialize;
}

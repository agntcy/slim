// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common traits for authentication mechanisms.

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::errors::AuthError;

/// Trait for verifying JWT tokens
#[async_trait]
pub trait Verifier {
    /// Verifies the JWT token and returns the claims if valid.
    ///
    /// The `Claims` type parameter represents the expected structure of the JWT claims.
    async fn verify<Claims>(&mut self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;
}

/// Trait for signing JWT claims
pub trait Signer {
    /// Signs the claims and returns a JWT token.
    ///
    /// The `Claims` type parameter represents the structure of the JWT claims to be signed.
    fn sign<Claims>(&self, claims: &Claims) -> Result<String, AuthError>
    where
        Claims: Serialize;
}

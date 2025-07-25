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
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StandardClaims {
    /// Issuer (who issued the JWT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,

    /// Subject (whom the JWT is about)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,

    /// Audience (who the JWT is intended for)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<Vec<String>>,

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

impl StandardClaims {
    /// Creates a new instance of `StandardClaims` with the required fields.
    pub fn new(exp: u64) -> Self {
        Self {
            iss: None,
            sub: None,
            aud: None,
            exp,
            iat: None,
            jti: None,
            nbf: None,
            custom_claims: HashMap::new(),
        }
    }
}

/// Trait for verifying JWT tokens
#[async_trait]
pub trait Verifier {
    /// Verifies the JWT token and returns the claims if valid.
    ///
    /// The `Claims` type parameter represents the expected structure of the JWT claims.
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;

    /// Try to verify the JWT token without async context and return the claims if valid.
    /// If an async operation is needed, an error is returned.
    fn try_verify<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
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

    /// Sign standard claims and return a JWT token.
    fn sign_standard_claims(&self) -> Result<String, AuthError>;
}

/// Trait for providing JWT claims
pub trait TokenProvider {
    // Try to get a token
    fn get_token(&self) -> Result<String, AuthError>;
}

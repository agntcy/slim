// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common traits for authentication mechanisms.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;
use crate::metadata::MetadataMap;

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
    pub custom_claims: MetadataMap,
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
            custom_claims: MetadataMap::new(),
        }
    }
}

/// Trait for verifying JWT tokens
#[async_trait]
pub trait Verifier {
    /// Initializes the verifier asynchronously.
    async fn initialize(&mut self) -> Result<(), AuthError>;

    /// Verifies the token.
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError>;

    /// Try to verify the token without async context.
    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError>;

    /// Gets the claims from the token after verification.
    /// The `Claims` type parameter represents the expected structure of the JWT claims.
    async fn get_claims<Claims>(
        &self,
        token: impl Into<String> + Send,
    ) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;

    /// Try to get claims from the token without async context.
    /// If an async operation is needed, an error is returned.
    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
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
#[async_trait]
pub trait TokenProvider {
    /// Initializes the token provider asynchronously.
    /// Usage notes:
    /// - For most lightweight providers (SharedSecret, SignerJwt, StaticTokenProvider) this is a
    ///   no-op and can be safely skipped.
    /// - For providers that manage background tasks or external resources (e.g. `SpireIdentityManager`
    ///   and `OidcTokenProvider`), call their inherent `initialize` method (e.g.
    ///   `spire_identity_manager.initialize().await`) immediately after construction. The trait-level
    ///   `initialize` is currently a no-op placeholder to satisfy generic constraints.
    /// - Framework code that receives a generic `P: TokenProvider` MAY call `initialize` for
    ///   uniformity; implementations that need real setup should rely on an inherent method.
    async fn initialize(&mut self) -> Result<(), AuthError>;

    /// Try to get a token
    fn get_token(&self) -> Result<String, AuthError>;

    /// Try to get a token with custom claims
    ///
    /// # Arguments
    /// * `custom_claims` - Additional custom claims to include in the token.
    async fn get_token_with_claims(&self, custom_claims: MetadataMap) -> Result<String, AuthError> {
        // Default implementation ignores custom claims for backward compatibility
        let _ = custom_claims;
        self.get_token()
    }

    /// Get ID from the identity provider, e.g. the sub claim in JWT
    fn get_id(&self) -> Result<String, AuthError>;
}

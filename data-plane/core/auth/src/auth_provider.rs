// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Unified authentication provider and verifier enums
//!
//! This module provides consolidated enums for all authentication providers and verifiers
//! used throughout the SLIM authentication system. It serves as a single point of reference
//! for all supported authentication mechanisms.
//!
//! # Overview
//!
//! The SLIM authentication system supports multiple authentication mechanisms:
//! - **JWT (JSON Web Tokens)**: For cryptographically secure token-based authentication
//! - **Shared Secrets**: For simple symmetric key authentication
//! - **Static Tokens**: For file-based token provisioning
//!
//! This module consolidates these mechanisms into two main enums:
//! - [`AuthProvider`]: For generating authentication tokens
//! - [`AuthVerifier`]: For verifying authentication tokens
//!
//! # Examples
//!
//! ## Creating an authentication provider
//!
//! ```rust
//! use slim_auth::auth_provider::AuthProvider;
//!
//! // Create a shared secret provider
//! let provider = AuthProvider::shared_secret_from_str("service-id", "my-secret");
//! let token = provider.get_token().unwrap();
//! ```
//!
//! ## Creating an authentication verifier
//!
//! ```rust
//! use slim_auth::auth_provider::AuthVerifier;
//!
//! // Create a shared secret verifier
//! let verifier = AuthVerifier::shared_secret_from_str("service-id", "my-secret");
//!
//! // Verify a token
//! # tokio_test::block_on(async {
//! let result = verifier.verify("my-secret:service-id").await;
//! assert!(result.is_ok());
//! # });
//! ```
//!
//! ## Thread-safe usage
//!
//! Both enums implement `Clone` and can be wrapped in `Arc` for multi-threaded usage:
//!
//! ```rust
//! use std::sync::Arc;
//! use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
//!
//! let provider = Arc::new(AuthProvider::shared_secret_from_str("id", "secret"));
//! let verifier = Arc::new(AuthVerifier::shared_secret_from_str("id", "secret"));
//!
//! // These can now be safely shared across threads
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;
use crate::jwt::{SignerJwt, StaticTokenProvider, VerifierJwt};
use crate::shared_secret::SharedSecret;
use crate::traits::{TokenProvider, Verifier};

/// Unified enum for all authentication token providers
///
/// This enum consolidates all available token provider implementations
/// into a single type that can be used throughout the system.
///
/// # Variants
///
/// - `JwtSigner`: Uses JWT signing keys to generate cryptographically secure tokens
/// - `StaticToken`: Reads pre-generated tokens from files with optional file watching
/// - `SharedSecret`: Generates simple tokens based on shared secrets
///
/// # Thread Safety
///
/// This enum implements `Clone` and all variants are thread-safe, making it suitable
/// for use in multi-threaded environments when wrapped in `Arc`.
///
/// # Examples
///
/// ```rust
/// use slim_auth::auth_provider::AuthProvider;
/// use slim_auth::traits::TokenProvider;
///
/// // Create from shared secret
/// let provider = AuthProvider::shared_secret_from_str("my-service", "secret-key");
/// let token = provider.get_token().unwrap();
/// assert_eq!(token, "secret-key:my-service");
/// ```
#[derive(Clone)]
pub enum AuthProvider {
    /// JWT-based token provider using signing keys
    JwtSigner(SignerJwt),

    /// Static token provider that reads tokens from files
    StaticToken(StaticTokenProvider),

    /// Shared secret-based token provider
    SharedSecret(SharedSecret),
}

/// Unified enum for all authentication token verifiers
///
/// This enum consolidates all available token verifier implementations
/// into a single type that can be used throughout the system.
///
/// # Variants
///
/// - `JwtVerifier`: Uses JWT verification keys to validate cryptographically signed tokens
/// - `SharedSecret`: Validates tokens based on shared secrets
///
/// # Thread Safety
///
/// This enum implements `Clone` and all variants are thread-safe, making it suitable
/// for use in multi-threaded environments when wrapped in `Arc`.
///
/// # Examples
///
/// ```rust
/// use slim_auth::auth_provider::AuthVerifier;
/// use slim_auth::traits::Verifier;
///
/// // Create from shared secret
/// let verifier = AuthVerifier::shared_secret_from_str("my-service", "secret-key");
///
/// # tokio_test::block_on(async {
/// // Verify a valid token
/// let result = verifier.verify("secret-key:my-service").await;
/// assert!(result.is_ok());
///
/// // Verify an invalid token
/// let result = verifier.verify("wrong-key:my-service").await;
/// assert!(result.is_err());
/// # });
/// ```
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AuthVerifier {
    /// JWT-based token verifier using verification keys
    JwtVerifier(VerifierJwt),

    /// Shared secret-based token verifier
    SharedSecret(SharedSecret),
}

impl std::fmt::Debug for AuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthProvider::JwtSigner(_) => f.debug_tuple("JwtSigner").field(&"<SignerJwt>").finish(),
            AuthProvider::StaticToken(_) => f
                .debug_tuple("StaticToken")
                .field(&"<StaticTokenProvider>")
                .finish(),
            AuthProvider::SharedSecret(secret) => {
                f.debug_tuple("SharedSecret").field(secret).finish()
            }
        }
    }
}

impl std::fmt::Debug for AuthVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthVerifier::JwtVerifier(_) => f
                .debug_tuple("JwtVerifier")
                .field(&"<VerifierJwt>")
                .finish(),
            AuthVerifier::SharedSecret(secret) => {
                f.debug_tuple("SharedSecret").field(secret).finish()
            }
        }
    }
}

impl TokenProvider for AuthProvider {
    fn get_token(&self) -> Result<String, AuthError> {
        match self {
            AuthProvider::JwtSigner(signer) => signer.get_token(),
            AuthProvider::StaticToken(provider) => provider.get_token(),
            AuthProvider::SharedSecret(secret) => secret.get_token(),
        }
    }
}

#[async_trait]
impl Verifier for AuthVerifier {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        match self {
            AuthVerifier::JwtVerifier(verifier) => verifier.verify(token).await,
            AuthVerifier::SharedSecret(secret) => secret.verify(token).await,
        }
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        match self {
            AuthVerifier::JwtVerifier(verifier) => verifier.try_verify(token),
            AuthVerifier::SharedSecret(secret) => secret.try_verify(token),
        }
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            AuthVerifier::JwtVerifier(verifier) => verifier.get_claims(token).await,
            AuthVerifier::SharedSecret(secret) => secret.get_claims(token).await,
        }
    }

    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            AuthVerifier::JwtVerifier(verifier) => verifier.try_get_claims(token),
            AuthVerifier::SharedSecret(secret) => secret.try_get_claims(token),
        }
    }
}

/// Convenience constructors and utility methods for [`AuthProvider`]
impl AuthProvider {
    /// Create a new JWT signer provider
    ///
    /// # Arguments
    /// * `signer` - A configured JWT signer instance
    ///
    /// # Examples
    /// ```rust,ignore
    /// use slim_auth::auth_provider::AuthProvider;
    ///
    /// let signer = /* create JWT signer */;
    /// let provider = AuthProvider::jwt_signer(signer);
    /// ```
    pub fn jwt_signer(signer: SignerJwt) -> Self {
        AuthProvider::JwtSigner(signer)
    }

    /// Create a new static token provider
    ///
    /// # Arguments
    /// * `provider` - A configured static token provider instance
    ///
    /// # Examples
    /// ```rust,ignore
    /// use slim_auth::auth_provider::AuthProvider;
    ///
    /// let provider = /* create static token provider */;
    /// let auth_provider = AuthProvider::static_token(provider);
    /// ```
    pub fn static_token(provider: StaticTokenProvider) -> Self {
        AuthProvider::StaticToken(provider)
    }

    /// Create a new shared secret provider
    ///
    /// # Arguments
    /// * `secret` - A configured shared secret instance
    ///
    /// # Examples
    /// ```rust
    /// use slim_auth::auth_provider::AuthProvider;
    /// use slim_auth::shared_secret::SharedSecret;
    ///
    /// let secret = SharedSecret::new("service-id", "my-secret");
    /// let provider = AuthProvider::shared_secret(secret);
    /// ```
    pub fn shared_secret(secret: SharedSecret) -> Self {
        AuthProvider::SharedSecret(secret)
    }

    /// Create a new shared secret provider from ID and secret string
    ///
    /// This is a convenience method that creates the underlying `SharedSecret`
    /// instance automatically.
    ///
    /// # Arguments
    /// * `id` - The service/entity identifier
    /// * `secret` - The shared secret string
    ///
    /// # Examples
    /// ```rust
    /// use slim_auth::auth_provider::AuthProvider;
    /// use slim_auth::traits::TokenProvider;
    ///
    /// let provider = AuthProvider::shared_secret_from_str("my-service", "secret123");
    /// let token = provider.get_token().unwrap();
    /// assert_eq!(token, "secret123:my-service");
    /// ```
    pub fn shared_secret_from_str(id: &str, secret: &str) -> Self {
        AuthProvider::SharedSecret(SharedSecret::new(id, secret))
    }
}

/// Convenience constructors and utility methods for [`AuthVerifier`]
impl AuthVerifier {
    /// Create a new JWT verifier
    ///
    /// # Arguments
    /// * `verifier` - A configured JWT verifier instance
    ///
    /// # Examples
    /// ```rust,ignore
    /// use slim_auth::auth_provider::AuthVerifier;
    ///
    /// let jwt_verifier = /* create JWT verifier */;
    /// let verifier = AuthVerifier::jwt_verifier(jwt_verifier);
    /// ```
    pub fn jwt_verifier(verifier: VerifierJwt) -> Self {
        AuthVerifier::JwtVerifier(verifier)
    }

    /// Create a new shared secret verifier
    ///
    /// # Arguments
    /// * `secret` - A configured shared secret instance
    ///
    /// # Examples
    /// ```rust
    /// use slim_auth::auth_provider::AuthVerifier;
    /// use slim_auth::shared_secret::SharedSecret;
    ///
    /// let secret = SharedSecret::new("service-id", "my-secret");
    /// let verifier = AuthVerifier::shared_secret(secret);
    /// ```
    pub fn shared_secret(secret: SharedSecret) -> Self {
        AuthVerifier::SharedSecret(secret)
    }

    /// Create a new shared secret verifier from ID and secret string
    ///
    /// This is a convenience method that creates the underlying `SharedSecret`
    /// instance automatically.
    ///
    /// # Arguments
    /// * `id` - The service/entity identifier
    /// * `secret` - The shared secret string
    ///
    /// # Examples
    /// ```rust
    /// use slim_auth::auth_provider::AuthVerifier;
    /// use slim_auth::traits::Verifier;
    ///
    /// let verifier = AuthVerifier::shared_secret_from_str("my-service", "secret123");
    ///
    /// # tokio_test::block_on(async {
    /// let result = verifier.verify("secret123:my-service").await;
    /// assert!(result.is_ok());
    /// # });
    /// ```
    pub fn shared_secret_from_str(id: &str, secret: &str) -> Self {
        AuthVerifier::SharedSecret(SharedSecret::new(id, secret))
    }
}

/// Thread-safe wrapper types for use in multi-threaded contexts
///
/// These type aliases provide convenient `Arc`-wrapped versions of the main enums
/// for use in scenarios where the authenticators need to be shared across threads.
pub type AuthProviderArc = Arc<AuthProvider>;
pub type AuthVerifierArc = Arc<AuthVerifier>;

/// Configuration enum for token providers with serialization support
///
/// This enum is used for configuration purposes and can be serialized/deserialized
/// from configuration files. It contains minimal configuration data needed to identify
/// the provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TokenProviderType {
    /// JWT signer type
    JwtSigner,
    /// Static token type
    StaticToken,
    /// Shared secret type
    SharedSecret,
}

/// Configuration enum for token verifiers with serialization support
///
/// This enum is used for configuration purposes and can be serialized/deserialized
/// from configuration files. It contains minimal configuration data needed to identify
/// the verifier type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TokenVerifierType {
    /// JWT verifier type
    JwtVerifier,
    /// Shared secret type
    SharedSecret,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_provider_shared_secret_constructor() {
        let provider = AuthProvider::shared_secret_from_str("test-id", "test-secret");

        match provider {
            AuthProvider::SharedSecret(secret) => {
                assert_eq!(secret.id(), "test-id");
                assert_eq!(secret.shared_secret(), "test-secret");
            }
            _ => panic!("Expected SharedSecret variant"),
        }
    }

    #[test]
    fn test_auth_verifier_shared_secret_constructor() {
        let verifier = AuthVerifier::shared_secret_from_str("test-id", "test-secret");

        match verifier {
            AuthVerifier::SharedSecret(secret) => {
                assert_eq!(secret.id(), "test-id");
                assert_eq!(secret.shared_secret(), "test-secret");
            }
            _ => panic!("Expected SharedSecret variant"),
        }
    }

    #[test]
    fn test_token_provider_type_serialization() {
        let provider_type = TokenProviderType::SharedSecret;
        let json = serde_json::to_string(&provider_type).unwrap();
        assert_eq!(json, r#""shared_secret""#);

        let deserialized: TokenProviderType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TokenProviderType::SharedSecret);
    }

    #[test]
    fn test_token_verifier_type_serialization() {
        let verifier_type = TokenVerifierType::SharedSecret;
        let json = serde_json::to_string(&verifier_type).unwrap();
        assert_eq!(json, r#""shared_secret""#);

        let deserialized: TokenVerifierType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TokenVerifierType::SharedSecret);
    }

    #[tokio::test]
    async fn test_auth_provider_token_generation() {
        let provider = AuthProvider::shared_secret_from_str("test-id", "test-secret");
        let token = provider.get_token().unwrap();
        assert_eq!(token, "test-secret:test-id");
    }

    #[tokio::test]
    async fn test_auth_verifier_token_verification() {
        let verifier = AuthVerifier::shared_secret_from_str("test-id", "test-secret");
        let token = "test-secret:test-id";

        assert!(verifier.verify(token).await.is_ok());
        assert!(verifier.try_verify(token).is_ok());

        // Test invalid token
        let invalid_token = "wrong-secret:test-id";
        assert!(verifier.verify(invalid_token).await.is_err());
        assert!(verifier.try_verify(invalid_token).is_err());
    }

    #[test]
    fn test_auth_provider_cloning() {
        let provider = AuthProvider::shared_secret_from_str("test-id", "test-secret");
        let cloned_provider = provider.clone();

        // Both should generate the same token
        assert_eq!(
            provider.get_token().unwrap(),
            cloned_provider.get_token().unwrap()
        );
    }

    #[test]
    fn test_auth_verifier_cloning() {
        let verifier = AuthVerifier::shared_secret_from_str("test-id", "test-secret");
        let cloned_verifier = verifier.clone();
        let token = "test-secret:test-id";

        // Both should verify the same way
        assert!(verifier.try_verify(token).is_ok());
        assert!(cloned_verifier.try_verify(token).is_ok());
    }

    #[test]
    fn test_auth_provider_debug() {
        let provider = AuthProvider::shared_secret_from_str("test-id", "test-secret");
        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("SharedSecret"));
        assert!(debug_str.contains("test-id"));
        assert!(debug_str.contains("test-secret"));
    }

    #[test]
    fn test_auth_verifier_debug() {
        let verifier = AuthVerifier::shared_secret_from_str("test-id", "test-secret");
        let debug_str = format!("{:?}", verifier);
        assert!(debug_str.contains("SharedSecret"));
        assert!(debug_str.contains("test-id"));
        assert!(debug_str.contains("test-secret"));
    }
}

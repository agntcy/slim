// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Common traits for authentication mechanisms.

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
#[trait_variant::make(Send)]
pub trait Verifier {
    /// Initializes the verifier asynchronously.
    async fn initialize(&mut self) -> Result<(), AuthError>;

    /// Verifies the token.
    async fn verify(&self, token: impl AsRef<str> + Send) -> Result<(), AuthError>;

    /// Try to verify the token without async context.
    fn try_verify(&self, token: impl AsRef<str>) -> Result<(), AuthError>;

    /// Gets the claims from the token after verification.
    /// The `Claims` type parameter represents the expected structure of the JWT claims.
    async fn get_claims<Claims>(&self, token: impl AsRef<str> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send;

    /// Try to get claims from the token without async context.
    /// If an async operation is needed, an error is returned.
    fn try_get_claims<Claims>(&self, token: impl AsRef<str>) -> Result<Claims, AuthError>
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

/// A provider's persistable identity, captured so an app can be restored
/// verbatim across a restart.
///
/// Providers that support full-identity persistence return this from
/// [`TokenProvider::export_identity`] and accept it in
/// [`TokenProvider::with_restored_identity`]. `credential` is opaque,
/// provider-specific material (for `SharedSecret`, the shared secret bytes) and
/// is sensitive — persist it only in an encrypted store.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportedIdentity {
    /// Stable identity string (e.g. `SharedSecret`'s `<base_id>_<suffix>`).
    pub id: String,
    /// Provider credential material (e.g. the shared secret). Sensitive.
    pub credential: Vec<u8>,
    /// MLS signature secret key bytes.
    pub signature_secret_key: Vec<u8>,
    /// MLS signature public key bytes.
    pub signature_public_key: Vec<u8>,
}

/// Trait for providing JWT claims
#[trait_variant::make(Send)]
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

    /// Get ID from the identity provider, e.g. the sub claim in JWT
    fn get_id(&self) -> Result<String, AuthError>;

    /// Get the MLS signature key pair as `(secret_key_bytes, public_key_bytes)`.
    ///
    /// Returned as a single pair so callers can never observe a torn
    /// (secret, public) combination from a concurrent [`set_signature_keys`]
    /// rotation — header signing selects its algorithm from the public-key
    /// length, so a mismatched pair would make signing fail. Returns
    /// `Err(AuthError::MlsNotSupported)` by default.
    fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        Err(AuthError::MlsNotSupported)
    }

    /// Whether MLS-ciphersuite-correct signature keys have been installed via
    /// [`set_signature_keys`].
    ///
    /// Providers may pre-populate placeholder keys at construction that are not
    /// valid for the MLS ciphersuite in use (which is configurable and may
    /// differ from the provider's default). The MLS layer uses this to decide
    /// whether to generate a fresh key pair via its own crypto provider (first
    /// use) or reuse the already-installed one — the latter lets an app and all
    /// sessions cloned from it share a single signing identity. Defaults to
    /// `false`.
    fn mls_signature_keys_installed(&self) -> bool {
        false
    }

    /// Replace the MLS signature key pair with externally-generated keys.
    ///
    /// The MLS layer always generates ciphersuite-correct keys via its crypto
    /// provider and pushes them here so the identity provider can embed the
    /// public key in tokens. Implementations should treat this as marking
    /// [`mls_signature_keys_installed`] true.
    async fn set_signature_keys(
        &mut self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError>;

    /// Export the provider's persistable identity (stable id, credential, MLS
    /// keypair), or `None` if this provider does not support full-identity
    /// persistence. Used to save an app's identity so it can be restored
    /// verbatim after a restart.
    fn export_identity(&self) -> Option<ExportedIdentity> {
        None
    }

    /// Rebuild this provider from a previously [`export_identity`]-ed identity,
    /// so it presents the same id and MLS keys as before a restart. The default
    /// returns `self` unchanged (providers without persistable identity). MLS
    /// keys installed this way count as installed
    /// ([`mls_signature_keys_installed`] becomes true).
    fn with_restored_identity(self, identity: ExportedIdentity) -> Result<Self, AuthError>
    where
        Self: Sized,
    {
        let _ = identity;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal provider that implements only the required methods, leaving the
    /// MLS signature-key methods to their default `MlsNotSupported` impls.
    struct MinimalProvider;

    impl TokenProvider for MinimalProvider {
        async fn initialize(&mut self) -> Result<(), AuthError> {
            Ok(())
        }

        fn get_token(&self) -> Result<String, AuthError> {
            Ok("token".to_string())
        }

        fn get_id(&self) -> Result<String, AuthError> {
            Ok("id".to_string())
        }

        async fn set_signature_keys(
            &mut self,
            _private_key: Vec<u8>,
            _public_key: Vec<u8>,
        ) -> Result<(), AuthError> {
            Err(AuthError::MlsNotSupported)
        }
    }

    #[tokio::test]
    async fn default_mls_methods_return_not_supported() {
        let mut p = MinimalProvider;
        assert!(matches!(
            p.get_signature_keys(),
            Err(AuthError::MlsNotSupported)
        ));
        assert!(matches!(
            p.set_signature_keys(vec![1, 2, 3], vec![4, 5, 6]).await,
            Err(AuthError::MlsNotSupported)
        ));

        // The required methods remain usable on the same minimal provider.
        assert_eq!(p.get_token().unwrap(), "token");
        assert_eq!(p.get_id().unwrap(), "id");
    }

    #[tokio::test]
    async fn default_initialize_is_a_noop() {
        let mut p = MinimalProvider;
        assert!(p.initialize().await.is_ok());
    }
}

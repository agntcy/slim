// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Builder pattern implementation for auth components.

use core::panic;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::Duration;

use jsonwebtoken_aws_lc::{Algorithm, DecodingKey, EncodingKey, Validation};

use crate::errors::AuthError;
use crate::jwt::{Key, KeyData, SignerJwt, VerifierJwt};
use crate::resolver::KeyResolver;
use crate::traits::StandardClaims;

/// State markers for the JWT builder state machine.
///
/// This module defines empty structs that act as phantom types for the state machine pattern.
/// Each struct represents a specific state in the JWT building process, enforcing the correct
/// sequence of method calls at compile time.
///
/// The state transitions are as follows:
/// `Initial` -> `WithPrivateKey` -> -> `Final` -> `Jwt`
/// Or
/// `Initial` -> `WithPublicKey` -> `Final` -> `Jwt`
pub mod state {
    /// Initial state for the JWT builder.
    ///
    /// This state allows setting either a public or a private key
    pub struct Initial;

    /// State after setting public key.
    ///
    /// This state allows configuring additional parameters like a validator.
    pub struct WithPrivateKey;

    /// State after setting private key
    pub struct WithPublicKey;

    /// State after setting a token
    pub struct WithToken;

    /// Final state, ready to build the JWT instance.
    ///
    /// This state can only be reached after all required configuration is complete.
    pub struct Final;
}

/// Builder for JWT Authentication configuration.
///
/// The builder uses type state to enforce the correct sequence of method calls.
/// The state transitions are:
///
/// 1. `Initial`: The starting state with no configuration
/// 2. `WithPrivateKey`: After setting a private key
/// 3. `WithPublicKey`: After setting a public key or enabling auto-resolve
///
/// Each method transitions the builder to the appropriate state, ensuring at
/// compile time that all required information is provided.
pub struct JwtBuilder<S = state::Initial> {
    // Required fields
    issuer: Option<String>,
    audience: Option<String>,
    subject: Option<String>,

    // Private and public keys
    private_key: Option<String>,
    public_key: Option<String>,
    algorithm: Algorithm,

    // Token settings
    token_duration: Duration,

    // Key resolution
    auto_resolve_keys: bool,

    // Required claims
    required_claims: Vec<String>,

    // Custom claims
    custom_claims: HashMap<String, serde_json::Value>,

    // Token file
    token_file: Option<String>,

    // PhantomData to track state
    _state: PhantomData<S>,
}

impl Default for JwtBuilder<state::Initial> {
    fn default() -> Self {
        Self {
            issuer: None,
            audience: None,
            subject: None,
            private_key: None,
            public_key: None,
            algorithm: Algorithm::HS256, // Default algorithm
            token_duration: Duration::from_secs(3600), // Default 1 hour
            auto_resolve_keys: false,
            required_claims: Vec::new(),
            custom_claims: HashMap::new(),
            token_file: None,
            _state: PhantomData,
        }
    }
}

// Base implementation for any state
impl<S> JwtBuilder<S> {
    fn build_validation(&self) -> Validation {
        let mut validation = Validation::new(self.algorithm);
        if let Some(audience) = &self.audience {
            validation.set_audience(&[audience]);
        }
        if let Some(issuer) = &self.issuer {
            validation.set_issuer(&[issuer]);
        }

        if !self.required_claims.is_empty() {
            validation.set_required_spec_claims(self.required_claims.as_ref());
        }

        validation
    }

    fn build_claims(&self) -> StandardClaims {
        StandardClaims {
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            sub: self.subject.clone(),
            exp: 0,    // Will be set later
            iat: None, // Will be set later
            nbf: None, // Will be set later
            jti: None, // Will be set later
            custom_claims: self.custom_claims.clone(),
        }
    }

    fn resolve_key(&self, key: &Key) -> String {
        // Resolve private key from Key enum
        match &key.key {
            KeyData::Pem(key) => key.clone(),
            KeyData::File(path) => {
                // TODO(msardara/micpapal): here we can use the file watcher
                std::fs::read_to_string(path).unwrap()
            }
        }
    }
}

// Implementation for the Initial state
impl JwtBuilder<state::Initial> {
    /// Create a new JWT builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the issuer for the JWT tokens.
    pub fn issuer(self, issuer: impl Into<String>) -> Self {
        Self {
            issuer: Some(issuer.into()),
            ..self
        }
    }

    /// Set the audience for the JWT tokens.
    pub fn audience(self, audience: impl Into<String>) -> Self {
        Self {
            audience: Some(audience.into()),
            ..self
        }
    }

    /// Set the subject for the JWT tokens.
    pub fn subject(self, subject: impl Into<String>) -> Self {
        Self {
            subject: Some(subject.into()),
            ..self
        }
    }

    /// Require exp (claims expiration) in the JWT.
    pub fn require_exp(self) -> Self {
        let mut required_claims = self.required_claims.clone();
        required_claims.push("exp".to_string());
        Self {
            required_claims,
            ..self
        }
    }

    /// Require nbf (not before) in the JWT.
    pub fn require_nbf(self) -> Self {
        let mut required_claims = self.required_claims.clone();
        required_claims.push("nbf".to_string());
        Self {
            required_claims,
            ..self
        }
    }

    /// Require aud (audience) in the JWT.
    pub fn require_aud(self) -> Self {
        let mut required_claims = self.required_claims.clone();
        required_claims.push("aud".to_string());
        Self {
            required_claims,
            ..self
        }
    }

    /// Require iss (issuer) in the JWT.
    pub fn require_iss(self) -> Self {
        let mut required_claims = self.required_claims.clone();
        required_claims.push("iss".to_string());
        Self {
            required_claims,
            ..self
        }
    }

    /// Require sub (subject) in the JWT.
    pub fn require_sub(self) -> Self {
        let mut required_claims = self.required_claims.clone();
        required_claims.push("sub".to_string());
        Self {
            required_claims,
            ..self
        }
    }

    /// Set the private key and transition to WithPrivateKey state.
    pub fn private_key(self, key: &Key) -> JwtBuilder<state::WithPrivateKey> {
        let private_key = self.resolve_key(key);

        JwtBuilder::<state::WithPrivateKey> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: Some(private_key),
            public_key: None,
            algorithm: key.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            required_claims: self.required_claims,
            custom_claims: self.custom_claims,
            token_file: None,
            _state: PhantomData,
        }
    }

    /// Set the public key and transition to WithPublicKey state.
    pub fn public_key(self, key: &Key) -> JwtBuilder<state::WithPublicKey> {
        let public_key = self.resolve_key(key);

        JwtBuilder::<state::WithPublicKey> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: None,
            public_key: Some(public_key),
            algorithm: key.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            required_claims: self.required_claims,
            custom_claims: self.custom_claims,
            token_file: None,
            _state: PhantomData,
        }
    }

    /// Enable automatic key resolution and transition to WithPublicKey state.
    pub fn auto_resolve_keys(self, enable: bool) -> JwtBuilder<state::WithPublicKey> {
        JwtBuilder::<state::WithPublicKey> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: None,
            public_key: None,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: enable,
            required_claims: self.required_claims,
            custom_claims: self.custom_claims,
            token_file: None,
            _state: PhantomData,
        }
    }

    pub fn token_file(self, token_file: impl Into<String>) -> JwtBuilder<state::WithToken> {
        // This method is not implemented yet, but it can be used to set a token file
        // and transition to a state that handles token files.
        JwtBuilder::<state::WithToken> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            required_claims: self.required_claims,
            custom_claims: self.custom_claims,
            token_file: Some(token_file.into()),
            _state: PhantomData,
        }
    }
}

// Implementation for the RequiredInfo state
impl JwtBuilder<state::WithPrivateKey> {
    /// Set the token duration in seconds.
    pub fn token_duration(self, duration: Duration) -> Self {
        Self {
            token_duration: duration,
            ..self
        }
    }

    /// Set custom claims
    pub fn custom_claims(self, claims: HashMap<String, serde_json::Value>) -> Self {
        Self {
            custom_claims: claims,
            ..self
        }
    }

    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<SignerJwt, AuthError> {
        // Set up validation
        let validation = self.build_validation();

        // Configure encoding key
        let encoding_key = match &self.private_key {
            Some(key) => {
                let key_str = key.as_str();
                match self.algorithm {
                    Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                        Some(EncodingKey::from_secret(key_str.as_bytes()))
                    }
                    Algorithm::RS256
                    | Algorithm::RS384
                    | Algorithm::RS512
                    | Algorithm::PS256
                    | Algorithm::PS384
                    | Algorithm::PS512 => {
                        // PEM-encoded private key
                        Some(EncodingKey::from_rsa_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid RSA private key: {}", e))
                        })?)
                    }
                    Algorithm::ES256 | Algorithm::ES384 => {
                        // PEM-encoded EC private key
                        Some(EncodingKey::from_ec_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid EC private key: {}", e))
                        })?)
                    }
                    Algorithm::EdDSA => {
                        // PEM-encoded EdDSA private key
                        Some(EncodingKey::from_ed_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid EdDSA private key: {}", e))
                        })?)
                    }
                }
            }
            None => {
                // This should never happen because we require a private key in this state
                panic!("Private key must be set in WithPrivateKey state");
            }
        };

        // Create new Jwt instance
        Ok(
            SignerJwt::new(self.build_claims(), self.token_duration, validation)
                .with_encoding_key(encoding_key.unwrap()),
        )
    }
}

// Implementation for the WithPublicKey state
impl JwtBuilder<state::WithPublicKey> {
    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<VerifierJwt, AuthError> {
        // Set up validation
        let validation = self.build_validation();

        let verifier = VerifierJwt::new(self.build_claims(), self.token_duration, validation);

        // Configure decoding key
        if self.auto_resolve_keys {
            // We'll auto-resolve keys, so we don't need to set it now
            Ok(verifier.with_key_resolver(KeyResolver::new()))
        } else {
            let decoding_key = match &self.public_key {
                Some(public_key) => {
                    // Use public key for verification
                    match self.algorithm {
                        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                            let key_str = public_key.as_str();
                            Some(DecodingKey::from_secret(key_str.as_bytes()))
                        }
                        Algorithm::RS256
                        | Algorithm::RS384
                        | Algorithm::RS512
                        | Algorithm::PS256
                        | Algorithm::PS384
                        | Algorithm::PS512 => {
                            // PEM-encoded public key
                            Some(
                                DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(|e| {
                                    AuthError::ConfigError(format!("Invalid RSA public key: {}", e))
                                })?,
                            )
                        }
                        Algorithm::ES256 | Algorithm::ES384 => {
                            // PEM-encoded EC public key
                            Some(
                                DecodingKey::from_ec_pem(public_key.as_bytes()).map_err(|e| {
                                    AuthError::ConfigError(format!("Invalid EC public key: {}", e))
                                })?,
                            )
                        }
                        Algorithm::EdDSA => {
                            // PEM-encoded EdDSA public key
                            Some(
                                DecodingKey::from_ed_pem(public_key.as_bytes()).map_err(|e| {
                                    AuthError::ConfigError(format!(
                                        "Invalid EdDSA public key: {}",
                                        e
                                    ))
                                })?,
                            )
                        }
                    }
                }
                None => {
                    // This should never happen because we require a public key in this state
                    panic!("Public key must be set in WithPublicKey state");
                }
            };

            Ok(verifier.with_decoding_key(decoding_key.unwrap()))
        }
    }
}

// Implementation for the WithToken state
impl JwtBuilder<state::WithToken> {
    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<SignerJwt, AuthError> {
        // Create new Jwt instance
        Ok(SignerJwt::new(
            self.build_claims(),               // not used
            std::time::Duration::from_secs(0), // not used
            self.build_validation(),           // not used
        )
        .with_token_file(self.token_file.unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{Signer, Verifier};
    use serde::{Deserialize, Serialize};
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use crate::testutils::initialize_crypto_provider;

    #[test]
    fn test_jwt_builder_basic() {
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("test-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = jwt.create_claims();

        assert_eq!(claims.iss.unwrap(), "test-issuer");
        assert_eq!(claims.aud.unwrap(), "test-audience");
        assert_eq!(claims.sub.unwrap(), "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_builder_sign_verify() {
        // Using the explicit state machine
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("test-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("test-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();
        let verified: crate::traits::StandardClaims = verifier.verify(&token).await.unwrap();

        assert_eq!(verified.iss.unwrap(), "test-issuer");
        assert_eq!(verified.aud.unwrap(), "test-audience");
        assert_eq!(verified.sub.unwrap(), "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_builder_custom_claims() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct CustomClaims {
            iss: String,
            aud: String,
            sub: String,
            exp: u64,
            role: String,
        }

        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("test-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("test-key".to_string()),
            })
            .build()
            .unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let custom_claims = CustomClaims {
            iss: "test-issuer".to_string(),
            aud: "test-audience".to_string(),
            sub: "test-subject".to_string(),
            exp: now + 3600,
            role: "admin".to_string(),
        };

        let token = signer.sign(&custom_claims).unwrap();
        let verified: CustomClaims = verifier.verify(&token).await.unwrap();

        assert_eq!(verified, custom_claims);
    }

    #[test]
    fn test_jwt_builder_auto_resolve_keys() {
        // Set crypto provider
        initialize_crypto_provider();

        // Using state machine with direct transition
        let jwt = JwtBuilder::new()
            .issuer("https://example.com")
            .audience("test-audience")
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build();
        assert!(jwt.is_ok());
    }
}

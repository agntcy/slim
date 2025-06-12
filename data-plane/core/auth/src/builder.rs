// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Builder pattern implementation for auth components.

use jsonwebtoken_aws_lc::{Algorithm, DecodingKey, EncodingKey, Validation};
use std::marker::PhantomData;
use std::time::Duration;

use crate::errors::AuthError;
use crate::jwt::Jwt;
use crate::resolver::KeyResolver;

/// State markers for the JWT builder state machine.
///
/// This module defines empty structs that act as phantom types for the state machine pattern.
/// Each struct represents a specific state in the JWT building process, enforcing the correct
/// sequence of method calls at compile time.
///
/// The state transitions are as follows:
/// `Initial` -> `RequiredInfo` -> `KeyConfig` -> `Final` -> `Jwt`
pub mod state {
    /// Initial state for the JWT builder.
    ///
    /// This state allows setting basic properties like issuer, audience, and subject.
    /// It doesn't allow building a Jwt instance directly.
    pub struct Initial;

    /// State after setting required info (issuer, audience, subject).
    ///
    /// This state allows configuring keys or enabling auto key resolution.
    pub struct RequiredInfo;

    /// State after setting key configuration (private/public keys or auto-resolution).
    ///
    /// This state allows either adding a key resolver or building the final Jwt instance.
    pub struct KeyConfig;

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
/// 2. `RequiredInfo`: After setting issuer, audience, and subject
/// 3. `KeyConfig`: After setting key configuration (private key, public key, or auto-resolve)
/// 4. `Final`: Ready to build the JWT
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
            _state: PhantomData,
        }
    }
}

// Base implementation for any state
impl<S> JwtBuilder<S> {
    /// Set the algorithm used for signing and verifying tokens.
    pub fn algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Set the token duration in seconds.
    pub fn token_duration(mut self, duration: Duration) -> Self {
        self.token_duration = duration;
        self
    }
}

// Implementation for the Initial state
impl JwtBuilder<state::Initial> {
    /// Create a new JWT builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the issuer for the JWT tokens.
    pub fn issuer(mut self, issuer: impl Into<String>) -> JwtBuilder<state::Initial> {
        self.issuer = Some(issuer.into());
        self
    }

    /// Set the audience for the JWT tokens.
    pub fn audience(mut self, audience: impl Into<String>) -> JwtBuilder<state::Initial> {
        self.audience = Some(audience.into());
        self
    }

    /// Set the subject for the JWT tokens.
    pub fn subject(mut self, subject: impl Into<String>) -> JwtBuilder<state::Initial> {
        self.subject = Some(subject.into());
        self
    }

    /// Set the private key and transition directly to KeyConfig state.
    /// This is a convenience method that validates required fields and transitions to KeyConfig.
    pub fn private_key(
        self,
        algorithm: Algorithm,
        private_key: impl Into<String>,
    ) -> Result<JwtBuilder<state::KeyConfig>, AuthError> {
        let required_info = self.with_required_info()?;
        Ok(required_info.private_key(algorithm, private_key))
    }

    /// Set the public key and transition directly to KeyConfig state.
    /// This is a convenience method that validates required fields and transitions to KeyConfig.
    pub fn public_key(
        self,
        algorithm: Algorithm,
        public_key: impl Into<String>,
    ) -> Result<JwtBuilder<state::KeyConfig>, AuthError> {
        let required_info = self.with_required_info()?;
        Ok(required_info.public_key(algorithm, public_key))
    }

    /// Enable automatic key resolution and transition directly to KeyConfig state.
    /// This is a convenience method that validates required fields and transitions to KeyConfig.
    pub fn auto_resolve_keys(
        self,
        enable: bool,
    ) -> Result<JwtBuilder<state::KeyConfig>, AuthError> {
        let required_info = self.with_required_info()?;
        Ok(required_info.auto_resolve_keys(enable))
    }

    /// Transition to RequiredInfo state once all required fields are set.
    ///
    /// This method validates that the issuer, audience, and subject are all present,
    /// and transitions the builder to the RequiredInfo state if they are.
    pub fn with_required_info(self) -> Result<JwtBuilder<state::RequiredInfo>, AuthError> {
        let issuer = self
            .issuer
            .ok_or_else(|| AuthError::ConfigError("Issuer is required".to_string()))?;
        let audience = self
            .audience
            .ok_or_else(|| AuthError::ConfigError("Audience is required".to_string()))?;
        let subject = self
            .subject
            .ok_or_else(|| AuthError::ConfigError("Subject is required".to_string()))?;

        Ok(JwtBuilder {
            issuer: Some(issuer),
            audience: Some(audience),
            subject: Some(subject),
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            _state: PhantomData,
        })
    }
}

// Implementation for the RequiredInfo state
impl JwtBuilder<state::RequiredInfo> {
    /// Set the private key used for signing tokens.
    pub fn private_key(
        mut self,
        algorithm: Algorithm,
        private_key: impl Into<String>,
    ) -> JwtBuilder<state::KeyConfig> {
        let key = private_key.into();
        assert!(!key.is_empty(), "Private key cannot be empty");

        self.private_key = Some(key);
        self.algorithm = algorithm;

        JwtBuilder {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            _state: PhantomData,
        }
    }

    /// Set the public key used for verifying tokens.
    pub fn public_key(
        mut self,
        algorithm: Algorithm,
        public_key: impl Into<String>,
    ) -> JwtBuilder<state::KeyConfig> {
        let key = public_key.into();
        assert!(!key.is_empty(), "Public key cannot be empty");

        self.public_key = Some(key);
        self.algorithm = algorithm;

        JwtBuilder {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            _state: PhantomData,
        }
    }

    /// Enable automatic key resolution from well-known endpoints
    pub fn auto_resolve_keys(mut self, enable: bool) -> JwtBuilder<state::KeyConfig> {
        self.auto_resolve_keys = enable;
        JwtBuilder {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            _state: PhantomData,
        }
    }
}

// Implementation for the KeyConfig state (ready to build)
impl JwtBuilder<state::KeyConfig> {
    /// Move to final state if all required configurations are set
    ///
    /// This public method allows testing the state transition explicitly.
    /// It validates that we have either keys or auto-resolution enabled.
    pub fn to_final_state(self) -> Result<JwtBuilder<state::Final>, AuthError> {
        // Validate that we have either a key or auto-resolution enabled
        if !self.auto_resolve_keys && self.private_key.is_none() && self.public_key.is_none() {
            return Err(AuthError::ConfigError(
                "Either private key, public key, or auto_resolve_keys must be set".to_string(),
            ));
        }

        Ok(JwtBuilder {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: self.private_key,
            public_key: self.public_key,
            algorithm: self.algorithm,
            token_duration: self.token_duration,
            auto_resolve_keys: self.auto_resolve_keys,
            _state: PhantomData,
        })
    }

    /// Build a new Jwt instance.
    ///
    /// This method transitions to the Final state and then builds the JWT.
    pub fn build(self) -> Result<Jwt, AuthError> {
        let final_state = self.to_final_state()?;
        final_state.build()
    }
}

// Implementation for the Final state begins below

// Implementation for the Final state
impl JwtBuilder<state::Final> {
    /// Build a new Jwt instance.
    pub fn build(self) -> Result<Jwt, AuthError> {
        // We know all required fields are set because we've validated in previous states
        let issuer = self.issuer.unwrap(); // Safe to unwrap
        let audience = self.audience.unwrap(); // Safe to unwrap
        let subject = self.subject.unwrap(); // Safe to unwrap

        // Set up validation
        let mut validation = Validation::new(self.algorithm);
        validation.set_audience(&[&audience]);
        validation.set_issuer(&[&issuer]);

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
            None => None,
        };

        // Configure decoding key
        let decoding_key = if self.auto_resolve_keys {
            // We'll auto-resolve keys, so we don't need to set it now
            None
        } else {
            match (&self.public_key, &self.private_key) {
                (Some(public_key), _) => {
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
                (None, Some(private_key)) => {
                    // Use private key for HMAC algorithms
                    match self.algorithm {
                        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                            let key_str = private_key.as_str();
                            Some(DecodingKey::from_secret(key_str.as_bytes()))
                        }
                        _ => {
                            if !self.auto_resolve_keys {
                                return Err(AuthError::ConfigError(
                                    "Public key required for asymmetric algorithms or enable auto_resolve_keys".to_string(),
                                ));
                            }
                            None
                        }
                    }
                }
                (None, None) => {
                    // We've already validated that either we have keys or auto_resolve_keys is enabled
                    None
                }
            }
        };

        // If auto-resolving keys, we need a key resolver
        let resolver = if self.auto_resolve_keys {
            Some(KeyResolver::new())
        } else {
            None
        };

        // Create new Jwt instance
        Ok(Jwt::new(
            issuer,
            audience,
            subject,
            self.token_duration,
            validation,
            encoding_key,
            decoding_key,
            resolver,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{Signer, Verifier};
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    #[test]
    fn test_jwt_builder_basic() {
        // Using the explicit state machine
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap()
            .private_key(Algorithm::HS512, "test-key")
            .build()
            .unwrap();

        let custom_claims = HashMap::new();
        let claims = jwt.create_standard_claims(Some(custom_claims));

        assert_eq!(claims.iss, "test-issuer");
        assert_eq!(claims.aud, "test-audience");
        assert_eq!(claims.sub, "test-subject");

        // Using direct transition
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "test-key")
            .unwrap()
            .build()
            .unwrap();

        let custom_claims = HashMap::new();
        let claims = jwt.create_standard_claims(Some(custom_claims));

        assert_eq!(claims.iss, "test-issuer");
        assert_eq!(claims.aud, "test-audience");
        assert_eq!(claims.sub, "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_builder_sign_verify() {
        // Using the explicit state machine
        let mut jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap()
            .private_key(Algorithm::HS512, "test-key")
            .build()
            .unwrap();

        let claims = jwt.create_standard_claims(None);
        let token = jwt.sign(&claims).unwrap();
        let verified: crate::jwt::StandardClaims = jwt.verify(&token).await.unwrap();

        assert_eq!(verified.iss, "test-issuer");
        assert_eq!(verified.aud, "test-audience");
        assert_eq!(verified.sub, "test-subject");

        // Using direct transition
        let mut jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "test-key")
            .unwrap()
            .build()
            .unwrap();

        let claims = jwt.create_standard_claims(None);
        let token = jwt.sign(&claims).unwrap();
        let verified: crate::jwt::StandardClaims = jwt.verify(&token).await.unwrap();

        assert_eq!(verified.iss, "test-issuer");
        assert_eq!(verified.aud, "test-audience");
        assert_eq!(verified.sub, "test-subject");
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

        // Using direct transition
        let mut jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "test-key")
            .unwrap()
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

        let token = jwt.sign(&custom_claims).unwrap();
        let verified: CustomClaims = jwt.verify(&token).await.unwrap();

        assert_eq!(verified, custom_claims);

        // Using the explicit state machine
        let mut jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap()
            .private_key(Algorithm::HS512, "test-key")
            .build()
            .unwrap();

        let token = jwt.sign(&custom_claims).unwrap();
        let verified: CustomClaims = jwt.verify(&token).await.unwrap();

        assert_eq!(verified, custom_claims);
    }

    #[test]
    fn test_jwt_builder_missing_fields() {
        // Testing explicit state machine pattern

        // Missing issuer - state machine
        let result = JwtBuilder::new()
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info();
        assert!(result.is_err());

        // Missing audience - state machine
        let result = JwtBuilder::new()
            .issuer("test-issuer")
            .subject("test-subject")
            .with_required_info();
        assert!(result.is_err());

        // Missing subject - state machine
        let result = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .with_required_info();
        assert!(result.is_err());

        // Required fields present but missing key - state machine
        let required_info = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap();

        // Try to build without setting key configuration

        // This should panic
        let result = std::panic::catch_unwind(|| {
            required_info
                .private_key(Algorithm::HS512, "")
                .to_final_state()
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_jwt_builder_auto_resolve_keys() {
        // Using state machine with direct transition
        let jwt = JwtBuilder::new()
            .issuer("https://example.com")
            .audience("test-audience")
            .subject("test-subject")
            .auto_resolve_keys(true)
            .unwrap()
            .build();
        assert!(jwt.is_ok());

        // Using full state machine
        let jwt = JwtBuilder::new()
            .issuer("https://example.com")
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap()
            .auto_resolve_keys(true)
            .build();
        assert!(jwt.is_ok());
    }
}

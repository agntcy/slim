// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Builder pattern implementation for auth components.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken_aws_lc::jwk::{Jwk, JwkSet};
use jsonwebtoken_aws_lc::{Algorithm, DecodingKey, EncodingKey, Validation};
use parking_lot::RwLock;

use crate::errors::AuthError;
use crate::file_watcher::FileWatcher;
use crate::jwt::{Key, KeyData, KeyFormat, SignerJwt, StaticTokenProvider, VerifierJwt};
use crate::metadata::MetadataMap;
use crate::resolver::KeyResolver;
use crate::traits::StandardClaims;

/// State markers for the JWT builder state machine.
///
/// This module defines empty structs that act as phantom types for the state machine pattern.
/// Each struct represents a specific state in the JWT building process, enforcing the correct
/// sequence of method calls at compile time.
///
/// The state transitions are as follows:
/// `Initial` -> `WithPrivateKey` -> `Jwt`
/// Or
/// `Initial` -> `WithPublicKey` -> `Jwt`
/// Or
/// `Initial` -> `WithToken` -> `Jwt`
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
    audience: Option<Vec<String>>,
    subject: Option<String>,

    // Private and public keys
    private_key: Option<Key>,
    public_key: Option<Key>,
    algorithm: Algorithm,

    // Token settings
    token_duration: Duration,

    // Key resolution
    auto_resolve_keys: bool,

    // Required claims
    required_claims: Vec<String>,

    // Custom claims
    custom_claims: MetadataMap,

    // Token file
    token_file: Option<String>,

    // PhantomData to track state
    _state: PhantomData<S>,
}

fn resolve_key(key: &Key) -> String {
    match &key.key {
        KeyData::Data(key) => key.clone(),
        KeyData::File(path) => std::fs::read_to_string(path).expect("error reading key file"),
    }
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
            custom_claims: MetadataMap::new(),
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
            tracing::info!(?audience, "Setting audience");
            validation.set_audience(audience);
        }
        if let Some(issuer) = &self.issuer {
            tracing::info!("Setting issuer: {}", issuer);
            validation.set_issuer(&[issuer]);
        }

        if !self.required_claims.is_empty() {
            tracing::info!("Setting required claims: {:?}", self.required_claims);
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
    pub fn audience(self, audience: &[impl Into<String> + Clone]) -> Self {
        Self {
            audience: Some(audience.iter().map(|a| a.clone().into()).collect()),
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
        JwtBuilder::<state::WithPrivateKey> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: Some(key.clone()),
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
        JwtBuilder::<state::WithPublicKey> {
            issuer: self.issuer,
            audience: self.audience,
            subject: self.subject,
            private_key: None,
            public_key: Some(key.clone()),
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
    pub fn custom_claims(self, claims: MetadataMap) -> Self {
        Self {
            custom_claims: claims,
            ..self
        }
    }

    fn build_internal(key: &Key) -> Result<EncodingKey, AuthError> {
        // JWK are not supported as encoding keys
        if key.format == KeyFormat::Jwk {
            return Err(AuthError::ConfigError(
                "JWK format is not supported for encoding keys".to_string(),
            ));
        }

        let key_str = resolve_key(key);
        match key.algorithm {
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                Ok(EncodingKey::from_secret(key_str.as_bytes()))
            }
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512 => {
                // PEM-encoded private key
                EncodingKey::from_rsa_pem(key_str.as_bytes())
                    .map_err(|e| AuthError::ConfigError(format!("Invalid RSA private key: {}", e)))
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                // PEM-encoded EC private key
                EncodingKey::from_ec_pem(key_str.as_bytes())
                    .map_err(|e| AuthError::ConfigError(format!("Invalid EC private key: {}", e)))
            }
            Algorithm::EdDSA => {
                // PEM-encoded EdDSA private key
                EncodingKey::from_ed_pem(key_str.as_bytes()).map_err(|e| {
                    AuthError::ConfigError(format!("Invalid EdDSA private key: {}", e))
                })
            }
        }
    }

    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<SignerJwt, AuthError> {
        // Set up validation
        let validation = self.build_validation();

        // Create encoding key, either from file or from inline PEM
        let key = Self::build_internal(self.private_key.as_ref().unwrap())?;

        // Transform it to Arc
        let encoding_key = Arc::new(RwLock::new(key));

        // Create a signer
        let signer = SignerJwt::new(self.build_claims(), self.token_duration, validation)
            .with_encoding_key(encoding_key.clone());

        // If the private key is a file, setup also a file watcher for it
        let signer = match &self.private_key.as_ref().unwrap().key {
            KeyData::File(path) => {
                // If the key is a file, we need to set up a file watcher
                let encoding_key_clone = encoding_key.clone();
                let key_clone = self.private_key.clone().unwrap();
                let mut w = FileWatcher::create_watcher(move |_file: &str| {
                    let new_key =
                        Self::build_internal(&key_clone).expect("error processing new key");
                    *encoding_key_clone.as_ref().write() = new_key;
                });
                w.add_file(path).expect("error adding file to the watcher");

                signer.with_watcher(w)
            }
            _ => signer,
        };

        // Return the signer
        Ok(signer)
    }
}

enum DecodingKeyInternal {
    DecKey(DecodingKey),
    Jwks(JwkSet),
}

// Implementation for the WithPublicKey state
impl JwtBuilder<state::WithPublicKey> {
    fn build_internal(key: &Key) -> Result<DecodingKeyInternal, AuthError> {
        let key_str = resolve_key(key);

        match &key.format {
            KeyFormat::Pem => {
                // Use public key for verification
                match key.algorithm {
                    Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => Ok(
                        DecodingKeyInternal::DecKey(DecodingKey::from_secret(key_str.as_bytes())),
                    ),
                    Algorithm::RS256
                    | Algorithm::RS384
                    | Algorithm::RS512
                    | Algorithm::PS256
                    | Algorithm::PS384
                    | Algorithm::PS512 => {
                        // PEM-encoded public key
                        let ret = DecodingKey::from_rsa_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid RSA public key: {}", e))
                        });

                        ret.map(DecodingKeyInternal::DecKey)
                    }
                    Algorithm::ES256 | Algorithm::ES384 => {
                        // PEM-encoded EC public key
                        let ret = DecodingKey::from_ec_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid EC public key: {}", e))
                        });

                        ret.map(DecodingKeyInternal::DecKey)
                    }
                    Algorithm::EdDSA => {
                        // PEM-encoded EdDSA public key
                        let ret = DecodingKey::from_ed_pem(key_str.as_bytes()).map_err(|e| {
                            AuthError::ConfigError(format!("Invalid EdDSA public key: {}", e))
                        });

                        ret.map(DecodingKeyInternal::DecKey)
                    }
                }
            }
            KeyFormat::Jwk => {
                // JWK format
                let jwk: Jwk = serde_json::from_str(&resolve_key(key))
                    .map_err(|e| AuthError::ConfigError(format!("Invalid JWK: {}", e)))?;
                let ret = DecodingKey::from_jwk(&jwk)
                    .map_err(|e| AuthError::ConfigError(format!("Invalid JWK: {}", e)));

                ret.map(DecodingKeyInternal::DecKey)
            }
            KeyFormat::Jwks => {
                // JWKS format
                let jwk_set: JwkSet = serde_json::from_str(&resolve_key(key))
                    .map_err(|e| AuthError::ConfigError(format!("Invalid JWKS: {}", e)))?;

                Ok(DecodingKeyInternal::Jwks(jwk_set))
            }
        }
    }

    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<VerifierJwt, AuthError> {
        // Set up validation
        let validation = self.build_validation();

        let verifier = VerifierJwt::new(self.build_claims(), self.token_duration, validation);

        // Autoresolver is enabled
        if self.auto_resolve_keys {
            return Ok(verifier.with_key_resolver(KeyResolver::new()));
        }

        // Create encoding key, either from file or from inline PEM
        let key = Self::build_internal(self.public_key.as_ref().unwrap())?;

        match key {
            DecodingKeyInternal::DecKey(key) => {
                // Transform it to Arc
                let decoding_key = Arc::new(RwLock::new(key));

                // Create a verifier
                let verifier = verifier.with_decoding_key(decoding_key.clone());

                // If the public key is a file, setup also a file watcher for it
                let verifier = match &self.public_key.as_ref().unwrap().key {
                    KeyData::File(path) => {
                        // If the key is a file, we need to set up a file watcher
                        let decoding_key_clone = decoding_key.clone();
                        let key_clone = self.public_key.clone().unwrap();
                        let mut w = FileWatcher::create_watcher(move |_file: &str| {
                            let new_key =
                                Self::build_internal(&key_clone).expect("error processing new key");
                            *decoding_key_clone.as_ref().write() = match new_key {
                                DecodingKeyInternal::DecKey(key) => key,
                                _ => panic!("Expected DecodingKey, got Jwks"),
                            };
                        });
                        w.add_file(path).expect("error adding file to the watcher");

                        verifier.with_watcher(w)
                    }
                    _ => verifier,
                };

                Ok(verifier)
            }
            DecodingKeyInternal::Jwks(jwk_set) => {
                // Create key resolver with the JWKS
                let resolver = KeyResolver::with_jwks(jwk_set);

                // Use JWKS for verification
                Ok(verifier.with_key_resolver(resolver))
            }
        }
    }
}

// Implementation for the WithToken state
impl JwtBuilder<state::WithToken> {
    /// Transition to the final state after setting required information.
    pub fn build(self) -> Result<StaticTokenProvider, AuthError> {
        // Setup file watcher
        let static_token = std::fs::read_to_string(self.token_file.as_ref().unwrap())
            .expect("error reading token file");
        let static_token = Arc::new(RwLock::new(static_token));

        let token_clone = static_token.clone();
        let mut w = FileWatcher::create_watcher(move |file: &str| {
            let token = std::fs::read_to_string(file).expect("error reading token file");
            *token_clone.as_ref().write() = token;
        });
        w.add_file(self.token_file.as_ref().unwrap())
            .map_err(|e| AuthError::ConfigError(e.to_string()))?;

        // Create new Jwt instance
        Ok(SignerJwt::new(
            self.build_claims(),               // not used
            std::time::Duration::from_secs(0), // not used
            self.build_validation(),           // not used
        )
        .with_static_token(static_token)
        .with_watcher(w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::TokenProvider;
    use crate::traits::{Signer, Verifier};
    use serde::{Deserialize, Serialize};
    use std::env;
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use slim_config::tls::provider::initialize_crypto_provider;

    fn create_file(file_path: &str, content: &str) -> std::io::Result<()> {
        let mut file = File::create(file_path)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    fn delete_file(file_path: &str) -> std::io::Result<()> {
        fs::remove_file(file_path)?;
        Ok(())
    }

    #[test]
    fn test_jwt_builder_basic() {
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = jwt.create_claims();

        assert_eq!(claims.iss.unwrap(), "test-issuer");
        assert_eq!(claims.aud.unwrap(), &["test-audience"]);
        assert_eq!(claims.sub.unwrap(), "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_builder_basic_key_from_file() {
        // crate file
        let path = env::current_dir().expect("error reading local path");
        let full_path = path.join("key_file_builder.txt");
        let file_name = full_path.to_str().unwrap();
        create_file(file_name, "tesk-key").expect("failed to create file");

        // create jwt builder
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::File(file_name.to_string()),
            })
            .build()
            .unwrap();

        let claims = jwt.create_claims();

        assert_eq!(claims.iss.unwrap(), "test-issuer");
        assert_eq!(claims.aud.unwrap(), &["test-audience"]);
        assert_eq!(claims.sub.unwrap(), "test-subject");

        delete_file(file_name).expect("error deleting file");
    }

    #[tokio::test]
    async fn test_jwt_builder_sign_verify() {
        // Using the explicit state machine
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();
        let verified: crate::traits::StandardClaims = verifier.get_claims(&token).await.unwrap();

        assert_eq!(verified.iss.unwrap(), "test-issuer");
        assert_eq!(verified.aud.unwrap(), &["test-audience"]);
        assert_eq!(verified.sub.unwrap(), "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_builder_custom_claims() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct CustomClaims {
            iss: String,
            aud: Vec<String>,
            sub: String,
            exp: u64,
            role: String,
        }

        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let custom_claims = CustomClaims {
            iss: "test-issuer".to_string(),
            aud: vec!["test-audience".to_string()],
            sub: "test-subject".to_string(),
            exp: now + 3600,
            role: "admin".to_string(),
        };

        let token = signer.sign(&custom_claims).unwrap();
        let verified: CustomClaims = verifier.get_claims(&token).await.unwrap();

        assert_eq!(verified, custom_claims);
    }

    #[test]
    fn test_jwt_builder_auto_resolve_keys() {
        // Set crypto provider
        initialize_crypto_provider();

        // Using state machine with direct transition
        let jwt = JwtBuilder::new()
            .issuer("https://example.com")
            .audience(&["test-audience"])
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build();
        assert!(jwt.is_ok());
    }

    #[tokio::test]
    async fn test_static_token_provider() {
        // Set crypto provider
        initialize_crypto_provider();

        let tokenvalue = "thecontent";

        create_file("/tmp/token", tokenvalue).unwrap();

        let provider = JwtBuilder::new()
            .issuer("https://example.com")
            .audience(&["test-audience"])
            .subject("test-subject")
            .token_file("/tmp/token")
            .build()
            .unwrap();

        // check the content of the token
        let token = provider.get_token().unwrap();
        assert_eq!(token, tokenvalue);

        // Modify the file
        let new_token_value = "thenewcontent";
        create_file("/tmp/token", new_token_value).unwrap();

        // let's wait 100ms for the modification to take place and the file
        // watcher to update the token
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Let's check the token again
        let token = provider.get_token().unwrap();
        assert_eq!(token, new_token_value);
    }

    #[tokio::test]
    async fn initialize_static_token_provider() {
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["aud"])
            .subject("sub")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-key".into()),
            })
            .build()
            .unwrap();
        let token = Arc::new(RwLock::new("header.payload.sig".to_string()));
        let mut static_provider: StaticTokenProvider = jwt.with_static_token(token);
        let _ = static_provider.initialize().await; // no-op
        let token = static_provider.get_token().unwrap(); // may not be a valid JWT; only presence matters here
        assert_eq!(token, "header.payload.sig");
    }
}

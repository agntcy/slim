// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
pub use jsonwebtoken_aws_lc::Algorithm;
use jsonwebtoken_aws_lc::{
    DecodingKey, EncodingKey, Header as JwtHeader, TokenData, Validation, decode, decode_header,
    encode, errors::ErrorKind,
};

use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;
use crate::file_watcher::FileWatcher;
use crate::resolver::KeyResolver;
use crate::traits::{Signer, StandardClaims, TokenProvider, Verifier};

/// Enum representing key data types
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KeyData {
    /// PEM encoded key
    Pem(String),
    /// File path to the key
    File(String),
}

/// Represents a key
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Key {
    /// Algorithm used for signing the JWT
    pub algorithm: Algorithm,

    /// PEM encoded key or file path
    #[serde(flatten, with = "serde_yaml::with::singleton_map")]
    pub key: KeyData,
}

/// Cache entry for validated tokens
#[derive(Debug, Clone)]
struct TokenCacheEntry {
    /// Expiration time in seconds since UNIX epoch
    expiry: u64,
}

/// Cache for validated tokens to avoid repeated signature verification
#[derive(Debug)]
struct TokenCache {
    /// Map from token string to cache entry
    entries: RwLock<HashMap<String, TokenCacheEntry>>,
}

impl TokenCache {
    /// Create a new token cache
    fn new() -> Self {
        TokenCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store a token and its claims in the cache
    fn store(&self, token: impl Into<String>, expiry: u64) {
        let entry = TokenCacheEntry { expiry };
        self.entries.write().insert(token.into(), entry);
    }

    /// Retrieve a token's claims from the cache if it exists and is still valid
    fn get(&self, token: impl Into<String>) -> Option<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let token = token.into();
        if let Some(entry) = self.entries.read().get(&token) {
            if entry.expiry > now {
                // Decode the claims part of the token
                let parts: Vec<&str> = token.split('.').collect();
                if parts.len() == 3 {
                    return Some(parts[1].to_string());
                }
            }
        }

        None
    }
}

pub type SignerJwt = Jwt<S>;
pub type VerifierJwt = Jwt<V>;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExpClaim {
    /// Expiration time in seconds since UNIX epoch
    exp: u64,
}

/// JWT implementation that uses the jsonwebtoken crate.
#[derive(Clone)]
pub struct Jwt<T> {
    claims: StandardClaims,
    token_duration: Duration,
    validation: Validation,
    encoding_key: Arc<RwLock<Option<EncodingKey>>>,
    decoding_key: Arc<RwLock<Option<DecodingKey>>>,
    key_resolver: std::sync::Arc<Option<KeyResolver>>,
    token_cache: std::sync::Arc<TokenCache>,
    watcher: std::sync::Arc<Option<FileWatcher>>,

    /// Static token from file. Unused for the moment.
    _token_file: Option<String>,

    _phantom: std::marker::PhantomData<T>,
}

impl<T> Jwt<T> {
    /// Internal constructor used by the builder.
    ///
    /// This should not be called directly. Use the builder pattern instead:
    /// ```
    /// let jwt = Jwt::builder()
    ///     .issuer("my-issuer")
    ///     .audience("my-audience")
    ///     .subject("user-123")
    ///     .private_key("secret-key")
    ///     .build()?;
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn new(claims: StandardClaims, token_duration: Duration, validation: Validation) -> Self {
        Self {
            claims,
            token_duration,
            validation,
            encoding_key: Arc::new(RwLock::new(None)),
            decoding_key: Arc::new(RwLock::new(None)),
            key_resolver: Arc::new(None),
            watcher: Arc::new(None),
            _token_file: None,
            token_cache: std::sync::Arc::new(TokenCache::new()),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_encoding_key(
        self,
        encoding_key: Arc<RwLock<Option<EncodingKey>>>,
        watcher: Option<FileWatcher>,
    ) -> SignerJwt {
        SignerJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key,
            decoding_key: Arc::new(RwLock::new(None)),
            key_resolver: Arc::new(None),
            watcher: Arc::new(watcher),
            _token_file: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_token_file(self, token_file: impl Into<String>) -> SignerJwt {
        SignerJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: Arc::new(RwLock::new(None)),
            decoding_key: Arc::new(RwLock::new(None)),
            key_resolver: Arc::new(None),
            watcher: self.watcher,
            _token_file: Some(token_file.into()),
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_decoding_key(
        self,
        decoding_key: Arc<RwLock<Option<DecodingKey>>>,
        watcher: Option<FileWatcher>,
    ) -> VerifierJwt {
        VerifierJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: Arc::new(RwLock::new(None)),
            decoding_key,
            key_resolver: Arc::new(None),
            watcher: Arc::new(watcher),
            _token_file: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_key_resolver(self, key_resolver: KeyResolver) -> VerifierJwt {
        VerifierJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: Arc::new(RwLock::new(None)),
            decoding_key: Arc::new(RwLock::new(None)),
            key_resolver: Arc::new(Some(key_resolver)),
            watcher: self.watcher,
            _token_file: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn delete(&self) {
        if let Some(w) = self.watcher.as_ref() {
            w.stop_watcher()
        }
    }
}

#[derive(Clone)]
pub struct S {}

impl<S> Jwt<S> {
    /// Creates a StandardClaims object with the default values.
    pub fn create_claims(&self) -> StandardClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let expiration = now + self.token_duration.as_secs();

        StandardClaims {
            exp: expiration,
            iat: Some(now),
            nbf: Some(now),
            ..self.claims.clone()
        }
    }

    fn sign_claims<Claims: Serialize>(&self, claims: &Claims) -> Result<String, AuthError> {
        // Ensure we have an encoding key for signing
        let key_lock = self.encoding_key.read();
        let encoding_key = key_lock.as_ref().ok_or_else(|| {
            AuthError::ConfigError("Private key not configured for signing".to_string())
        })?;

        // Create the JWT header
        let header = JwtHeader::new(self.validation.algorithms[0]);

        // Encode the claims into a JWT token
        encode(&header, claims, encoding_key).map_err(|e| AuthError::SigningError(format!("{}", e)))
    }

    fn sign_internal_claims(&self) -> Result<String, AuthError> {
        let claims = self.create_claims();
        self.sign_claims(&claims)
    }
}

#[derive(Clone)]
pub struct V {}

impl<V> Jwt<V> {
    fn try_verify_claims<Claims: serde::de::DeserializeOwned>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, AuthError> {
        // Convert the token into a String
        let token = token.into();

        // Check if the token is in the cache first for cacheable claim types
        if let Some(cached_claims) = self.get_cached_claims::<Claims>(&token) {
            return Ok(cached_claims);
        }

        // Try to decode the key from the cache first
        let decoding_key = self.decoding_key(&token)?;

        // If we have a decoding key, proceed with verification
        self.verify_internal::<Claims>(token, decoding_key)
    }

    async fn verify_claims<Claims: serde::de::DeserializeOwned>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, AuthError> {
        let token = token.into();

        // Check if the token is in the cache first for cacheable claim types
        if let Some(cached_claims) = self.get_cached_claims::<Claims>(&token) {
            return Ok(cached_claims);
        }

        // Resolve the decoding key for verification
        let decoding_key = self.resolve_decoding_key(&token).await?;

        self.verify_internal::<Claims>(token, decoding_key)
    }

    fn verify_internal<Claims: serde::de::DeserializeOwned>(
        &self,
        token: impl Into<String>,
        decoding_key: DecodingKey,
    ) -> Result<Claims, AuthError> {
        let token = token.into();

        // Get the token header
        let token_header = decode_header(&token).map_err(|e| {
            AuthError::TokenInvalid(format!("Failed to decode token header: {}", e))
        })?;

        // Derive a validation using the same algorithm
        let mut validation = self.get_validation(token_header.alg);

        // Decode and verify the token
        let token_data: TokenData<Claims> =
            decode(&token, &decoding_key, &validation).map_err(|e| match e.kind() {
                ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::VerificationError(format!("{}", e)),
            })?;

        // Get the exp to cache the token
        validation.insecure_disable_signature_validation();
        // Decode and verify the exp
        let token_exp_data: TokenData<ExpClaim> = decode(&token, &decoding_key, &validation)
            .map_err(|e| match e.kind() {
                ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::VerificationError(format!("{}", e)),
            })?;

        // Cache the token with its expiry
        self.cache(token, token_exp_data.claims.exp);

        // Parse the token to extract the expiry claim
        Ok(token_data.claims)
    }

    fn get_cached_claims<Claims: serde::de::DeserializeOwned>(
        &self,
        token: &str,
    ) -> Option<Claims> {
        // Check if the token is in the cache first for cacheable claim types
        if let Some(_cached_claims) = self.token_cache.get(token) {
            // Return the token skipping the signature verification
            let mut validation = self.get_validation(self.validation.algorithms[0]);
            validation.insecure_disable_signature_validation();

            let token_data: TokenData<Claims> =
                decode(token, &DecodingKey::from_secret(b"notused"), &validation)
                    .map_err(|e| AuthError::VerificationError(format!("{}", e)))
                    .ok()?;

            // Return the claims from the cached token
            return Some(token_data.claims);
        }

        None
    }

    fn cache(&self, token: impl Into<String>, expiry: u64) {
        // Store the token in the cache with its expiry
        self.token_cache.store(token, expiry);
    }

    fn get_validation(&self, alg: Algorithm) -> Validation {
        // Create a validation object with the configured issuer, audience, and subject
        let mut ret = self.validation.clone();
        ret.algorithms[0] = alg;

        ret
    }

    fn unsecure_get_token_data<T: DeserializeOwned>(
        &self,
        token: &str,
    ) -> Result<TokenData<T>, AuthError> {
        let mut validation = self.validation.clone();
        validation.insecure_disable_signature_validation();
        let decoding_key = DecodingKey::from_secret(b"unused");

        // Get issuer from claims
        decode(token, &decoding_key, &validation)
            .map_err(|e| AuthError::VerificationError(format!("Failed to decode token: {}", e)))
    }

    /// Get decoding key for verification
    fn decoding_key(&self, token: &str) -> Result<DecodingKey, AuthError> {
        // If the decoding key is available, return it
        {
            let key_lock = self.decoding_key.read();
            if let Some(key) = &*key_lock {
                return Ok(key.clone());
            }
        }

        // Try to get a cached decoding key
        if let Some(resolver) = &self.key_resolver.as_ref() {
            let mut validation = self.validation.clone();
            validation.insecure_disable_signature_validation();
            let decoding_key = DecodingKey::from_secret(b"unused");

            // Get issuer from claims
            let token_data: TokenData<StandardClaims> = decode(token, &decoding_key, &validation)
                .map_err(|e| {
                AuthError::VerificationError(format!("Failed to decode token: {}", e))
            })?;

            let issuer = token_data.claims.iss.as_ref().ok_or_else(|| {
                AuthError::ConfigError("no issuer found in JWT claims".to_string())
            })?;

            return resolver.get_cached_key(issuer, &token_data.header);
        }

        // If we don't have a decoding key and no resolver, we can't proceed
        Err(AuthError::ConfigError(
            "Decoding key not configured for JWT verification".to_string(),
        ))
    }

    /// Resolve a decoding key for token verification
    async fn resolve_decoding_key(&self, token: &str) -> Result<DecodingKey, AuthError> {
        // First check if we already have a decoding key
        {
            let key_lock = self.decoding_key.read();
            if let Some(key) = &*key_lock {
                return Ok(key.clone());
            }
        }

        // As we don't have a decoding key, we need to resolve it. The resolver
        // should be set, otherwise we can't proceed.
        let resolver = self
            .key_resolver
            .as_ref()
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("Key resolver not configured".to_string()))?;

        // Parse the token header to get the key ID and algorithm
        let token_data = self.unsecure_get_token_data::<StandardClaims>(token)?;

        let issuer =
            token_data.claims.iss.as_ref().ok_or_else(|| {
                AuthError::ConfigError("no issuer found in JWT claims".to_string())
            })?;

        // Resolve the key
        resolver.resolve_key(issuer, &token_data.header).await
    }
}

impl Signer for SignerJwt {
    fn sign<Claims>(&self, claims: &Claims) -> Result<String, AuthError>
    where
        Claims: Serialize,
    {
        self.sign_claims(claims)
    }

    fn sign_standard_claims(&self) -> Result<String, AuthError> {
        self.sign_internal_claims()
    }
}

#[async_trait]
impl TokenProvider for SignerJwt {
    fn try_get_token(&self) -> Result<String, AuthError> {
        self.sign_internal_claims()
    }

    async fn get_token(&self) -> Result<String, AuthError> {
        panic!("signerjwt has no support for async token retrieval")
    }
}

#[async_trait]
impl Verifier for VerifierJwt {
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.verify_claims(token).await
    }

    fn try_verify<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.try_verify_claims(token)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::{env, fs};

    use super::*;
    use jsonwebtoken_aws_lc::{Algorithm, Header};
    use tokio::time;
    use tracing_test::traced_test;

    use crate::builder::JwtBuilder;
    use crate::testutils::{initialize_crypto_provider, setup_test_jwt_resolver};

    fn create_file(file_path: &str, content: &str) -> std::io::Result<()> {
        let mut file = File::create(file_path)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    fn modify_file(file_path: &str, new_content: &str) -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).open(file_path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(new_content.as_bytes())?;
        Ok(())
    }

    fn delete_file(file_path: &str) -> std::io::Result<()> {
        fs::remove_file(file_path)?;
        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn test_jwt_singer_update_key_from_file() {
        // crate file
        let path = env::current_dir().expect("error reading local path");
        let full_path = path.join("key_file_signer.txt");
        let file_name = full_path.to_str().unwrap();
        let first_key = "test-key";
        create_file(file_name, first_key).expect("failed to create file");

        // create jwt builder
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::File(file_name.to_string()),
            })
            .build()
            .unwrap();

        let claims = jwt.create_claims();

        assert_eq!(claims.iss.unwrap(), "test-issuer");
        assert_eq!(claims.aud.unwrap(), "test-audience");
        assert_eq!(claims.sub.unwrap(), "test-subject");

        {
            let dec_key = jwt.decoding_key.read();
            assert!(dec_key.is_none());
        }
        {
            let expected = EncodingKey::from_secret(first_key.as_bytes());
            let enc_key = jwt.encoding_key.read();
            assert!(enc_key.is_some());
            let k = enc_key.clone().unwrap();

            #[derive(Debug, Serialize, Deserialize)]
            struct Claims {
                sub: String,
                company: String,
            }

            let my_claims = Claims {
                sub: "b@b.com".to_owned(),
                company: "ACME".to_owned(),
            };

            let token_1 = encode(&Header::default(), &my_claims, &expected).unwrap();
            let token_2 = encode(&Header::default(), &my_claims, &k).unwrap();
            assert_eq!(token_1, token_2);
        }

        let second_key = "another-test-key";
        modify_file(file_name, second_key).expect("failed to create file");
        time::sleep(Duration::from_millis(100)).await;

        {
            let dec_key = jwt.decoding_key.read();
            assert!(dec_key.is_none());
        }
        {
            let expected = EncodingKey::from_secret(second_key.as_bytes());
            let enc_key = jwt.encoding_key.read();
            assert!(enc_key.is_some());
            let k = enc_key.clone().unwrap();

            #[derive(Debug, Serialize, Deserialize)]
            struct Claims {
                sub: String,
                company: String,
            }

            let my_claims = Claims {
                sub: "b@b.com".to_owned(),
                company: "ACME".to_owned(),
            };

            let token_1 = encode(&Header::default(), &my_claims, &expected).unwrap();
            let token_2 = encode(&Header::default(), &my_claims, &k).unwrap();
            assert_eq!(token_1, token_2);
        }

        jwt.delete();

        delete_file(file_name).expect("error deleting file");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_jwt_verifier_update_key_from_file() {
        // crate file
        let path = env::current_dir().expect("error reading local path");
        let full_path = path.join("key_file_verifier.txt");
        let file_name = full_path.to_str().unwrap();
        let first_key = "test-key";
        create_file(file_name, first_key).expect("failed to create file");

        // create jwt builder
        let jwt = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::File(file_name.to_string()),
            })
            .build()
            .unwrap();

        {
            let dec_key = jwt.decoding_key.read();
            assert!(dec_key.is_some());
        }
        {
            let enc_key = jwt.encoding_key.read();
            assert!(enc_key.is_none());
        }

        // test the verifier with the first key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem(String::from(first_key)),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();

        let verified_claims: StandardClaims = jwt.verify(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), "test-audience");
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        let second_key = "another-test-key";
        modify_file(file_name, second_key).expect("failed to create file");
        time::sleep(Duration::from_millis(100)).await;

        {
            let dec_key = jwt.decoding_key.read();
            assert!(dec_key.is_some());
        }
        {
            let enc_key = jwt.encoding_key.read();
            assert!(enc_key.is_none());
        }

        // test the verifier with the second key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem(String::from(second_key)),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();

        let verified_claims: StandardClaims = jwt.verify(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), "test-audience");
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        jwt.delete();

        delete_file(file_name).expect("error deleting file");
    }

    #[tokio::test]
    async fn test_jwt_sign_and_verify() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign_claims(&claims).unwrap();

        let verified_claims: StandardClaims = verifier.verify(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), "test-audience");
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        // Try to verify with an invalid token
        let invalid_token = "invalid.token.string";
        let result: Result<StandardClaims, AuthError> =
            verifier.verify(invalid_token.to_string()).await;
        assert!(
            result.is_err(),
            "Expected verification to fail for invalid token"
        );

        // Create a verifier with the wrong key
        let wrong_verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("wrong-secret-key".to_string()),
            })
            .build()
            .unwrap();
        let wrong_result: Result<StandardClaims, AuthError> = wrong_verifier.verify(token).await;
        assert!(
            wrong_result.is_err(),
            "Expected verification to fail with wrong key"
        );
    }

    #[tokio::test]
    async fn test_jwt_sign_and_verify_custom_claims() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let mut custom_claims = HashMap::new();
        custom_claims.insert(
            "role".to_string(),
            serde_json::Value::String("admin".to_string()),
        );
        custom_claims.insert(
            "permissions".to_string(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("read".to_string()),
                serde_json::Value::String("write".to_string()),
            ]),
        );

        let mut claims = signer.create_claims();
        claims.custom_claims = custom_claims;
        let token = signer.sign_claims(&claims).unwrap();

        let verified_claims: StandardClaims = verifier.verify(token).await.unwrap();

        assert_eq!(verified_claims.custom_claims.get("role").unwrap(), "admin");
        assert_eq!(
            verified_claims.custom_claims.get("permissions").unwrap(),
            &serde_json::Value::Array(vec![
                serde_json::Value::String("read".to_string()),
                serde_json::Value::String("write".to_string())
            ])
        );
    }

    #[tokio::test]
    async fn test_validate_jwt_with_provided_key() {}

    async fn test_jwt_resolve_with_algorithm(algorithm: Algorithm) {
        initialize_crypto_provider();

        let (test_key, mock_server, _alg_str) = setup_test_jwt_resolver(algorithm).await;

        // Build the JWT with auto key resolution
        let signer = JwtBuilder::new()
            .issuer(mock_server.uri())
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm,
                key: KeyData::Pem(test_key),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer(mock_server.uri())
            .audience("test-audience")
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build()
            .unwrap();

        // Sign and verify the token
        let token = signer.sign_claims(&signer.create_claims()).unwrap();
        let claims: StandardClaims = verifier.verify(token).await.unwrap();

        // Validate the claims
        assert_eq!(claims.iss.unwrap(), mock_server.uri());
        assert_eq!(claims.aud.unwrap(), "test-audience");
        assert_eq!(claims.sub.unwrap(), "test-subject");
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs256() {
        test_jwt_resolve_with_algorithm(Algorithm::RS256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs384() {
        test_jwt_resolve_with_algorithm(Algorithm::RS384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs512() {
        test_jwt_resolve_with_algorithm(Algorithm::RS512).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps256() {
        // Set aws-lc as default crypto provider
        test_jwt_resolve_with_algorithm(Algorithm::PS256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps384() {
        test_jwt_resolve_with_algorithm(Algorithm::PS384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps512() {
        test_jwt_resolve_with_algorithm(Algorithm::PS512).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_es256() {
        test_jwt_resolve_with_algorithm(Algorithm::ES256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_es384() {
        test_jwt_resolve_with_algorithm(Algorithm::ES384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_eddsa() {
        test_jwt_resolve_with_algorithm(Algorithm::EdDSA).await;
    }

    #[tokio::test]
    async fn test_jwt_verify_caching() {
        // Create test JWT objects
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let mut verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                key: KeyData::Pem("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign_claims(&claims).unwrap();

        // First verification
        let first_result: StandardClaims = verifier.try_verify(token.clone()).unwrap();

        // Alter the decoding_key to simulate a situation where signature verification would fail
        // if attempted again. Since we're using the cache, it should still work.
        verifier.decoding_key = Arc::new(RwLock::new(None));

        // Second verification with the same token - should use the cache
        let second_result: StandardClaims = verifier.try_verify(token.clone()).unwrap();

        // Both results should be the same
        assert_eq!(first_result.iss, second_result.iss);
        assert_eq!(first_result.aud, second_result.aud);
        assert_eq!(first_result.sub, second_result.sub);
        assert_eq!(first_result.exp, second_result.exp);

        // Now create a different token
        let mut custom_claims = std::collections::HashMap::new();
        custom_claims.insert(
            "role".to_string(),
            serde_json::Value::String("admin".to_string()),
        );

        let mut claims2 = signer.create_claims();
        claims2.custom_claims = custom_claims;

        let token2 = signer.sign_claims(&claims2).unwrap();

        // Verify the new token - should fail because we removed the decoding_key
        let result = verifier.try_verify::<StandardClaims>(token2);
        assert!(
            result.is_err(),
            "Should have failed due to missing decoding key"
        );
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken_aws_lc::jwk::KeyAlgorithm;
pub use jsonwebtoken_aws_lc::{Algorithm, Validation};
use jsonwebtoken_aws_lc::{
    DecodingKey, EncodingKey, Header as JwtHeader, TokenData, decode, decode_header, encode,
    errors::ErrorKind, jwk::Jwk,
};

use parking_lot::RwLock;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;
use crate::file_watcher::FileWatcher;
use crate::metadata::MetadataMap;
use crate::resolver::KeyResolver;
use crate::traits::{Signer, StandardClaims, TokenProvider, Verifier};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyFormat {
    Pem,
    Jwk,
    Jwks,
}

/// Enum representing key data types
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyData {
    /// String with encoded key(s)
    Data(String),
    /// File path to the key(s)
    File(String),
}

/// Represents a key
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Key {
    /// Algorithm used for signing the JWT
    #[schemars(with = "AlgorithmRepr")]
    pub algorithm: Algorithm,

    /// Key format - PEM, JWK or JWKS
    pub format: KeyFormat,

    /// Encoded key or file path
    pub key: KeyData,
}

/// Local enum used only for JSON Schema generation of the `algorithm` field.
/// Remote schema representation of jsonwebtoken_aws_lc::Algorithm
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum AlgorithmRepr {
    HS256,
    HS384,
    HS512,
    ES256,
    ES384,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    EdDSA,
}

fn key_alg_to_algorithm(key: &KeyAlgorithm) -> Result<Algorithm, AuthError> {
    match key {
        KeyAlgorithm::ES256 => Ok(Algorithm::ES256),
        KeyAlgorithm::ES384 => Ok(Algorithm::ES384),
        KeyAlgorithm::HS256 => Ok(Algorithm::HS256),
        KeyAlgorithm::HS384 => Ok(Algorithm::HS384),
        KeyAlgorithm::HS512 => Ok(Algorithm::HS512),
        KeyAlgorithm::RS256 => Ok(Algorithm::RS256),
        KeyAlgorithm::RS384 => Ok(Algorithm::RS384),
        KeyAlgorithm::RS512 => Ok(Algorithm::RS512),
        KeyAlgorithm::PS256 => Ok(Algorithm::PS256),
        KeyAlgorithm::PS384 => Ok(Algorithm::PS384),
        KeyAlgorithm::PS512 => Ok(Algorithm::PS512),
        KeyAlgorithm::EdDSA => Ok(Algorithm::EdDSA),
        _ => Err(AuthError::ConfigError(format!(
            "Unsupported key algorithm: {:?}",
            key
        ))),
    }
}

pub fn algorithm_from_jwk(jwk: &str) -> Result<Algorithm, AuthError> {
    let jwk: Jwk = serde_json::from_str(jwk)
        .map_err(|e| AuthError::ConfigError(format!("Failed to parse JWK: {}", e)))?;

    tracing::info!(?jwk, "JWK parsed successfully");

    let alg = jwk
        .common
        .key_algorithm
        .ok_or_else(|| AuthError::ConfigError("JWK does not contain an algorithm".to_string()))?;

    key_alg_to_algorithm(&alg)
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
        if let Some(entry) = self.entries.read().get(&token)
            && entry.expiry > now
        {
            // Decode the claims part of the token
            let parts: Vec<&str> = token.split('.').collect();
            if parts.len() == 3 {
                return Some(parts[1].to_string());
            }
        }

        None
    }
}

pub type SignerJwt = Jwt<S>;
pub type VerifierJwt = Jwt<V>;
pub type StaticTokenProvider = Jwt<P>;

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
    encoding_key: Option<Arc<RwLock<EncodingKey>>>,
    decoding_key: Option<Arc<RwLock<DecodingKey>>>,
    key_resolver: Option<Arc<KeyResolver>>,
    token_cache: std::sync::Arc<TokenCache>,
    watchers: Arc<Vec<FileWatcher>>,

    /// Static token from file
    static_token: Option<Arc<RwLock<String>>>,

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
            encoding_key: None,
            decoding_key: None,
            key_resolver: None,
            watchers: Arc::new(Vec::new()),
            static_token: None,
            token_cache: std::sync::Arc::new(TokenCache::new()),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_watcher(mut self, w: FileWatcher) -> Self {
        // Clone the Arc, get a mutable reference to the Vec, and push to it
        let watchers =
            Arc::get_mut(&mut self.watchers).expect("Failed to get mutable reference to watchers");
        watchers.push(w);

        Self { ..self }
    }

    pub fn with_encoding_key(self, encoding_key: Arc<RwLock<EncodingKey>>) -> SignerJwt {
        SignerJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: Some(encoding_key),
            decoding_key: None,
            key_resolver: None,
            watchers: Arc::new(Vec::new()),
            static_token: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_decoding_key(self, decoding_key: Arc<RwLock<DecodingKey>>) -> VerifierJwt {
        VerifierJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: None,
            decoding_key: Some(decoding_key),
            key_resolver: None,
            watchers: Arc::new(Vec::new()),
            static_token: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_key_resolver(self, key_resolver: KeyResolver) -> VerifierJwt {
        VerifierJwt {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: None,
            decoding_key: None,
            key_resolver: Some(Arc::new(key_resolver)),
            watchers: Arc::new(Vec::new()),
            static_token: None,
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_static_token(self, token: Arc<RwLock<String>>) -> StaticTokenProvider {
        StaticTokenProvider {
            claims: self.claims,
            token_duration: self.token_duration,
            validation: self.validation,
            encoding_key: None,
            decoding_key: None,
            key_resolver: None,
            watchers: self.watchers,
            static_token: Some(token),
            token_cache: self.token_cache,
            _phantom: std::marker::PhantomData,
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

    /// Creates a StandardClaims object with custom claims merged in.
    pub fn create_claims_with_custom(&self, custom_claims: MetadataMap) -> StandardClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let expiration = now + self.token_duration.as_secs();

        let mut merged_claims = self.claims.custom_claims.clone();
        merged_claims.extend(custom_claims);

        StandardClaims {
            exp: expiration,
            iat: Some(now),
            nbf: Some(now),
            custom_claims: merged_claims,
            ..self.claims.clone()
        }
    }

    fn sign_claims<Claims: Serialize>(&self, claims: &Claims) -> Result<String, AuthError> {
        // Ensure we have an encoding key for signing

        let encoding_key = self.encoding_key.as_ref().ok_or_else(|| {
            AuthError::ConfigError("Private key not configured for signing".to_string())
        })?;

        // Create the JWT header
        let header = JwtHeader::new(self.validation.algorithms[0]);

        // Encode the claims into a JWT token
        encode(&header, claims, &encoding_key.read())
            .map_err(|e| AuthError::SigningError(format!("{}", e)))
    }

    fn sign_internal_claims(&self) -> Result<String, AuthError> {
        let claims = self.create_claims();
        self.sign_claims(&claims)
    }

    fn sign_internal_claims_with_custom(
        &self,
        custom_claims: MetadataMap,
    ) -> Result<String, AuthError> {
        let claims = self.create_claims_with_custom(custom_claims);
        self.sign_claims(&claims)
    }
}

#[derive(Clone)]
pub struct P {}
impl<P> Jwt<P> {
    /// Get token
    fn get_token(&self) -> Result<String, AuthError> {
        Ok(self
            .static_token
            .as_ref()
            .ok_or(AuthError::GetTokenError("No token available".to_string()))?
            .read()
            .clone())
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

        // Try to decode the key from the cache first; if this would require async, return WouldBlockOn
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
            if let Some(key) = &self.decoding_key {
                return Ok(key.read().clone());
            }
        }

        // Try to get a cached decoding key
        if let Some(resolver) = &self.key_resolver {
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

            match resolver.get_cached_key(issuer, &token_data.header) {
                Ok(k) => return Ok(k),
                Err(_e) => {
                    // No cached key yet; async resolution would be required.
                    return Err(AuthError::WouldBlockOn);
                }
            }
        }

        // If we don't have a decoder
        Err(AuthError::ConfigError(
            "no resolver available for JWT key resolution".to_string(),
        ))
    }

    /// Resolve a decoding key for token verification
    async fn resolve_decoding_key(&self, token: &str) -> Result<DecodingKey, AuthError> {
        // First check if we already have a decoding key
        {
            if let Some(key) = &self.decoding_key {
                return Ok(key.read().clone());
            }
        }

        // As we don't have a decoding key, we need to resolve it. The resolver
        // should be set, otherwise we can't proceed.
        let resolver = self
            .key_resolver
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
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // SignerJwt has no asynchronous initialization requirements.
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        self.sign_internal_claims()
    }

    async fn get_token_with_claims(&self, custom_claims: MetadataMap) -> Result<String, AuthError> {
        if custom_claims.is_empty() {
            self.sign_internal_claims()
        } else {
            self.sign_internal_claims_with_custom(custom_claims)
        }
    }

    fn get_id(&self) -> Result<String, AuthError> {
        self.claims
            .sub
            .clone()
            .ok_or(AuthError::TokenInvalid("missing subject claim".to_string()))
    }
}

#[async_trait]
impl TokenProvider for StaticTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // StaticTokenProvider exposes a statically loaded token; nothing async to perform.
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        self.static_token
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("Static token not configured".to_string()))
            .map(|token| token.read().clone())
    }

    fn get_id(&self) -> Result<String, AuthError> {
        let token = self.get_token()?;
        extract_sub_claim_unsafe(&token)
    }

    async fn get_token_with_claims(
        &self,
        _custom_claims: MetadataMap,
    ) -> Result<String, AuthError> {
        // This provider does not support custom claims in the token
        Err(AuthError::UnsupportedOperation(
            "StaticTokenProvider does not support custom claims".to_string(),
        ))
    }
}

#[async_trait]
impl Verifier for VerifierJwt {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(()) // no-op
    }

    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        // Just verify the token is valid, don't extract claims
        self.verify_claims::<StandardClaims>(token)
            .await
            .map(|_| ())
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        // Just verify the token is valid, don't extract claims
        self.try_verify_claims::<StandardClaims>(token).map(|_| ())
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.verify_claims(token).await
    }

    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.try_verify_claims(token)
    }
}

/// Helper function to extract the 'sub' claim from a JWT token without signature validation
pub(crate) fn extract_sub_claim_unsafe(token: &str) -> Result<String, AuthError> {
    let mut validation = Validation::default();
    validation.insecure_disable_signature_validation();

    // Decode the token without signature validation
    let token_data = decode::<serde_json::Value>(
        token,
        &DecodingKey::from_secret(&[]), // Empty key since we're not validating
        &validation,
    )
    .map_err(|e| AuthError::TokenInvalid(format!("Failed to decode token: {}", e)))?;

    // Extract the 'sub' claim
    token_data
        .claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AuthError::TokenInvalid("Missing 'sub' claim in token".to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::{env, fs, vec};

    use super::*;
    use jsonwebtoken_aws_lc::{Algorithm, Header};
    use tokio::time;
    use tracing_test::traced_test;

    use slim_config::tls::provider::initialize_crypto_provider;

    use crate::builder::JwtBuilder;
    use slim_testing::utils::setup_test_jwt_resolver;

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
        let mut jwt = JwtBuilder::new()
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

        let _ = jwt.initialize().await;
        let claims = jwt.create_claims();

        assert_eq!(claims.iss.unwrap(), "test-issuer");
        assert_eq!(claims.aud.unwrap(), ["test-audience"]);
        assert_eq!(claims.sub.unwrap(), "test-subject");

        assert!(jwt.decoding_key.is_none());
        {
            let expected = EncodingKey::from_secret(first_key.as_bytes());
            assert!(jwt.encoding_key.is_some());
            let k = jwt.encoding_key.as_ref().unwrap();

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
            let token_2 = encode(&Header::default(), &my_claims, &k.read()).unwrap();
            assert_eq!(token_1, token_2);
        }

        let second_key = "another-test-key";
        modify_file(file_name, second_key).expect("failed to create file");
        time::sleep(Duration::from_millis(100)).await;

        assert!(jwt.decoding_key.is_none());
        {
            let expected = EncodingKey::from_secret(second_key.as_bytes());
            assert!(jwt.encoding_key.is_some());
            let k = jwt.encoding_key.as_ref().unwrap();

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
            let token_2 = encode(&Header::default(), &my_claims, &k.read()).unwrap();
            assert_eq!(token_1, token_2);
        }

        delete_file(file_name).expect("error deleting file");
    }

    #[test]
    fn test_jwt_try_verify_would_block_on_missing_cached_key_valid_token() {
        // Build verifier with auto_resolve (no cached key yet)
        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build()
            .unwrap();
        // Use test utility to generate a syntactically valid unsigned token
        let token = slim_testing::utils::generate_test_token(
            "test-issuer",
            "test-audience",
            "test-subject",
        );

        let res = verifier.try_verify(&token);
        assert!(
            matches!(res, Err(AuthError::WouldBlockOn)),
            "Expected WouldBlockOn, got {:?}",
            res
        );
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
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::File(file_name.to_string()),
            })
            .build()
            .unwrap();

        assert!(jwt.decoding_key.is_some());
        assert!(jwt.encoding_key.is_none());

        // test the verifier with the first key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data(String::from(first_key)),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();

        let verified_claims: StandardClaims = jwt.get_claims(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), &["test-audience"]);
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        let second_key = "another-test-key";
        modify_file(file_name, second_key).expect("failed to create file");
        time::sleep(Duration::from_millis(100)).await;

        assert!(jwt.decoding_key.is_some());
        assert!(jwt.encoding_key.is_none());

        // test the verifier with the second key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data(String::from(second_key)),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();

        let verified_claims: StandardClaims = jwt.get_claims(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), &["test-audience"]);
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        delete_file(file_name).expect("error deleting file");
    }

    #[tokio::test]
    async fn test_jwt_sign_and_verify() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-key".to_string()),
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
                key: KeyData::Data("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign_claims(&claims).unwrap();

        let verified_claims: StandardClaims = verifier.get_claims(token.clone()).await.unwrap();

        assert_eq!(verified_claims.iss.unwrap(), "test-issuer");
        assert_eq!(verified_claims.aud.unwrap(), ["test-audience"]);
        assert_eq!(verified_claims.sub.unwrap(), "test-subject");

        // Try to verify with an invalid token
        let invalid_token = "invalid.token.string";
        let result: Result<StandardClaims, AuthError> =
            verifier.get_claims(invalid_token.to_string()).await;
        assert!(
            result.is_err(),
            "Expected verification to fail for invalid token"
        );

        // Create a verifier with the wrong key
        let wrong_verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("wrong-secret-key".to_string()),
            })
            .build()
            .unwrap();
        let wrong_result: Result<StandardClaims, AuthError> =
            wrong_verifier.get_claims(token).await;
        assert!(
            wrong_result.is_err(),
            "Expected verification to fail with wrong key"
        );
    }

    #[tokio::test]
    async fn test_jwt_sign_and_verify_custom_claims() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-key".to_string()),
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
                key: KeyData::Data("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let mut custom_claims = MetadataMap::new();
        custom_claims.insert("role".to_string(), "admin".to_string());
        custom_claims.insert(
            "permissions".to_string(),
            vec!["read".to_string(), "write".to_string()],
        );

        let mut claims = signer.create_claims();
        claims.custom_claims = custom_claims;
        let token = signer.sign_claims(&claims).unwrap();

        let verified_claims: StandardClaims = verifier.get_claims(token).await.unwrap();

        let role: String = verified_claims
            .custom_claims
            .get("role")
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(&role, "admin");

        let permissions: Vec<String> = verified_claims
            .custom_claims
            .get("permissions")
            .unwrap()
            .as_list()
            .unwrap()
            .iter()
            .map(|v| v.try_into().unwrap())
            .collect();
        assert_eq!(
            permissions,
            vec![
                serde_json::Value::String("read".to_string()),
                serde_json::Value::String("write".to_string())
            ]
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
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm,
                format: KeyFormat::Pem,
                key: KeyData::Data(test_key),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer(mock_server.uri())
            .audience(&["test-audience"])
            .subject("test-subject")
            .auto_resolve_keys(true)
            .build()
            .unwrap();

        // Sign and verify the token
        let token = signer.sign_claims(&signer.create_claims()).unwrap();
        let claims: StandardClaims = verifier.get_claims(token).await.unwrap();

        // Validate the claims
        assert_eq!(claims.iss.unwrap(), mock_server.uri());
        assert_eq!(claims.aud.unwrap(), vec!["test-audience"]);
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
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let mut verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS512,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-key".to_string()),
            })
            .build()
            .unwrap();

        let claims = signer.create_claims();
        let token = signer.sign_claims(&claims).unwrap();

        // First verification
        let first_result: StandardClaims = verifier.try_get_claims(token.clone()).unwrap();

        // Alter the decoding_key to simulate a situation where signature verification would fail
        // if attempted again. Since we're using the cache, it should still work.
        verifier.decoding_key = None;

        // Second verification with the same token - should use the cache
        let second_result: StandardClaims = verifier.try_get_claims(token.clone()).unwrap();

        // Both results should be the same
        assert_eq!(first_result.iss, second_result.iss);
        assert_eq!(first_result.aud, second_result.aud);
        assert_eq!(first_result.sub, second_result.sub);
        assert_eq!(first_result.exp, second_result.exp);

        // Now create a different token
        let mut custom_claims = MetadataMap::new();
        custom_claims.insert("role", "admin");

        let mut claims2 = signer.create_claims();
        claims2.custom_claims = custom_claims;

        let token2 = signer.sign_claims(&claims2).unwrap();

        // Verify the new token - should fail because we removed the decoding_key
        let result = verifier.try_get_claims::<StandardClaims>(token2);
        assert!(
            result.is_err(),
            "Should have failed due to missing decoding key"
        );
    }

    #[tokio::test]
    async fn test_signer_jwt_get_id() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-user-123")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret".to_string()),
            })
            .build()
            .unwrap();

        let id = signer.get_id().unwrap();
        assert_eq!(id, "test-user-123");
    }

    #[tokio::test]
    async fn test_signer_jwt_get_id_missing_sub() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            // No subject set
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret".to_string()),
            })
            .build()
            .unwrap();

        let result = signer.get_id();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing subject claim")
        );
    }

    #[tokio::test]
    async fn test_static_token_provider_get_id() {
        // Create a valid JWT token with sub claim
        let header = JwtHeader::new(Algorithm::HS256);
        let claims = serde_json::json!({
            "sub": "static-user-456",
            "exp": (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600)
        });
        let key = EncodingKey::from_secret(b"secret");
        let token = encode(&header, &claims, &key).unwrap();

        let provider: StaticTokenProvider = Jwt::<P>::new(
            StandardClaims::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
            ),
            Duration::from_secs(3600),
            Validation::default(),
        )
        .with_static_token(Arc::new(RwLock::new(token)));

        let id = provider.get_id().unwrap();
        assert_eq!(id, "static-user-456");
    }

    #[tokio::test]
    async fn test_static_token_provider_get_id_invalid_token() {
        let provider: StaticTokenProvider = Jwt::<P>::new(
            StandardClaims::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
            ),
            Duration::from_secs(3600),
            Validation::default(),
        )
        .with_static_token(Arc::new(RwLock::new("invalid.token.here".to_string())));

        let result = provider.get_id();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to decode token")
        );
    }

    #[tokio::test]
    async fn test_static_token_provider_get_id_missing_sub() {
        // Create a JWT token without sub claim
        let header = JwtHeader::new(Algorithm::HS256);
        let claims = serde_json::json!({
            "exp": (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600)
        });
        let key = EncodingKey::from_secret(b"secret");
        let token = encode(&header, &claims, &key).unwrap();

        let provider: StaticTokenProvider = Jwt::<P>::new(
            StandardClaims::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
            ),
            Duration::from_secs(3600),
            Validation::default(),
        )
        .with_static_token(Arc::new(RwLock::new(token)));

        let result = provider.get_id();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Missing 'sub' claim in token")
        );
    }

    #[tokio::test]
    async fn test_initialize_jwt_signer() {
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
        let mut signer: SignerJwt = jwt;
        let _ = signer.initialize().await; // no-op
        // Verify the signer is properly configured
        assert!(signer.encoding_key.is_some());
        assert!(signer.decoding_key.is_none());

        // Produce a token explicitly to verify signer works after initialize.
        let claims = signer.create_claims();
        assert_eq!(claims.iss.as_ref().unwrap(), "test-issuer");
        assert_eq!(claims.aud.as_ref().unwrap(), &["aud"]);
        assert_eq!(claims.sub.as_ref().unwrap(), "sub");
        assert!(claims.exp > 0);
        assert!(claims.iat.is_some());
        assert!(claims.nbf.is_some());
        let token = signer.sign(&claims).unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3); // JWT should have 3 parts
    }
}

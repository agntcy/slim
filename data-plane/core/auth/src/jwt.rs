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
use serde::{Deserialize, Serialize};

use crate::errors::AuthError;
use crate::resolver::KeyResolver;
use crate::traits::{Claimer, Signer, StandardClaims, Verifier};

/// Represents a key
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Key {
    /// Algorithm used for signing the JWT
    pub algorithm: Algorithm,

    /// Key used for signing or validating the JWT
    pub key: String,
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
    issuer: Option<String>,
    audience: Option<String>,
    subject: Option<String>,
    token_duration: Duration,
    validation: Validation,
    encoding_key: Option<EncodingKey>,
    decoding_key: Option<DecodingKey>,
    key_resolver: std::sync::Arc<Option<KeyResolver>>,
    token_cache: std::sync::Arc<TokenCache>,

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
    pub fn new(
        issuer: Option<String>,
        audience: Option<String>,
        subject: Option<String>,
        token_duration: Duration,
        validation: Validation,
        encoding_key: Option<EncodingKey>,
        decoding_key: Option<DecodingKey>,
        key_resolver: Option<KeyResolver>,
    ) -> Self {
        Self {
            issuer,
            audience,
            subject,
            token_duration,
            validation,
            encoding_key,
            decoding_key,
            key_resolver: Arc::new(key_resolver),
            token_cache: std::sync::Arc::new(TokenCache::new()),
            _phantom: std::marker::PhantomData,
        }
    }
}

#[derive(Clone)]
pub struct S {}

impl<S> Jwt<S> {
    /// Creates a StandardClaims object with the default values.
    pub fn create_standard_claims(
        &self,
        custom_claims: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> StandardClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let expiration = now + self.token_duration.as_secs();

        StandardClaims {
            iss: self.issuer.clone(),
            sub: self.subject.clone(),
            aud: self.audience.clone(),
            exp: expiration,
            iat: Some(now),
            jti: None,
            nbf: Some(now),
            custom_claims: custom_claims.unwrap_or_default(),
        }
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

    /// Get decoding key for verification
    fn decoding_key(&self, token: &str) -> Result<DecodingKey, AuthError> {
        // If the decoding key is available, return it
        if let Some(key) = &self.decoding_key {
            return Ok(key.clone());
        }

        // Try to get a cached decoding key
        if let Some(resolver) = &self.key_resolver.as_ref() {
            // Parse the token header to get the key ID and algorithm
            let token_header = decode_header(token).map_err(|e| {
                AuthError::TokenInvalid(format!("Failed to decode token header: {}", e))
            })?;

            // get the issuer
            let issuer = self.issuer.as_ref().ok_or_else(|| {
                AuthError::ConfigError("Issuer not configured for JWT verification".to_string())
            })?;

            return resolver.get_cached_key(issuer, &token_header);
        }

        // If we don't have a decoding key and no resolver, we can't proceed
        Err(AuthError::ConfigError(
            "Decoding key not configured for JWT verification".to_string(),
        ))
    }

    /// Resolve a decoding key for token verification
    async fn resolve_decoding_key(&self, token: &str) -> Result<DecodingKey, AuthError> {
        // First check if we already have a decoding key
        if let Some(key) = &self.decoding_key {
            return Ok(key.clone());
        }

        // As we don't have a decoding key, we need to resolve it. The resolver
        // should be set, otherwise we can't proceed.
        let resolver = self
            .key_resolver
            .as_ref()
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("Key resolver not configured".to_string()))?;

        // Parse the token header to get the key ID and algorithm
        let token_header = decode_header(token).map_err(|e| {
            AuthError::TokenInvalid(format!("Failed to decode token header: {}", e))
        })?;

        // Issuer should be set, if it's not we can't proceed.
        let issuer = self.issuer.as_ref().ok_or_else(|| {
            AuthError::ConfigError("Issuer not configured for JWT verification".to_string())
        })?;

        // Resolve the key
        resolver.resolve_key(issuer, &token_header).await
    }
}

impl<T> Claimer for Jwt<T> {
    fn create_standard_claims(
        &self,
        custom_claims: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> StandardClaims {
        self.create_standard_claims(custom_claims)
    }
}

impl Signer for SignerJwt {
    fn sign<Claims>(&self, claims: &Claims) -> Result<String, AuthError>
    where
        Claims: Serialize,
    {
        let encoding_key = self.encoding_key.as_ref().ok_or_else(|| {
            AuthError::ConfigError("Private key not configured for signing".to_string())
        })?;

        let header = JwtHeader::new(self.validation.algorithms[0]);

        encode(&header, claims, encoding_key).map_err(|e| AuthError::SigningError(format!("{}", e)))
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

    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken_aws_lc::Algorithm;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::builder::JwtBuilder;

    #[tokio::test]
    async fn test_jwt_sign_and_verify() {
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "secret-key")
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(Algorithm::HS512, "secret-key")
            .build()
            .unwrap();

        let claims = signer.create_standard_claims(None);
        let token = signer.sign(&claims).unwrap();

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
            .public_key(Algorithm::HS512, "wrong-key")
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
            .private_key(Algorithm::HS512, "secret-key")
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(Algorithm::HS512, "secret-key")
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

        let claims = signer.create_standard_claims(Some(custom_claims));
        let token = signer.sign(&claims).unwrap();

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

    async fn setup_test_jwt_resolver(algorithm: Algorithm) -> (String, MockServer, String) {
        // Set up the mock server for JWKS
        let mock_server = MockServer::start().await;

        // Get the algorithm name as a string and prepare the key
        let (test_key, jwks_key_json, alg_str) = match algorithm {
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512 => {
                // Create an RSA keypair for testing
                let keypair = openssl::rsa::Rsa::generate(2048).unwrap();

                // Get modulus and exponent
                let modulus = keypair.n();
                let exponent = keypair.e();

                // Create key ID and parameters
                let key_id = URL_SAFE_NO_PAD.encode(keypair.public_key_to_der().unwrap());
                let modulus_encoded = URL_SAFE_NO_PAD.encode(modulus.to_vec());
                let exponent_encoded = URL_SAFE_NO_PAD.encode(exponent.to_vec());

                let alg_str = match algorithm {
                    Algorithm::RS256 => "RS256",
                    Algorithm::RS384 => "RS384",
                    Algorithm::RS512 => "RS512",
                    Algorithm::PS256 => "PS256",
                    Algorithm::PS384 => "PS384",
                    Algorithm::PS512 => "PS512",
                    _ => unreachable!(),
                };

                let jwks_key = json!({
                    "kty": "RSA",
                    "alg": alg_str,
                    "use": "sig",
                    "kid": key_id,
                    "n": modulus_encoded,
                    "e": exponent_encoded,
                });

                (
                    String::from_utf8(keypair.private_key_to_pem().unwrap()),
                    jwks_key,
                    alg_str.to_string(),
                )
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                // Choose the right curve for the algorithm
                let nid = match algorithm {
                    Algorithm::ES256 => openssl::nid::Nid::X9_62_PRIME256V1, // P-256
                    Algorithm::ES384 => openssl::nid::Nid::SECP384R1,        // P-384
                    _ => unreachable!(),
                };

                // Create EC key with appropriate curve
                let ecgroup = openssl::ec::EcGroup::from_curve_name(nid).unwrap();
                let keypair = openssl::ec::EcKey::generate(&ecgroup).unwrap();

                // Get the EC public key in DER format
                let public_key_der = keypair.public_key_to_der().unwrap();
                let key_id = URL_SAFE_NO_PAD.encode(&public_key_der);

                // Extract x and y coordinates for JWKS using the BigNum API
                let ec_point = keypair.public_key();
                let mut bn_ctx = openssl::bn::BigNumContext::new().unwrap();
                let mut x = openssl::bn::BigNum::new().unwrap();
                let mut y = openssl::bn::BigNum::new().unwrap();

                // Get the affine coordinates
                ec_point
                    .affine_coordinates(&ecgroup, &mut x, &mut y, &mut bn_ctx)
                    .unwrap();

                // Convert coordinates to base64url
                let x_encoded = URL_SAFE_NO_PAD.encode(x.to_vec());
                let y_encoded = URL_SAFE_NO_PAD.encode(y.to_vec());

                // These variables are now defined above using the raw public key bytes

                let alg_str = match algorithm {
                    Algorithm::ES256 => "ES256",
                    Algorithm::ES384 => "ES384",
                    _ => unreachable!(),
                };

                let curve_name = match algorithm {
                    Algorithm::ES256 => "P-256",
                    Algorithm::ES384 => "P-384",
                    _ => unreachable!(),
                };

                let jwks_key = json!({
                    "kty": "EC",
                    "alg": alg_str,
                    "use": "sig",
                    "kid": key_id,
                    "crv": curve_name,
                    "x": x_encoded,
                    "y": y_encoded
                });

                // Convert keypair to PKey
                // NOTE(msardara): it seems ECDSA keys shohuld be in the PKCS#8 format,
                // otherwise jsonwebtoken will fail to decode them.
                let pkey =
                    openssl::pkey::PKey::<openssl::pkey::Private>::from_ec_key(keypair).unwrap();

                (
                    String::from_utf8(pkey.private_key_to_pem_pkcs8().unwrap()),
                    jwks_key,
                    alg_str.to_string(),
                )
            }
            Algorithm::EdDSA => {
                // Generate Ed25519 key
                let keypair = openssl::pkey::PKey::generate_ed25519().unwrap();

                // For EdDSA, let's use the public key in DER format
                let public_key_der = keypair.public_key_to_der().unwrap();
                let key_id = URL_SAFE_NO_PAD.encode(&public_key_der);

                // Extract the raw key bytes from DER format - the actual key is after the header
                // For simplicity, we'll use a fixed offset that works for Ed25519
                let x_encoded = URL_SAFE_NO_PAD.encode(&public_key_der[12..]);

                let jwks_key = json!({
                    "kty": "OKP",
                    "alg": "EdDSA",
                    "use": "sig",
                    "kid": key_id,
                    "crv": "Ed25519",
                    "x": x_encoded
                });

                (
                    String::from_utf8(keypair.private_key_to_pem_pkcs8().unwrap()),
                    jwks_key,
                    "EdDSA".to_string(),
                )
            }
            _ => panic!("Unsupported algorithm for this test: {:?}", algorithm),
        };

        // Setup mock for OpenID discovery endpoint
        let jwks_uri = format!("{}/custom/path/to/jwks.json", mock_server.uri());
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": mock_server.uri(),
                "jwks_uri": jwks_uri
            })))
            .mount(&mock_server)
            .await;

        // Setup mock for JWKS
        Mock::given(method("GET"))
            .and(path("/custom/path/to/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [jwks_key_json]
            })))
            .mount(&mock_server)
            .await;

        (test_key.unwrap(), mock_server, alg_str.to_string())
    }

    async fn test_jwt_resolve_with_algorithm(algorithm: Algorithm) {
        let (test_key, mock_server, _alg_str) = setup_test_jwt_resolver(algorithm).await;

        // Build the JWT with auto key resolution
        let signer = JwtBuilder::new()
            .issuer(mock_server.uri())
            .audience("test-audience")
            .subject("test-subject")
            .private_key(algorithm, test_key)
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
        let token = signer.sign(&signer.create_standard_claims(None)).unwrap();
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
            .private_key(Algorithm::HS512, "secret-key")
            .build()
            .unwrap();

        let mut verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .public_key(Algorithm::HS512, "secret-key")
            .build()
            .unwrap();

        let claims = signer.create_standard_claims(None);
        let token = signer.sign(&claims).unwrap();

        // First verification
        let first_result: StandardClaims = verifier.try_verify(token.clone()).unwrap();

        // Alter the decoding_key to simulate a situation where signature verification would fail
        // if attempted again. Since we're using the cache, it should still work.
        verifier.decoding_key = None;

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

        let claims2 = signer.create_standard_claims(Some(custom_claims));
        let token2 = signer.sign(&claims2).unwrap();

        // Verify the new token - should fail because we removed the decoding_key
        let result = verifier.try_verify::<StandardClaims>(token2);
        assert!(
            result.is_err(),
            "Should have failed due to missing decoding key"
        );
    }
}

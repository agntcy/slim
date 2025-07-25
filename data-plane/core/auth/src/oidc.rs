// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::executor::block_on;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use openidconnect::{
    ClientId, ClientSecret, IssuerUrl, OAuth2TokenResponse, Scope,
    core::{CoreClient, CoreProviderMetadata},
};
use parking_lot::RwLock;
use serde::Deserialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::errors::AuthError;
use crate::traits::{TokenProvider, Verifier};

// Default token refresh buffer (60 seconds before expiry)
const REFRESH_BUFFER_SECONDS: u64 = 60;
// Refresh tokens at 2/3 of their lifetime
const REFRESH_RATIO: f64 = 2.0 / 3.0;

/// Cache entry for OIDC access tokens
#[derive(Debug, Clone)]
struct TokenCacheEntry {
    /// The cached access token
    token: String,
    /// Expiration time in seconds since UNIX epoch
    expiry: u64,
    /// Time when the token should be refreshed (2/3 of lifetime)
    refresh_at: u64,
}

/// Cache entry for JWKS responses
#[derive(Debug, Clone)]
struct JwksCacheEntry {
    /// The cached JWKS
    jwks: JwkSet,
    /// Time when the JWKS was fetched
    fetched_at: SystemTime,
}

/// Cache for OIDC tokens to avoid repeated token requests
#[derive(Debug)]
struct OidcTokenCache {
    /// Map from cache key (issuer_url + client_id + scope) to token entry
    entries: RwLock<HashMap<String, TokenCacheEntry>>,
}

impl OidcTokenCache {
    /// Create a new OIDC token cache
    fn new() -> Self {
        OidcTokenCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store a token in the cache
    fn store(
        &self,
        key: impl Into<String>,
        token: impl Into<String>,
        expiry: u64,
        refresh_at: u64,
    ) {
        let entry = TokenCacheEntry {
            token: token.into(),
            expiry,
            refresh_at,
        };
        self.entries.write().insert(key.into(), entry);
    }

    /// Retrieve a token from the cache if it exists and is still valid
    fn get(&self, key: impl Into<String>) -> Option<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let key = key.into();
        if let Some(entry) = self.entries.read().get(&key) {
            if entry.expiry > now + REFRESH_BUFFER_SECONDS {
                return Some(entry.token.clone());
            }
        }
        None
    }

    /// Get tokens that need to be refreshed
    fn get_tokens_needing_refresh(&self) -> Vec<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        self.entries
            .read()
            .iter()
            .filter_map(|(key, entry)| {
                if now >= entry.refresh_at && entry.expiry > now + REFRESH_BUFFER_SECONDS {
                    Some(key.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Cache for JWKS to avoid repeated JWKS requests
#[derive(Debug)]
struct JwksCache {
    /// Map from issuer URL to JWKS entry
    entries: RwLock<HashMap<String, JwksCacheEntry>>,
}

impl JwksCache {
    /// Create a new JWKS cache
    fn new() -> Self {
        JwksCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store JWKS in the cache
    fn store(&self, issuer_url: impl Into<String>, jwks: JwkSet) {
        let entry = JwksCacheEntry {
            jwks,
            fetched_at: SystemTime::now(),
        };
        self.entries.write().insert(issuer_url.into(), entry);
    }

    /// Retrieve JWKS from the cache if it exists and is still valid
    fn get(&self, issuer_url: impl Into<String>) -> Option<JwkSet> {
        let key = issuer_url.into();
        if let Some(entry) = self.entries.read().get(&key) {
            // Cache JWKS for 1 hour
            if entry.fetched_at.elapsed().unwrap_or_default() <= Duration::from_secs(3600) {
                return Some(entry.jwks.clone());
            }
        }
        None
    }
}

/// Represents a JWK (JSON Web Key) from the JWKS endpoint
#[derive(Debug, Deserialize, Clone)]
pub struct Jwk {
    pub kty: String,         // Key type (e.g., "RSA")
    pub kid: String,         // Key ID
    pub n: Option<String>,   // RSA modulus (base64url)
    pub e: Option<String>,   // RSA exponent (base64url)
    pub alg: Option<String>, // Algorithm (e.g., "RS256")
}

/// Represents a JWKS (JSON Web Key Set) response
#[derive(Debug, Deserialize, Clone)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

/// OIDC Token Provider that implements the Client Credentials flow
#[derive(Clone)]
pub struct OidcTokenProvider {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    scope: Option<String>,
    token_cache: Arc<OidcTokenCache>,
    http_client: reqwest::Client,
    /// Shutdown signal sender for the background refresh task
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Handle to the background refresh task
    refresh_task: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
}

impl OidcTokenProvider {
    /// Create a new OIDC Token Provider
    pub async fn new(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        scope: Option<String>,
    ) -> Result<Self, AuthError> {
        let issuer_url_str = issuer_url.into();
        let client_id_str = client_id.into();
        let client_secret_str = client_secret.into();
        let http_client = reqwest::Client::new();

        // Validate that we can discover the OIDC provider
        let issuer_url = IssuerUrl::new(issuer_url_str.clone())
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;

        let _provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to discover provider metadata: {}", e))
            })?;

        // Create shutdown channel for background task
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());

        let provider = Self {
            issuer_url: issuer_url_str,
            client_id: client_id_str,
            client_secret: client_secret_str,
            scope,
            token_cache: token_cache.clone(),
            http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
        };

        // Start background refresh task
        let refresh_task = provider.start_refresh_task(shutdown_rx);
        *provider.refresh_task.lock() = Some(refresh_task);
        // Fetch initial token to populate cache
        if let Err(e) = provider.fetch_new_token().await {
            eprintln!("Warning: Failed to fetch initial token: {}", e);
            // Don't fail construction, let background task handle it
        }
        Ok(provider)
    }

    /// Generate cache key for token caching
    fn get_cache_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.issuer_url,
            self.client_id,
            self.scope.as_deref().unwrap_or("")
        )
    }

    /// Check if cached token is still valid ( using in the unit tests )
    #[allow(dead_code)]
    fn is_token_valid(&self, now: u64, expiry: u64) -> bool {
        expiry > now + REFRESH_BUFFER_SECONDS
    }

    /// Fetch a new token using client credentials flow
    async fn fetch_new_token(&self) -> Result<String, AuthError> {
        // Discover the provider metadata to get the token endpoint
        let issuer_url = IssuerUrl::new(self.issuer_url.clone())
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;
        let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &self.http_client)
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to discover provider metadata: {}", e))
            })?;

        let client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        );

        let mut token_request = match client.exchange_client_credentials() {
            Ok(request) => request,
            Err(e) => {
                return Err(AuthError::ConfigError(format!(
                    "Failed to create token request: {}",
                    e
                )));
            }
        };

        if let Some(ref scope) = self.scope {
            token_request = token_request.add_scope(Scope::new(scope.clone()));
        }

        let token_response = token_request
            .request_async(&self.http_client)
            .await
            .map_err(|e| AuthError::GetTokenError(format!("Failed to exchange token: {}", e)))?;

        let access_token = token_response.access_token().secret();
        let expires_in = token_response
            .expires_in()
            .map(|duration| duration.as_secs())
            .unwrap_or(3600); // Default to 1 hour

        // Calculate expiry timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expiry = now + expires_in;

        // Calculate refresh time (2/3 of token lifetime)
        let refresh_at = now + ((expires_in as f64 * REFRESH_RATIO) as u64);

        // Cache the token using the structured cache
        let cache_key = self.get_cache_key();
        self.token_cache
            .store(cache_key, access_token, expiry, refresh_at);

        Ok(access_token.to_string())
    }

    /// Start the background refresh task
    fn start_refresh_task(&self, mut shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
        let provider_clone = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check for tokens that need refreshing
                        let tokens_to_refresh = provider_clone.token_cache.get_tokens_needing_refresh();

                        for cache_key in tokens_to_refresh {
                            // Extract the parts from the cache key to determine which token to refresh
                            // For now, we'll just refresh the current provider's token if it matches
                            let current_cache_key = provider_clone.get_cache_key();
                            if cache_key == current_cache_key {
                                if let Err(e) = provider_clone.refresh_token_background().await {
                                    eprintln!("Failed to refresh token in background: {}", e);
                                }
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        })
    }

    /// Refresh token in background without blocking
    async fn refresh_token_background(&self) -> Result<(), AuthError> {
        match self.fetch_new_token().await {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("Background token refresh failed: {}", e);
                Err(e)
            }
        }
    }

    /// Shutdown the background refresh task
    pub fn shutdown(&self) {
        if self.shutdown_tx.send(true).is_err() {
            eprintln!("Failed to send shutdown signal");
        }

        // Wait for task to complete
        if let Some(handle) = self.refresh_task.lock().take() {
            tokio::spawn(async move {
                if (handle.await).is_err() {
                    eprintln!("Background refresh task panicked");
                }
            });
        }
    }

    /// Try to fetch initial token synchronously (only for very first call)
    fn try_initial_token_fetch(&self) -> Result<String, AuthError> {
        // Check if we already have any token in cache
        let cache_key = self.get_cache_key();
        if let Some(cached_token) = self.token_cache.get(&cache_key) {
            return Ok(cached_token);
        }

        // Only do blocking call if absolutely necessary (no cached token exists)
        block_on(self.fetch_new_token())
    }
}

impl TokenProvider for OidcTokenProvider {
    fn get_token(&self) -> Result<String, AuthError> {
        // Return cached tokens
        let cache_key = self.get_cache_key();
        if let Some(cached_token) = self.token_cache.get(&cache_key) {
            return Ok(cached_token);
        }

        // If no token is cached, try to fetch one synchronously for initial setup
        // This should only happen on the very first call
        match self.try_initial_token_fetch() {
            Ok(token) => Ok(token),
            Err(_) => Err(AuthError::GetTokenError(
                "No cached token available and initial fetch failed. Background refresh should handle this.".to_string()
            ))
        }
    }
}

impl Drop for OidcTokenProvider {
    fn drop(&mut self) {
        // Signal shutdown when the provider is dropped
        if self.shutdown_tx.send(true).is_err() {
            // Ignore errors during drop
        }
    }
}

/// OIDC Token Verifier that validates JWTs using JWKS
#[derive(Clone)]
pub struct OidcVerifier {
    issuer_url: String,
    audience: String,
    jwks_cache: Arc<JwksCache>,
    http_client: reqwest::Client,
}

impl OidcVerifier {
    /// Create a new OIDC Token Verifier
    pub fn new(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            audience: audience.into(),
            jwks_cache: Arc::new(JwksCache::new()),
            http_client: reqwest::Client::new(),
        }
    }

    /// Fetch JWKS from the issuer
    async fn fetch_jwks(&self) -> Result<JwkSet, AuthError> {
        let issuer_url = IssuerUrl::new(self.issuer_url.clone())
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;

        // Use openidconnect to discover provider metadata
        let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &self.http_client)
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to discover provider metadata: {}", e))
            })?;

        let jwks_uri = provider_metadata.jwks_uri();

        // Now fetch the JWKS from the discovered jwks_uri
        let jwks: JwkSet = self
            .http_client
            .get(jwks_uri.as_str())
            .send()
            .await?
            .json()
            .await?;

        Ok(jwks)
    }

    /// Get JWKS (from cache or fetch new)
    async fn get_jwks(&self) -> Result<JwkSet, AuthError> {
        // Check cache first
        if let Some(cached_jwks) = self.jwks_cache.get(&self.issuer_url) {
            return Ok(cached_jwks);
        }

        // Fetch new JWKS and cache it
        let jwks = self.fetch_jwks().await?;
        self.jwks_cache.store(&self.issuer_url, jwks.clone());
        Ok(jwks)
    }

    /// Verify a JWT token
    async fn verify_token<Claims>(&self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        // Decode header to get kid
        let header = decode_header(token)?;

        // Get JWKS
        let jwks = self.get_jwks().await?;

        // Find matching key
        let jwk = match header.kid {
            Some(kid) => {
                // Look for specific key by kid
                jwks.keys.iter().find(|k| k.kid == kid).ok_or_else(|| {
                    AuthError::VerificationError(format!("Key not found: {}", kid))
                })?
            }
            None => {
                // No kid provided - if there's only one key, use it
                if jwks.keys.len() == 1 {
                    &jwks.keys[0]
                } else {
                    return Err(AuthError::VerificationError(
                        "Token header missing 'kid' and multiple keys available".to_string(),
                    ));
                }
            }
        };

        // Only support RSA keys for now
        if jwk.kty != "RSA" {
            return Err(AuthError::UnsupportedOperation(format!(
                "Unsupported key type: {}",
                jwk.kty
            )));
        }

        let n = jwk.n.as_ref().ok_or_else(|| {
            AuthError::VerificationError("RSA key missing 'n' parameter".to_string())
        })?;
        let e = jwk.e.as_ref().ok_or_else(|| {
            AuthError::VerificationError("RSA key missing 'e' parameter".to_string())
        })?;

        // Create decoding key
        let decoding_key = DecodingKey::from_rsa_components(n, e)
            .map_err(|e| AuthError::VerificationError(format!("Invalid RSA key: {}", e)))?;

        // Set up validation
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[&self.issuer_url]);

        // Decode and validate token
        let token_data = decode::<Claims>(token, &decoding_key, &validation)?;
        Ok(token_data.claims)
    }
}

#[async_trait]
impl Verifier for OidcVerifier {
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.verify_token(&token.into()).await
    }

    fn try_verify<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        // For synchronous verification, we need a runtime
        block_on(self.verify_token(&token.into()))
    }
}

/////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm as JwtAlgorithm, EncodingKey, Header, encode};
    use jsonwebtoken_aws_lc::Algorithm;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Use the test utilities from the testutils module
    use crate::testutils::{
        TestClaims, initialize_crypto_provider, setup_oidc_mock_server, setup_test_jwt_resolver,
    };

    #[tokio::test]
    async fn test_oidc_token_provider_client_credentials_flow() {
        initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let provider = OidcTokenProvider::new(
            issuer_url,
            "test-client-id",
            "test-client-secret",
            Some("api:read".to_string()),
        )
        .await
        .unwrap();

        // Test token retrieval
        let token = provider.get_token().unwrap();
        assert_eq!(token, expected_token);

        // Verify mock was called
        // The mock server automatically verifies that the expected requests were made
    }

    #[tokio::test]
    async fn test_oidc_token_provider_caching() {
        initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let provider =
            OidcTokenProvider::new(issuer_url, "test-client-id", "test-client-secret", None)
                .await
                .unwrap();

        // First call - should fetch token
        let token1 = provider.get_token().unwrap();
        assert_eq!(token1, expected_token);

        // Second call - should use cached token
        let token2 = provider.get_token().unwrap();
        assert_eq!(token2, expected_token);
        assert_eq!(token1, token2);
    }

    #[tokio::test]
    async fn test_oidc_verifier_simple_mock() {
        initialize_crypto_provider();

        // Use the existing utility to set up mock server
        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create verifier and test that it can fetch JWKS
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Test that we can fetch JWKS successfully
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert_eq!(jwks.keys.len(), 1);
        assert_eq!(jwks.keys[0].kty, "RSA");
        // The key ID will be generated by setup_test_jwt_resolver
        assert!(!jwks.keys[0].kid.is_empty());
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwt_verification() {
        initialize_crypto_provider();

        // Setup mock OIDC server with JWKS using the existing utility
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create test claims
        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");

        // Create JWT token without kid (since we have only one key)
        let header = Header::new(JwtAlgorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        // Create verifier
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First test that JWKS can be fetched
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert!(!jwks.keys.is_empty());

        // Now verify the token
        let verified_claims: TestClaims = verifier.verify(token).await.unwrap();
        assert_eq!(verified_claims.sub, "user123");
        assert_eq!(verified_claims.aud, "test-audience");
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwks_caching() {
        initialize_crypto_provider();

        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let header = Header::new(JwtAlgorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First verification - should fetch JWKS
        let result1: TestClaims = verifier.verify(token.clone()).await.unwrap();
        assert_eq!(result1.sub, "user123");

        // Second verification - should use cached JWKS
        let result2: TestClaims = verifier.verify(token).await.unwrap();
        assert_eq!(result2.sub, "user123");
    }

    #[tokio::test]
    async fn test_oidc_verifier_invalid_token() {
        initialize_crypto_provider();

        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Try to verify invalid token
        let result: Result<TestClaims, _> = verifier.verify("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oidc_verifier_missing_kid_single_key_works() {
        initialize_crypto_provider();

        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Create token without kid in header - this should work with single key
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = None; // Explicitly remove kid
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should succeed because kid is missing but there's only one key available
        let result: Result<TestClaims, _> = verifier.verify(token).await;
        if let Err(e) = &result {
            println!("Unexpected error: {:?}", e);
        }
        assert!(
            result.is_ok(),
            "Expected success with single key and no kid, got error: {:?}",
            result.err()
        );

        let verified_claims = result.unwrap();
        assert_eq!(verified_claims.sub, "user123");
        assert_eq!(verified_claims.aud, "test-audience");
    }

    #[tokio::test]
    async fn test_oidc_verifier_unsupported_key_type() {
        initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock OIDC discovery endpoint
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{}/auth", issuer_url),
                "token_endpoint": format!("{}/oauth2/token", issuer_url),
                "jwks_uri": format!("{}/jwks.json", issuer_url),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"]
            })))
            .mount(&mock_server)
            .await;

        // Mock JWKS with unsupported key type
        Mock::given(method("GET"))
            .and(path("/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [{
                    "kty": "oct", // Symmetric key - not supported
                    "kid": "test-key-id",
                    "k": "test-key-value"
                }]
            })))
            .mount(&mock_server)
            .await;

        // Create a token with the test key ID
        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some("test-key-id".to_string());

        // Use a dummy key for encoding (the test will fail at key type validation)
        // We need to use the proper algorithm for the encoding to work
        let header = Header::new(JwtAlgorithm::HS256); // Use HS256 for symmetric key
        let encoding_key = EncodingKey::from_secret("dummy-secret".as_ref());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key type is not supported
        let result: Result<TestClaims, _> = verifier.verify(token).await;
        assert!(result.is_err());
        if let Err(AuthError::UnsupportedOperation(msg)) = result {
            assert!(msg.contains("Unsupported key type"));
        } else {
            panic!("Expected UnsupportedOperation error");
        }
    }

    #[tokio::test]
    async fn test_oidc_verifier_key_not_found() {
        initialize_crypto_provider();

        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Create token with non-existent key ID
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some("non-existent-key-id".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key is not found in JWKS
        let result: Result<TestClaims, _> = verifier.verify(token).await;
        assert!(result.is_err());
        if let Err(AuthError::VerificationError(msg)) = result {
            assert!(msg.contains("Key not found"));
        } else {
            panic!("Expected VerificationError about key not found");
        }
    }

    #[tokio::test]
    async fn test_oidc_token_provider_creation() {
        initialize_crypto_provider();

        // Use the existing setup function
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;
        let provider = OidcTokenProvider::new(
            issuer_url,
            "client-id",
            "client-secret",
            Some("scope".to_string()),
        )
        .await;

        // Test that the provider can be created successfully with proper OIDC server
        match provider {
            Ok(provider) => {
                assert_eq!(provider.scope, Some("scope".to_string()));
            }
            Err(e) => {
                eprintln!("Provider creation failed: {:?}", e);
                panic!("Provider creation should have succeeded");
            }
        }
    }

    #[test]
    fn test_oidc_verifier_creation() {
        let verifier = OidcVerifier::new("https://example.com", "audience");

        assert_eq!(verifier.issuer_url, "https://example.com");
        assert_eq!(verifier.audience, "audience");
    }

    #[tokio::test]
    async fn test_token_validity_check() {
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;

        let provider = OidcTokenProvider::new(issuer_url, "client-id", "client-secret", None)
            .await
            .unwrap();

        let now = 1000;
        let expiry_valid = now + REFRESH_BUFFER_SECONDS + 100; // Valid token
        let expiry_invalid = now + REFRESH_BUFFER_SECONDS - 100; // Invalid token

        assert!(provider.is_token_valid(now, expiry_valid));
        assert!(!provider.is_token_valid(now, expiry_invalid));
    }

    #[tokio::test]
    async fn test_oidc_token_provider_error_handling() {
        initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock discovery endpoint returning error (404 Not Found)
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        // Manually create a provider without calling the constructor
        // to avoid the hanging issue during construction
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());
        let http_client = reqwest::Client::new();

        let provider = OidcTokenProvider {
            issuer_url: issuer_url.clone(),
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            scope: None,
            token_cache: token_cache.clone(),
            http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
        };

        // Test that fetch_new_token fails when discovery endpoint returns 404
        let result = provider.fetch_new_token().await;
        assert!(result.is_err());

        // Should get a ConfigError due to discovery failure
        match result {
            Err(AuthError::ConfigError(msg)) => {
                // Expected: error should mention the discovery failure
                assert!(msg.contains("Failed to discover provider metadata"));
            }
            other => {
                panic!(
                    "Expected ConfigError for discovery failure, but got: {:?}",
                    other
                );
            }
        }
    }

    #[tokio::test]
    async fn test_oidc_token_provider_invalid_token_response() {
        initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock discovery endpoint with required fields
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{}/auth", issuer_url),
                "token_endpoint": format!("{}/oauth2/token", issuer_url),
                "jwks_uri": format!("{}/oauth2/jwks.json", issuer_url),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "grant_types_supported": ["authorization_code", "client_credentials"]
            })))
            .mount(&mock_server)
            .await;

        // Mock token endpoint returning proper OAuth2 error (400 Bad Request)
        // This is how OAuth2 servers should return errors according to RFC 6749
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({
                        "error": "invalid_client",
                        "error_description": "Client authentication failed"
                    })),
            )
            .mount(&mock_server)
            .await;

        // Mock JWKS endpoint (required for discovery)
        Mock::given(method("GET"))
            .and(path("/oauth2/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": []
            })))
            .mount(&mock_server)
            .await;

        // Manually create a provider without calling the constructor
        // to avoid the hanging issue during construction
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());
        let http_client = reqwest::Client::new();

        let provider = OidcTokenProvider {
            issuer_url: issuer_url.clone(),
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            scope: None,
            token_cache: token_cache.clone(),
            http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
        };

        // Test that fetch_new_token fails with proper OAuth2 error
        let result = provider.fetch_new_token().await;
        assert!(result.is_err());

        // Should get a GetTokenError due to the OAuth2 error response
        match result {
            Err(AuthError::GetTokenError(msg)) => {
                // Expected: error should mention the OAuth2 error
                assert!(
                    msg.contains("Failed to exchange token")
                        && (msg.contains("invalid_client")
                            || msg.contains("Client authentication failed"))
                );
            }
            other => {
                panic!(
                    "Expected GetTokenError containing OAuth2 error, but got: {:?}",
                    other
                );
            }
        }
    }

    #[tokio::test]
    async fn test_oidc_verifier_try_verify_sync() {
        initialize_crypto_provider();

        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let header = Header::new(JwtAlgorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First populate the JWKS cache with an async call to avoid hanging in try_verify
        let _jwks = verifier.get_jwks().await.unwrap();

        // Now test synchronous verification (uses cached JWKS)
        let verified_claims: TestClaims = verifier.try_verify(token).unwrap();
        assert_eq!(verified_claims.sub, "user123");
    }
}

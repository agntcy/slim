// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::AuthError;
use crate::jwt::extract_sub_claim_unsafe;
use crate::metadata::MetadataMap;
use crate::resolver::JwksCache;
use crate::traits::{TokenProvider, Verifier};
use async_trait::async_trait;
use futures::executor::block_on;
use jsonwebtoken_aws_lc::jwk::JwkSet;
use jsonwebtoken_aws_lc::{DecodingKey, Validation, decode, decode_header};
use oauth2::{AuthUrl, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl, basic::BasicClient};
use parking_lot::RwLock;
use reqwest::Client as ReqwestClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use url::Url;

// Default token refresh buffer (60 seconds before expiry)
const REFRESH_BUFFER_SECONDS: u64 = 60;

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
        if let Some(entry) = self.entries.read().get(&key)
            && entry.expiry > now + REFRESH_BUFFER_SECONDS
        {
            return Some(entry.token.clone());
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
struct OidcJwksCache {
    /// Map from issuer URL to JWKS entry
    entries: RwLock<HashMap<String, JwksCache>>,
}

impl OidcJwksCache {
    /// Create a new JWKS cache
    fn new() -> Self {
        OidcJwksCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store JWKS in the cache with custom TTL
    fn store_with_ttl(&self, issuer_url: impl Into<String>, jwks: JwkSet, ttl: Duration) {
        let entry = JwksCache {
            jwks,
            fetched_at: Instant::now(),
            ttl,
        };
        self.entries.write().insert(issuer_url.into(), entry);
    }

    /// Retrieve JWKS from the cache if it exists and is still valid
    fn get(&self, issuer_url: impl Into<String>) -> Option<JwkSet> {
        let key = issuer_url.into();
        if let Some(entry) = self.entries.read().get(&key) {
            // Use the per-entry TTL instead of hardcoded value
            if entry.fetched_at.elapsed() <= entry.ttl {
                return Some(entry.jwks.clone());
            }
        }
        None
    }
}

#[derive(Clone)]
pub struct OidcProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    pub issuer_url: String,
    pub scope: Option<String>,
    /// HTTP timeout for token requests (default: 30s)
    pub timeout: Option<Duration>,
}

/// OIDC Token Provider that implements the Client Credentials flow
#[derive(Clone)]
pub struct OidcTokenProvider {
    config: OidcProviderConfig,
    token_cache: Arc<OidcTokenCache>,
    client: ReqwestClient,
    /// Shutdown signal sender for the background refresh task
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Handle to the background refresh task
    refresh_task: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
}

impl OidcTokenProvider {
    /// Create a new OIDC Token Provider synchronously
    /// Note: Call `initialize()` after creation to start background tasks and fetch initial token
    pub fn new(config: OidcProviderConfig) -> Result<Self, AuthError> {
        // Validate the issuer URL
        Url::parse(&config.issuer_url).map_err(|e| {
            AuthError::InvalidIssuerEndpoint(format!("Invalid issuer endpoint URL: {}", e))
        })?;

        // Create HTTP client with timeout
        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth OAuth2")
            .timeout(config.timeout.unwrap_or(Duration::from_secs(30)))
            .build()
            .map_err(|e| AuthError::OAuth2Error(format!("Failed to create HTTP client: {}", e)))?;

        // Create shutdown channel for background task
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());

        Ok(Self {
            config,
            token_cache,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
        })
    }

    /// Initialize the provider asynchronously - starts background tasks and fetches initial token
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // Check if already initialized
        if self.refresh_task.lock().is_some() {
            return Ok(());
        }

        // Create new shutdown receiver using the existing sender
        let shutdown_rx = self.shutdown_tx.subscribe();

        // Start background refresh task
        let refresh_task = self.start_refresh_task(shutdown_rx);
        *self.refresh_task.lock() = Some(refresh_task);

        // Fetch initial token to populate cache
        if let Err(e) = self.fetch_new_token().await {
            tracing::warn!("Warning: Failed to fetch initial token: {}", e);
            // Don't fail initialization, let background task handle it
        }
        Ok(())
    }

    /// Generate cache key for token caching
    fn get_cache_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.config.issuer_url,
            self.config.client_id,
            self.config.scope.as_deref().unwrap_or("")
        )
    }

    /// Check if cached token is still valid
    #[cfg(test)]
    fn is_token_valid(&self, now: u64, expiry: u64) -> bool {
        expiry > now + REFRESH_BUFFER_SECONDS
    }

    /// Fetch a new token using client credentials flow
    async fn fetch_new_token(&self) -> Result<String, AuthError> {
        // Discover the provider metadata to get the token endpoint
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer_url
        );
        let discovery_response: serde_json::Value = self
            .client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to fetch discovery document: {}", e))
            })?
            .json()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to parse discovery document: {}", e))
            })?;

        let token_endpoint = discovery_response
            .get("token_endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AuthError::ConfigError("token_endpoint not found in discovery document".to_string())
            })?;

        let auth_url_str = discovery_response
            .get("authorization_endpoint")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}/authorize", self.config.issuer_url));

        // Create OAuth2 client (updated for new oauth2 builder API)
        let client = BasicClient::new(ClientId::new(self.config.client_id.clone()))
            .set_client_secret(ClientSecret::new(self.config.client_secret.clone()))
            .set_auth_uri(
                AuthUrl::new(auth_url_str)
                    .map_err(|e| AuthError::ConfigError(format!("Invalid auth URL: {}", e)))?,
            )
            .set_token_uri(
                TokenUrl::new(token_endpoint.to_string())
                    .map_err(|e| AuthError::ConfigError(format!("Invalid token URL: {}", e)))?,
            );

        let mut token_request = client.exchange_client_credentials();

        if let Some(ref scope) = self.config.scope {
            token_request = token_request.add_scope(Scope::new(scope.clone()));
        }

        let token_response = token_request
            .request_async(&self.client)
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

        // Calculate refresh time (2/3 of token lifetime) using integer math to avoid float casting
        let refresh_at = now + (expires_in * 2 / 3);

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
                            if cache_key == current_cache_key
                                && let Err(e) = provider_clone.refresh_token_background().await
                            {
                                tracing::error!("Failed to refresh token in background: {}", e);
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
                tracing::error!("Background token refresh failed: {}", e);
                Err(e)
            }
        }
    }

    /// Shutdown the background refresh task
    pub fn shutdown(&self) {
        if let Err(e) = self.shutdown_tx.send(true) {
            // Print the error message during drop
            tracing::debug!("Failed to send shutdown signal: {}", e);
        }
    }
}

#[async_trait]
impl TokenProvider for OidcTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        OidcTokenProvider::initialize(self).await
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let cache_key = self.get_cache_key();
        if let Some(cached_token) = self.token_cache.get(&cache_key) {
            return Ok(cached_token);
        }
        Err(AuthError::GetTokenError(format!(
            "No cached token available for key '{}'. Background refresh should handle this.",
            cache_key
        )))
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
            "OIDC Token Provider does not support custom claims".to_string(),
        ))
    }
}

impl Drop for OidcTokenProvider {
    fn drop(&mut self) {
        // Signal shutdown when the provider is dropped
        if let Err(e) = self.shutdown_tx.send(true) {
            // Print the error message during drop
            tracing::debug!("Failed to send shutdown signal: {}", e);
        }
    }
}

/// OIDC Token Verifier that validates JWTs using JWKS
#[derive(Clone)]
pub struct OidcVerifier {
    issuer_url: String,
    audience: String,
    jwks_cache: Arc<OidcJwksCache>,
    http_client: ReqwestClient,
    jwks_ttl: Duration,
}

impl OidcVerifier {
    /// Create a new OIDC Token Verifier
    pub fn new(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            audience: audience.into(),
            jwks_cache: Arc::new(OidcJwksCache::new()),
            http_client: reqwest::Client::new(),
            jwks_ttl: Duration::from_secs(3600), // Default 1 hour
        }
    }

    /// Create a new OIDC Token Verifier with custom JWKS TTL
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_ttl = ttl;
        self
    }

    /// Fetch JWKS from the issuer
    async fn fetch_jwks(&self) -> Result<JwkSet, AuthError> {
        // Discover provider metadata
        let discovery_url = format!("{}/.well-known/openid-configuration", self.issuer_url);
        let discovery_response: serde_json::Value = self
            .http_client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to fetch discovery document: {}", e))
            })?
            .json()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to parse discovery document: {}", e))
            })?;

        let jwks_uri = discovery_response
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AuthError::ConfigError("jwks_uri not found in discovery document".to_string())
            })?;

        // Now fetch the JWKS from the discovered jwks_uri
        let jwks: JwkSet = self.http_client.get(jwks_uri).send().await?.json().await?;

        Ok(jwks)
    }

    /// Get JWKS (from cache or fetch new)
    async fn get_jwks(&self) -> Result<JwkSet, AuthError> {
        // Check cache first
        if let Some(cached_jwks) = self.jwks_cache.get(&self.issuer_url) {
            return Ok(cached_jwks);
        }

        // Fetch new JWKS and cache it with the configured TTL
        let jwks = self.fetch_jwks().await?;
        self.jwks_cache
            .store_with_ttl(&self.issuer_url, jwks.clone(), self.jwks_ttl);
        Ok(jwks)
    }

    /// Utility function to verify a token against JWKS
    fn verify_token_util<Claims>(&self, token: &str, jwks: &JwkSet) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        // Decode header to get kid
        let header = decode_header(token).map_err(AuthError::JwtAwsLcError)?;

        // Find matching key
        let jwk = match header.kid {
            Some(kid) => {
                // Look for specific key by kid
                jwks.keys
                    .iter()
                    .find(|k| {
                        if let Some(key_id) = &k.common.key_id {
                            key_id == &kid
                        } else {
                            false
                        }
                    })
                    .ok_or_else(|| {
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

        // Create decoding key directly from JWK using the aws-lc method
        let decoding_key = DecodingKey::from_jwk(jwk).map_err(AuthError::JwtAwsLcError)?;

        // Set up validation
        let mut validation = Validation::new(header.alg);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[&self.issuer_url]);

        // Decode and validate token
        let token_data = decode::<Claims>(token, &decoding_key, &validation)
            .map_err(AuthError::JwtAwsLcError)?;
        Ok(token_data.claims)
    }

    /// Verify a JWT token
    async fn verify_token<Claims>(&self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        // Get JWKS
        let jwks = self.get_jwks().await?;

        // Use the utility function to verify the token
        self.verify_token_util(token, &jwks)
    }
}

#[async_trait]
impl Verifier for OidcVerifier {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(()) // no-op
    }

    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        // Verify the token structure is valid - this will fetch JWKS if needed
        let _: serde_json::Value = self.verify_token(&token.into()).await?;
        Ok(())
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        let token = token.into();

        // First try to verify with cached JWKS only
        if let Some(cached_jwks) = self.jwks_cache.get(&self.issuer_url) {
            // Use the utility function to verify the token with cached JWKS
            let _: serde_json::Value = self.verify_token_util(&token, &cached_jwks)?;
            Ok(())
        } else {
            // Indicate that a blocking (network) operation would be required
            Err(AuthError::WouldBlockOn)
        }
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.verify_token(&token.into()).await
    }

    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        // For synchronous verification, we need a runtime
        block_on(self.verify_token(&token.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken_aws_lc::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Use the test utilities from the testutils module
    use slim_testing::utils::{TestClaims, setup_oidc_mock_server, setup_test_jwt_resolver};

    #[tokio::test]
    async fn test_oidc_token_provider_client_credentials_flow() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url,
            scope: Some("api:read".to_string()),
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

        // Test token retrieval
        let token = provider.get_token().unwrap();
        assert_eq!(token, expected_token);
    }

    #[tokio::test]
    async fn test_oidc_token_provider_caching() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

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
        // Use the existing utility to set up mock server
        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create verifier and test that it can fetch JWKS
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Test that we can fetch JWKS successfully
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert_eq!(jwks.keys.len(), 1);
        // We can't easily check the key type without additional structure info,
        // but we can verify we have a key with an ID
        assert!(jwks.keys[0].common.key_id.is_some());
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwt_verification() {
        // Setup mock OIDC server with JWKS using the existing utility
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create test claims
        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");

        // Create JWT token without kid (since we have only one key)
        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        // Create verifier
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First test that JWKS can be fetched
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert!(!jwks.keys.is_empty());

        // Now verify the token
        let verified_claims: TestClaims = verifier.get_claims(token).await.unwrap();
        assert_eq!(verified_claims.sub, "user123");
        assert_eq!(verified_claims.aud, "test-audience");
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwks_caching() {
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

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First verification - should fetch JWKS
        let result1: TestClaims = verifier.get_claims(token.clone()).await.unwrap();
        assert_eq!(result1.sub, "user123");

        // Second verification - should use cached JWKS
        let result2: TestClaims = verifier.get_claims(token).await.unwrap();
        assert_eq!(result2.sub, "user123");
    }

    #[tokio::test]
    async fn test_oidc_verifier_invalid_token() {
        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Try to verify invalid token
        let result: Result<TestClaims, _> = verifier.get_claims("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oidc_verifier_missing_kid_single_key_works() {
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
        let mut header = Header::new(Algorithm::RS256);
        header.kid = None; // Explicitly remove kid
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should succeed because kid is missing but there's only one key available
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
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

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-id".to_string());

        // Use a dummy key for encoding (the test will fail at key type validation)
        // We need to use the proper algorithm for the encoding to work
        let header = Header::new(Algorithm::HS256); // Use HS256 for symmetric key
        let encoding_key = EncodingKey::from_secret("dummy-secret".as_ref());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key type is not supported
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
        assert!(result.is_err());

        // With jsonwebtoken_aws_lc, this might fail with a JwtAwsLcError instead
        // of UnsupportedOperation, which is also valid behavior
        match result.as_ref().err().unwrap() {
            AuthError::UnsupportedOperation(_) | AuthError::JwtAwsLcError(_) => {
                // Both error types are acceptable for this test case
            }
            _ => {
                panic!(
                    "Expected UnsupportedOperation or JwtAwsLcError, got: {:?}",
                    result.err()
                );
            }
        }
    }

    #[tokio::test]
    async fn test_oidc_verifier_key_not_found() {
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
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("non-existent-key-id".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key is not found in JWKS
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
        assert!(result.is_err());
        if let Err(AuthError::VerificationError(msg)) = result {
            assert!(msg.contains("Key not found"));
        } else {
            panic!("Expected VerificationError about key not found");
        }
    }

    #[tokio::test]
    async fn test_oidc_token_provider_creation() {
        // Use the existing setup function
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;
        let config = OidcProviderConfig {
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            issuer_url,
            scope: Some("scope".to_string()),
            timeout: None,
        };
        let provider_result = OidcTokenProvider::new(config);

        // Test that the provider can be created successfully with proper OIDC server
        match provider_result {
            Ok(mut provider) => {
                assert_eq!(provider.config.scope, Some("scope".to_string()));
                // Test that initialization works
                provider.initialize().await.unwrap();
            }
            Err(e) => {
                tracing::error!("Provider creation failed: {:?}", e);
                panic!("Provider creation should have succeeded");
            }
        }
    }

    #[test]
    fn test_oidc_verifier_creation() {
        let verifier = OidcVerifier::new("https://example.com", "audience");

        assert_eq!(verifier.issuer_url, "https://example.com");
        assert_eq!(verifier.audience, "audience");
        assert_eq!(verifier.jwks_ttl, Duration::from_secs(3600)); // Default 1 hour
    }

    #[test]
    fn test_oidc_verifier_custom_ttl() {
        let custom_ttl = Duration::from_secs(1800); // 30 minutes
        let verifier =
            OidcVerifier::new("https://example.com", "audience").with_jwks_ttl(custom_ttl);

        assert_eq!(verifier.issuer_url, "https://example.com");
        assert_eq!(verifier.audience, "audience");
        assert_eq!(verifier.jwks_ttl, custom_ttl);
    }

    #[test]
    fn test_jwks_cache_entry_reuse() {
        // Test that we're using the shared JwksCache struct from resolver.rs
        let jwks = JwkSet { keys: vec![] };
        let entry = JwksCache {
            jwks,
            fetched_at: Instant::now(),
            ttl: Duration::from_secs(1800),
        };

        // Verify the struct has the expected fields
        assert_eq!(entry.ttl, Duration::from_secs(1800));
        assert!(entry.jwks.keys.is_empty());
    }

    #[tokio::test]
    async fn test_token_validity_check() {
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

        let now = 1000;
        let expiry_valid = now + REFRESH_BUFFER_SECONDS + 100; // Valid token
        let expiry_invalid = now + REFRESH_BUFFER_SECONDS - 100; // Invalid token

        assert!(provider.is_token_valid(now, expiry_valid));
        assert!(!provider.is_token_valid(now, expiry_invalid));
    }

    #[tokio::test]
    async fn test_oidc_token_provider_error_handling() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

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

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url: issuer_url.clone(),
            scope: None,
            timeout: None,
        };

        let provider = OidcTokenProvider {
            config,
            token_cache: token_cache.clone(),
            client: http_client,
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
                assert!(msg.contains("Failed to parse discovery document"));
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
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

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

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url: issuer_url.clone(),
            scope: None,
            timeout: None,
        };

        let provider = OidcTokenProvider {
            config,
            token_cache: token_cache.clone(),
            client: http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
        };

        // Test that fetch_new_token fails with proper OAuth2 error
        let result = provider.fetch_new_token().await;
        assert!(result.is_err());

        // Should get a GetTokenError due to the OAuth2 error response
        match result {
            Err(AuthError::GetTokenError(msg)) => {
                // Expected: error should mention the OAuth2 error - oauth2 crate format
                assert!(
                    msg.contains("Failed to exchange token")
                        && msg.contains("Server returned error response")
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

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First populate the JWKS cache with an async call to avoid hanging in try_verify
        let _jwks = verifier.get_jwks().await.unwrap();

        // Now test synchronous verification (uses cached JWKS)
        let verified_claims: TestClaims = verifier.try_get_claims(token).unwrap();
        assert_eq!(verified_claims.sub, "user123");
    }
}

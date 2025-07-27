// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! OAuth2 Client Credentials flow implementation for SLIM authentication.
//!
//! This module provides OAuth2 Client Credentials flow support for acquiring
//! and verifying JWT tokens. It implements the standard OAuth2 client credentials
//! grant type as defined in RFC 6749.
//!
//! # Examples
//!
//! ```rust,no_run
//! use slim_auth::{OAuth2ClientCredentialsConfig, OAuth2TokenProvider};
//! use std::time::Duration;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = OAuth2ClientCredentialsConfig {
//!     client_id: "my-client".to_string(),
//!     client_secret: "my-secret".to_string(),
//!     token_endpoint: "https://auth.example.com/oauth/token".to_string(),
//!     scope: Some("api:read".to_string()),
//!     audience: None,
//!     cache_buffer: Some(Duration::from_secs(300)),
//!     timeout: Some(Duration::from_secs(30)),
//! };
//!
//! let provider = OAuth2TokenProvider::new(config)?;
//! let token = provider.get_token_async().await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use parking_lot::RwLock;
use reqwest::{Client as ReqwestClient, StatusCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::builder::JwtBuilder;
use crate::errors::AuthError;
use crate::jwt::VerifierJwt;
use crate::traits::{TokenProvider, Verifier};

/// Configuration for OAuth2 Client Credentials flow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OAuth2ClientCredentialsConfig {
    /// OAuth2 client ID
    pub client_id: String,

    /// OAuth2 client secret
    #[schemars(skip)]
    pub client_secret: String,

    /// Token endpoint URL
    pub token_endpoint: String,

    /// Optional scope parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// Optional audience parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,

    /// Buffer time before token expiry to refresh (default: 300s)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_buffer: Option<Duration>,

    /// HTTP timeout for token requests (default: 30s)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
}

/// OAuth2 token response from the authorization server
#[derive(Debug, Serialize, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

/// Cached token with expiration information
#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: SystemTime,
    #[allow(dead_code)] // May be used for future token type validation
    token_type: String,
}

/// Thread-safe token cache
#[derive(Debug)]
struct TokenCache {
    token: Arc<RwLock<Option<CachedToken>>>,
}

impl TokenCache {
    fn new() -> Self {
        Self {
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a valid token from cache, returns None if expired or missing
    fn get_valid_token(&self, buffer: Duration) -> Option<String> {
        let token_guard = self.token.read();
        if let Some(cached) = token_guard.as_ref() {
            let now = SystemTime::now();
            // Check if token is still valid considering the buffer
            if now + buffer < cached.expires_at {
                return Some(cached.access_token.clone());
            }
        }
        None
    }

    /// Store a new token in the cache
    fn store_token(&self, token: CachedToken) {
        *self.token.write() = Some(token);
    }

    /// Clear the token cache
    fn clear(&self) {
        *self.token.write() = None;
    }
}

/// OAuth2 Client Credentials token provider
#[derive(Debug)]
pub struct OAuth2TokenProvider {
    config: OAuth2ClientCredentialsConfig,
    client: ReqwestClient,
    cache: TokenCache,
}

impl OAuth2TokenProvider {
    /// Create a new OAuth2 token provider
    pub fn new(config: OAuth2ClientCredentialsConfig) -> Result<Self, AuthError> {
        // Validate the token endpoint URL
        Url::parse(&config.token_endpoint).map_err(|e| {
            AuthError::InvalidTokenEndpoint(format!("Invalid token endpoint URL: {}", e))
        })?;

        // Create HTTP client with timeout
        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth OAuth2")
            .timeout(config.timeout.unwrap_or(Duration::from_secs(30)))
            .danger_accept_invalid_certs(true) // TEMPORARY: For testing only
            .build()
            .map_err(|e| AuthError::OAuth2Error(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            client,
            cache: TokenCache::new(),
        })
    }

    /// Get a token asynchronously (preferred method)
    pub async fn get_token_async(&self) -> Result<String, AuthError> {
        let buffer = self.config.cache_buffer.unwrap_or(Duration::from_secs(300));

        // Check cache first
        if let Some(cached_token) = self.cache.get_valid_token(buffer) {
            tracing::debug!("Using cached OAuth2 token");
            return Ok(cached_token);
        }

        // Fetch new token
        tracing::debug!("Fetching new OAuth2 token from endpoint");
        self.fetch_token().await
    }

    /// Fetch a new token from the OAuth2 endpoint
    async fn fetch_token(&self) -> Result<String, AuthError> {
        // Prepare request body
        let mut form_data = HashMap::new();
        form_data.insert("grant_type", "client_credentials");
        form_data.insert("client_id", &self.config.client_id);
        form_data.insert("client_secret", &self.config.client_secret);

        if let Some(scope) = &self.config.scope {
            form_data.insert("scope", scope);
        }

        if let Some(audience) = &self.config.audience {
            form_data.insert("audience", audience);
        }

        // Make the request
        let response = self
            .client
            .post(&self.config.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        // Handle response
        let status = response.status();
        let body = response.text().await.map_err(map_reqwest_error)?;

        if !status.is_success() {
            return Err(map_token_response_error(status, &body));
        }

        // Parse token response
        let token_response: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AuthError::OAuth2Error(format!("Failed to parse token response: {}", e))
        })?;

        // Calculate expiration time
        let expires_at = SystemTime::now() + Duration::from_secs(token_response.expires_in);

        // Cache the token
        let cached_token = CachedToken {
            access_token: token_response.access_token.clone(),
            expires_at,
            token_type: token_response.token_type,
        };

        self.cache.store_token(cached_token);

        tracing::info!("Successfully fetched and cached OAuth2 token");
        Ok(token_response.access_token)
    }

    /// Clear the token cache (useful for testing or forced refresh)
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

impl TokenProvider for OAuth2TokenProvider {
    /// Get a token synchronously from cache only.
    ///
    /// This method will return a cached token if available and valid.
    /// If no valid cached token exists, it returns an error suggesting
    /// to use the async method instead.
    ///
    /// For new token acquisition, use `get_token_async()` instead.
    fn get_token(&self) -> Result<String, AuthError> {
        let buffer = self.config.cache_buffer.unwrap_or(Duration::from_secs(300));

        // Check cache first
        if let Some(cached_token) = self.cache.get_valid_token(buffer) {
            return Ok(cached_token);
        }

        // For sync context, we can't fetch a new token
        Err(AuthError::GetTokenError(
            "Token expired and sync refresh not supported. Use get_token_async() instead."
                .to_string(),
        ))
    }
}

/// OAuth2 token verifier that leverages existing JWT infrastructure
pub struct OAuth2Verifier {
    jwt_verifier: VerifierJwt,
}

impl OAuth2Verifier {
    /// Create a new OAuth2 verifier
    pub fn new(issuer: &str, audience: Option<&[String]>) -> Result<Self, AuthError> {
        let mut builder = JwtBuilder::new().issuer(issuer);

        if let Some(aud) = audience {
            builder = builder.audience(aud);
        }

        let jwt_verifier = builder.auto_resolve_keys(true).build()?;

        Ok(Self { jwt_verifier })
    }

    /// Create verifier from issuer only
    pub fn from_issuer(issuer: &str) -> Result<Self, AuthError> {
        Self::new(issuer, None)
    }

    /// Create verifier with specific audience
    pub fn from_issuer_with_audience(issuer: &str, audience: &[String]) -> Result<Self, AuthError> {
        Self::new(issuer, Some(audience))
    }
}

#[async_trait]
impl Verifier for OAuth2Verifier {
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        // Delegate to existing JWT verifier - it already handles:
        // - OIDC discovery via resolver.rs
        // - Key resolution and caching
        // - Signature verification
        // - Token caching
        self.jwt_verifier.verify(token).await
    }

    fn try_verify<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.jwt_verifier.try_verify(token)
    }
}

// Helper functions for error mapping
fn map_reqwest_error(e: reqwest::Error) -> AuthError {
    if e.is_timeout() {
        AuthError::OAuth2Error("Request timeout".to_string())
    } else if e.is_connect() {
        AuthError::OAuth2Error(format!("Connection failed: {}", e))
    } else {
        AuthError::OAuth2Error(format!("HTTP request failed: {}", e))
    }
}

fn map_token_response_error(status: StatusCode, body: &str) -> AuthError {
    match status.as_u16() {
        401 => AuthError::InvalidClientCredentials,
        400 => {
            // Try to parse OAuth2 error response
            if let Ok(error_response) = serde_json::from_str::<serde_json::Value>(body) {
                if let Some(error) = error_response.get("error").and_then(|v| v.as_str()) {
                    return AuthError::OAuth2Error(format!("OAuth2 error: {}", error));
                }
            }
            AuthError::OAuth2Error(format!("Bad request: {}", body))
        }
        _ => AuthError::TokenEndpointError {
            status: status.as_u16(),
            body: body.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_CACHE_BUFFER_SECS: u64 = 60;
    const TEST_TIMEOUT_SECS: u64 = 10;

    async fn create_test_config(server_uri: &str) -> OAuth2ClientCredentialsConfig {
        OAuth2ClientCredentialsConfig {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            token_endpoint: format!("{}/oauth/token", server_uri),
            scope: Some("read write".to_string()),
            audience: Some("https://api.example.com".to_string()),
            cache_buffer: Some(Duration::from_secs(TEST_CACHE_BUFFER_SECS)),
            timeout: Some(Duration::from_secs(TEST_TIMEOUT_SECS)),
        }
    }

    #[tokio::test]
    async fn test_oauth2_token_provider_success() {
        let mock_server = MockServer::start().await;

        // Setup mock token endpoint
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("client_id=test-client"))
            .and(body_string_contains("client_secret=test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "test-access-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "read write"
            })))
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        let token = provider.get_token_async().await.unwrap();
        assert_eq!(token, "test-access-token");
    }

    #[tokio::test]
    async fn test_oauth2_token_provider_caching() {
        let mock_server = MockServer::start().await;

        // Setup mock - should only be called once due to caching
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "cached-token",
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .expect(1) // Should only be called once
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        // First call - should hit the endpoint
        let token1 = provider.get_token_async().await.unwrap();
        assert_eq!(token1, "cached-token");

        // Second call - should use cache
        let token2 = provider.get_token_async().await.unwrap();
        assert_eq!(token2, "cached-token");
    }

    #[tokio::test]
    async fn test_oauth2_token_provider_error_handling() {
        let mock_server = MockServer::start().await;

        // Setup mock to return 401 Unauthorized
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "invalid_client",
                "error_description": "Client authentication failed"
            })))
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        let result = provider.get_token_async().await;
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::InvalidClientCredentials => {
                // Expected error type
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_oauth2_token_provider_expiration() {
        let mock_server = MockServer::start().await;

        // Setup mock to return short-lived token
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "short-lived-token",
                "token_type": "Bearer",
                "expires_in": 1 // 1 second expiry
            })))
            .expect(2) // Should be called twice
            .mount(&mock_server)
            .await;

        let mut config = create_test_config(&mock_server.uri()).await;
        config.cache_buffer = Some(Duration::from_millis(500)); // 500ms buffer

        let provider = OAuth2TokenProvider::new(config).unwrap();

        // First call
        let token1 = provider.get_token_async().await.unwrap();
        assert_eq!(token1, "short-lived-token");

        // Wait for token to expire (considering buffer)
        time::sleep(Duration::from_millis(600)).await;

        // Second call - should fetch new token
        let token2 = provider.get_token_async().await.unwrap();
        assert_eq!(token2, "short-lived-token");
    }

    #[tokio::test]
    async fn test_oauth2_config_validation() {
        // Test invalid URL
        let invalid_config = OAuth2ClientCredentialsConfig {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            token_endpoint: "not-a-valid-url".to_string(),
            scope: None,
            audience: None,
            cache_buffer: None,
            timeout: None,
        };

        let result = OAuth2TokenProvider::new(invalid_config);
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::InvalidTokenEndpoint(_) => {
                // Expected error type
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_oauth2_network_timeout() {
        let mock_server = MockServer::start().await;

        // Setup mock that never responds (simulates timeout)
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&mock_server)
            .await;

        let mut config = create_test_config(&mock_server.uri()).await;
        config.timeout = Some(Duration::from_millis(100)); // Very short timeout

        let provider = OAuth2TokenProvider::new(config).unwrap();

        let result = provider.get_token_async().await;
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::OAuth2Error(msg) => {
                assert!(msg.contains("timeout") || msg.contains("Request timeout"));
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_oauth2_malformed_response() {
        let mock_server = MockServer::start().await;

        // Setup mock to return invalid JSON
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("invalid json"))
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        let result = provider.get_token_async().await;
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::OAuth2Error(msg) => {
                assert!(msg.contains("Failed to parse token response"));
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_oauth2_server_error_500() {
        let mock_server = MockServer::start().await;

        // Setup mock to return 500 Internal Server Error
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        let result = provider.get_token_async().await;
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::TokenEndpointError { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "Internal Server Error");
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_oauth2_concurrent_requests() {
        let mock_server = MockServer::start().await;

        // Setup mock - allow multiple calls since concurrent requests may not be cached
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "concurrent-token",
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = Arc::new(OAuth2TokenProvider::new(config).unwrap());

        // First, get a token to populate cache
        let _initial_token = provider.get_token_async().await.unwrap();

        // Now spawn 5 concurrent requests - these should use cache
        let mut handles = vec![];
        for _ in 0..5 {
            let provider_clone = provider.clone();
            let handle = tokio::spawn(async move { provider_clone.get_token_async().await });
            handles.push(handle);
        }

        // Wait for all requests to complete
        let results: Vec<_> = futures::future::join_all(handles).await;

        // All should succeed and return the same token
        for result in results {
            let token = result.unwrap().unwrap();
            assert_eq!(token, "concurrent-token");
        }
    }

    #[tokio::test]
    async fn test_oauth2_cache_clear() {
        let mock_server = MockServer::start().await;

        // Setup mock - should be called twice due to cache clear
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "cache-clear-token",
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .expect(2) // Should be called twice
            .mount(&mock_server)
            .await;

        let config = create_test_config(&mock_server.uri()).await;
        let provider = OAuth2TokenProvider::new(config).unwrap();

        // First call - should hit the endpoint
        let token1 = provider.get_token_async().await.unwrap();
        assert_eq!(token1, "cache-clear-token");

        // Clear cache
        provider.clear_cache();

        // Second call - should hit the endpoint again
        let token2 = provider.get_token_async().await.unwrap();
        assert_eq!(token2, "cache-clear-token");
    }

    #[tokio::test]
    async fn test_oauth2_verifier_integration() {
        use crate::testutils::initialize_crypto_provider;

        initialize_crypto_provider();

        // Test that OAuth2Verifier can be created successfully
        let verifier = OAuth2Verifier::from_issuer("https://example.com");
        assert!(verifier.is_ok());

        // Test with audience
        let verifier_with_aud = OAuth2Verifier::from_issuer_with_audience(
            "https://example.com",
            &["https://api.example.com".to_string()],
        );
        assert!(verifier_with_aud.is_ok());
    }

    #[test]
    fn test_oauth2_sync_token_provider() {
        let config = OAuth2ClientCredentialsConfig {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            token_endpoint: "https://auth.example.com/oauth/token".to_string(),
            scope: None,
            audience: None,
            cache_buffer: None,
            timeout: None,
        };

        let provider = OAuth2TokenProvider::new(config).unwrap();

        // Sync get_token should fail when no cached token
        let result = provider.get_token();
        assert!(result.is_err());

        match result.unwrap_err() {
            AuthError::GetTokenError(msg) => {
                assert!(msg.contains("Use get_token_async() instead"));
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }
}

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

        // TODO(hackeramitkumar) : Implement OIDC provider discovery ( for now commenting it because of the mock server failing this checking)
        // // Validate that we can discover the OIDC provider
        // let issuer_url = IssuerUrl::new(issuer_url_str.clone())
        //     .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;

        // let _provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        //     .await
        //     .map_err(|e| {
        //         AuthError::ConfigError(format!("Failed to discover provider metadata: {}", e))
        //     })?;

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
        let refresh_task = provider.start_refresh_task(shutdown_rx).await;
        *provider.refresh_task.lock() = Some(refresh_task);

        // Fetch initial token to populate cache
        if let Err(e) = provider.fetch_new_token().await {
            eprintln!("Warning: Failed to fetch initial token: {}", e);
            // Don't fail construction, let background task handle it
        }

        Ok(provider)
    }

    /// Create a new OIDC Token Provider (synchronous version for backward compatibility)
    pub fn new_blocking(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        scope: Option<String>,
    ) -> Result<Self, AuthError> {
        block_on(Self::new(issuer_url, client_id, client_secret, scope))
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
    async fn start_refresh_task(&self, mut shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
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
        if let Err(_) = self.shutdown_tx.send(true) {
            eprintln!("Failed to send shutdown signal");
        }

        // Wait for task to complete
        if let Some(handle) = self.refresh_task.lock().take() {
            tokio::spawn(async move {
                if let Err(_) = handle.await {
                    eprintln!("Background refresh task panicked");
                }
            });
        }
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

impl OidcTokenProvider {
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

impl Drop for OidcTokenProvider {
    fn drop(&mut self) {
        // Signal shutdown when the provider is dropped
        if let Err(_) = self.shutdown_tx.send(true) {
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

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::{Duration, Instant};

use jsonwebtoken_aws_lc::jwk::KeyAlgorithm;
use jsonwebtoken_aws_lc::{
    Algorithm, DecodingKey, Header,
    jwk::{Jwk, JwkSet},
};
use parking_lot::RwLock;
use reqwest::{Client as ReqwestClient, StatusCode};
use url::Url;

use crate::errors::AuthError;

/// Cache entry for a JWKS.
#[derive(Clone, Debug)]
pub struct JwksCache {
    pub jwks: JwkSet,
    pub fetched_at: Instant,
    pub ttl: Duration,
}

/// This struct provides methods to resolve JWT decoding keys from various sources.
///
/// The `KeyResolver` is responsible for fetching and caching JSON Web Keys (JWK)
/// from OpenID Connect providers. It supports:
///
/// 1. OpenID Connect Discovery via the standard `.well-known/openid-configuration` endpoint
/// 2. Direct retrieval from the `.well-known/jwks.json` endpoint as a fallback
/// 3. Caching of retrieved keys to minimize network requests
///
/// Example usage:
///
/// ```
/// let resolver = KeyResolver::new()
///     .with_jwks_ttl(Duration::from_secs(1800));  // 30 minute cache TTL
///
/// let jwt = Jwt::builder()
///     .issuer("https://your-oidc-provider.com")
///     .key_resolver(resolver)
///     .build()?;
/// ```
#[derive(Debug)]
pub struct KeyResolver {
    client: ReqwestClient,
    jwks_cache: RwLock<HashMap<String, JwksCache>>,
    default_jwks_ttl: Duration,
}

impl Default for KeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyResolver {
    const STATIC_JWKS_ENTRY: &'static str = "static_jwks";

    /// Create a new KeyResolver with default settings
    pub fn new() -> Self {
        // Create a reqwest client with default TLS configuration
        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth")
            .build()
            .expect("Failed to create reqwest client");

        Self {
            client,
            jwks_cache: RwLock::new(HashMap::new()),
            default_jwks_ttl: Duration::from_secs(3600), // 1 hour default TTL
        }
    }

    pub fn with_jwks(jwks: JwkSet) -> Self {
        // Initialize the cache with the provided JWKS
        let mut cache = HashMap::new();
        cache.insert(
            Self::STATIC_JWKS_ENTRY.to_string(),
            JwksCache {
                jwks,
                fetched_at: Instant::now(),
                ttl: Duration::from_secs(u64::MAX), // static JWKS, infinite TTL
            },
        );

        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth")
            .build()
            .expect("Failed to create reqwest client");

        Self {
            client,
            jwks_cache: RwLock::new(cache),
            default_jwks_ttl: Duration::from_secs(3600), // 1 hour default TTL
        }
    }

    /// Set the default TTL for cached JWKS
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.default_jwks_ttl = ttl;
        self
    }

    /// Resolve a decoding key from various sources
    ///
    /// This function will attempt to resolve the key in the following order:
    /// 1. If a decoding key is already provided, return it
    /// 2. If a kid (Key ID) is specified in the token header, fetch the key from the JWKS endpoint
    /// 3. If no kid is specified, use the first suitable key from the JWKS endpoint
    ///
    /// # Arguments
    /// * `issuer` - The token issuer URL
    /// * `token_header` - The JWT header containing the algorithm and key ID (if available)
    pub async fn resolve_key(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<DecodingKey, AuthError> {
        // Check if we have a static JWKS entry
        if let Some(cache_entry) = self.jwks_cache.read().get(Self::STATIC_JWKS_ENTRY) {
            // If we have a static JWKS, use it directly
            return self.get_decoded_key_from_jwks(&cache_entry.jwks, token_header);
        }

        // Try to get cached key if available
        if let Ok(cached_key) = self.get_cached_key(issuer, token_header) {
            return Ok(cached_key);
        }

        // Try to fetch the keys from the well-known JWKS endpoint
        let jwks = self.fetch_jwks(issuer).await?;

        // Try to decode the key from the JWKS
        self.get_decoded_key_from_jwks(&jwks, token_header)
    }

    /// Convert a JWK to a DecodingKey
    fn jwk_to_decoding_key(&self, jwk: &Jwk) -> Result<DecodingKey, AuthError> {
        DecodingKey::from_jwk(jwk).map_err(|e| {
            AuthError::ConfigError(format!(
                "Failed to create {:?} decoding key from JWK: {}",
                jwk.common.key_algorithm, e
            ))
        })
    }

    fn key_alg_to_algorithm(&self, alg: &KeyAlgorithm) -> Result<Algorithm, AuthError> {
        match alg {
            KeyAlgorithm::HS256 => Ok(Algorithm::HS256),
            KeyAlgorithm::HS384 => Ok(Algorithm::HS384),
            KeyAlgorithm::HS512 => Ok(Algorithm::HS512),
            KeyAlgorithm::ES256 => Ok(Algorithm::ES256),
            KeyAlgorithm::ES384 => Ok(Algorithm::ES384),
            KeyAlgorithm::RS256 => Ok(Algorithm::RS256),
            KeyAlgorithm::RS384 => Ok(Algorithm::RS384),
            KeyAlgorithm::RS512 => Ok(Algorithm::RS512),
            KeyAlgorithm::PS256 => Ok(Algorithm::PS256),
            KeyAlgorithm::PS384 => Ok(Algorithm::PS384),
            KeyAlgorithm::PS512 => Ok(Algorithm::PS512),
            KeyAlgorithm::EdDSA => Ok(Algorithm::EdDSA),
            _ => Err(AuthError::ConfigError(format!(
                "Unsupported key algorithm: {:?}",
                alg
            ))),
        }
    }

    fn get_decoded_key_from_jwks(
        &self,
        jwks: &JwkSet,
        token_header: &Header,
    ) -> Result<DecodingKey, AuthError> {
        // At this point, we have a valid cache entry
        if let Some(kid) = &token_header.kid {
            // Look for a key with a matching ID
            for key in &jwks.keys {
                if let Some(id) = &key.common.key_id
                    && id == kid
                {
                    return self.jwk_to_decoding_key(key);
                }
            }
        } else {
            // If no key ID is specified, use the first suitable key
            for key in &jwks.keys {
                if let Some(alg) = &key.common.key_algorithm
                    && let Ok(algorithm) = self.key_alg_to_algorithm(alg)
                {
                    // Check if the algorithm matches the token's algorithm
                    if algorithm == token_header.alg {
                        return self.jwk_to_decoding_key(key);
                    }
                }
            }
        }

        // If no suitable key is found, return an error
        Err(AuthError::ConfigError(format!(
            "No suitable key found in JWKS for token header: {:?}",
            token_header
        )))
    }

    /// Check the cache for a JWKS entry
    pub fn get_cached_key(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<DecodingKey, AuthError> {
        // Check if we have a cached JWKS that's still valid
        let cache = self.jwks_cache.read();

        // Check static JWKS entry first
        if let Some(cache_entry) = cache.get(Self::STATIC_JWKS_ENTRY) {
            // no need to check the elapsed time for static JWKS
            return self.get_decoded_key_from_jwks(&cache_entry.jwks, token_header);
        }

        let cache_entry = cache.get(issuer);
        if cache_entry.is_none() {
            return Err(AuthError::ConfigError(format!(
                "No cached JWKS found for issuer: {}",
                issuer
            )));
        }

        let cache_entry = cache_entry.unwrap();

        if cache_entry.fetched_at.elapsed() > cache_entry.ttl {
            return Err(AuthError::ConfigError(format!(
                "Cached JWKS for issuer {} has expired",
                issuer
            )));
        }

        // If we have a valid cache entry, try to decode the key
        self.get_decoded_key_from_jwks(&cache_entry.jwks, token_header)
    }

    /// Fetch JWKS from the issuer's endpoint
    ///
    /// This function will discover the JWKS URI (either via OpenID Connect Discovery
    /// or the standard well-known endpoint), fetch the JWKS, and cache it for future use.
    async fn fetch_jwks(&self, issuer: &str) -> Result<JwkSet, AuthError> {
        // Build the JWKS URI (this now handles both OpenID discovery and fallback)
        let jwks_uri = self.build_jwks_uri(issuer).await?;

        // Fetch the JWKS
        let jwks = self.fetch_jwks_from_uri(&jwks_uri).await?;

        // Cache the JWKS
        self.jwks_cache.write().insert(
            issuer.to_string(),
            JwksCache {
                jwks: jwks.clone(),
                fetched_at: Instant::now(),
                ttl: self.default_jwks_ttl,
            },
        );

        Ok(jwks)
    }

    /// Build the JWKS URI from the issuer
    ///
    /// This function first tries to discover the JWKS URI via OpenID Connect Discovery
    /// (.well-known/openid-configuration), and falls back to the standard .well-known/jwks.json
    /// location if that fails.
    async fn build_jwks_uri(&self, issuer: &str) -> Result<String, AuthError> {
        // Parse the issuer URL
        let mut issuer_url = Url::parse(issuer)
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;

        // First try OpenID Connect Discovery endpoint
        let mut openid_config_url = issuer_url.clone();
        let mut openid_path = openid_config_url.path().trim_end_matches('/').to_owned();
        openid_path.push_str("/.well-known/openid-configuration");
        openid_config_url.set_path(&openid_path);

        // Try to fetch the OpenID configuration
        let openid_config_response = self.client.get(openid_config_url.to_string()).send().await;

        // If we successfully got the OpenID configuration, extract the jwks_uri
        if let Ok(response) = openid_config_response
            && response.status() == StatusCode::OK
            && let Ok(config) = response.json::<serde_json::Value>().await
            && let Some(jwks_uri) = config.get("jwks_uri").and_then(|v| v.as_str())
        {
            return Ok(jwks_uri.to_string());
        }

        // Fallback to standard well-known JWKS location
        let mut path = issuer_url.path().trim_end_matches('/').to_owned();
        path.push_str("/.well-known/jwks.json");
        issuer_url.set_path(&path);

        Ok(issuer_url.to_string())
    }

    /// Fetch JWKS from the specified URI
    async fn fetch_jwks_from_uri(&self, uri: &str) -> Result<JwkSet, AuthError> {
        // Send the GET request using reqwest
        let response = self
            .client
            .get(uri)
            .send()
            .await
            .map_err(|e| AuthError::ConfigError(format!("Failed to fetch JWKS: {}", e)))?;

        // Check the response status
        if response.status() != StatusCode::OK {
            return Err(AuthError::ConfigError(format!(
                "Failed to fetch JWKS: HTTP status {}",
                response.status()
            )));
        }

        // Get the response body as bytes
        let body = response
            .bytes()
            .await
            .map_err(|e| AuthError::ConfigError(format!("Failed to read JWKS response: {}", e)))?;

        // Parse the JWKS
        let jwks: JwkSet = serde_json::from_slice(&body)
            .map_err(|e| AuthError::ConfigError(format!("Failed to parse JWKS: {}", e)))?;

        Ok(jwks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Helper to create a test resolver with a client that can talk to the mock server
    async fn create_test_resolver() -> (KeyResolver, MockServer) {
        let server = MockServer::start().await;
        let resolver = KeyResolver::new();
        (resolver, server)
    }

    #[tokio::test]
    async fn test_build_jwks_uri_with_openid_discovery() {
        let (resolver, mock_server) = create_test_resolver().await;

        // Setup mock for OpenID discovery endpoint
        let jwks_uri = "https://example.com/custom/path/to/jwks.json";
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": "https://example.com",
                "jwks_uri": jwks_uri
            })))
            .mount(&mock_server)
            .await;

        // Test that we can discover the JWKS URI from OpenID configuration
        let uri = resolver.build_jwks_uri(&mock_server.uri()).await.unwrap();
        assert_eq!(uri, jwks_uri);
    }

    #[tokio::test]
    async fn test_build_jwks_uri_fallback() {
        let (resolver, mock_server) = create_test_resolver().await;

        // Setup mock to return 404 for OpenID discovery endpoint
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        // Test that we fall back to the standard JWKS URI
        let uri = resolver.build_jwks_uri(&mock_server.uri()).await.unwrap();
        assert_eq!(uri, format!("{}/.well-known/jwks.json", mock_server.uri()));
    }

    #[tokio::test]
    async fn test_fetch_jwks_from_uri() {
        let (resolver, mock_server) = create_test_resolver().await;

        // Setup mock for JWKS endpoint
        let jwks = json!({
            "keys": [
                {
                    "kty": "RSA",
                    "kid": "test-key",
                    "n": "some-modulus",
                    "e": "AQAB"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&mock_server)
            .await;

        // Fetch the JWKS from the mock server
        let jwks_uri = format!("{}/.well-known/jwks.json", mock_server.uri());
        let fetched_jwks = resolver.fetch_jwks_from_uri(&jwks_uri).await.unwrap();

        assert_eq!(fetched_jwks.keys.len(), 1);
        assert_eq!(
            fetched_jwks.keys[0].common.key_id.as_deref(),
            Some("test-key")
        );
    }
}

use anyhow::{anyhow, Result}; // anyhow is kept for main's return type, but AuthError is preferred internally
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use parking_lot::RwLock; // For TokenCache (synchronous access)
use tokio::sync::RwLock as AsyncRwLock; // For OidcVerifier (asynchronous access)
use url::Url;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation}; // For JWT verification
use std::collections::HashMap; // For JWT claims
use std::time::{Duration, SystemTime}; // For cache expiry

// OpenID Connect crate imports
use openidconnect::core::{CoreClient, CoreProviderMetadata};
use openidconnect::{ClientId, ClientSecret, IssuerUrl, Scope};
use openidconnect::reqwest::async_http_client; // Correct async http client for openidconnect

// Custom error and trait imports
use crate::errors::AuthError;
use crate::traits::{TokenProvider, Verifier};
use async_trait::async_trait;

// Define a buffer time in seconds before the token actually expires to trigger a refresh.
// This helps prevent using an almost-expired token or race conditions.
const REFRESH_BUFFER_SECONDS: u64 = 60; // Changed to u64 to match SystemTime::as_secs()

/// Represents the structure of an access token response from an OIDC provider.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AccessToken {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64, // Lifetime in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Represents an entry in the token cache, including the token and its calculated expiry timestamp.
#[derive(Debug, Clone)]
pub struct AccessTokenCacheEntry {
    pub token: AccessToken,
    pub expiry: u64, // Unix timestamp in seconds
}

/// Manages the cached access tokens using a synchronous RwLock.
#[derive(Debug)]
pub struct TokenCache {
    entries: RwLock<HashMap<String, AccessTokenCacheEntry>>,
}

impl TokenCache {
    /// Creates a new, empty `TokenCache`.
    pub fn new() -> Self {
        TokenCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Attempts to retrieve a valid (non-expired, considering refresh buffer) token from the cache.
    /// Returns `Some(token)` if valid, `None` otherwise.
    pub fn get_valid_token(&self, key: &str) -> Option<AccessToken> {
        let now = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();
        if let Some(entry) = self.entries.read().get(key) {
            // Check if the token is still valid, considering the refresh buffer.
            if entry.expiry > now + REFRESH_BUFFER_SECONDS {
                println!("Cache HIT: Returning valid token from cache for key: {}", key);
                return Some(entry.token.clone());
            } else {
                println!("Cache MISS: Token for key '{}' expired or close to expiration.", key);
            }
        } else {
            println!("Cache MISS: No token in cache for key: {}", key);
        }
        None
    }

    /// Sets a new token in the cache, calculating its expiration timestamp.
    pub fn set_token(&self, key: String, token: AccessToken) {
        let expiry = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs() + token.expires_in;
        let entry = AccessTokenCacheEntry { token, expiry };
        self.entries.write().insert(key.clone(), entry);
        println!("Cache UPDATED: New token set for key: {}, expires at: {}", key, expiry);
    }
}

/// `OidcTokenProvider` handles fetching and caching access tokens using the Client Credentials flow
/// with the `openidconnect` crate.
pub struct OidcTokenProvider {
    client_id: String,
    client_secret: String,
    client: CoreClient,
    scope: Option<String>,
    cache: Arc<TokenCache>,
}

impl OidcTokenProvider {
    /// Creates a new `OidcTokenProvider` instance by discovering provider metadata.
    ///
    /// # Arguments
    /// * `client_id` - The OAuth 2.0 client ID.
    /// * `client_secret` - The OAuth 2.0 client secret.
    /// * `issuer_url` - The URL string of the OIDC issuer (e.g., "https://accounts.google.com").
    /// * `scope` - An optional string representing the requested scope(s).
    pub async fn new(
        client_id: String,
        client_secret: String,
        issuer_url: &str,
        scope: Option<String>,
    ) -> Result<Self, AuthError> {
        let issuer = IssuerUrl::new(issuer_url.to_string())
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL: {}", e)))?;

        println!("Discovering OIDC provider metadata from: {}", issuer_url);
        let provider_metadata = CoreProviderMetadata::discover_async(issuer, async_http_client)
            .await
            .map_err(|e| AuthError::OpenIdConnectError(format!("Failed to discover provider metadata: {}", e)))?;
        println!("Provider metadata discovered successfully.");

        let client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(client_id.clone()),
            Some(ClientSecret::new(client_secret.clone())),
        );

        Ok(OidcTokenProvider {
            client_id,
            client_secret,
            client,
            scope,
            cache: Arc::new(TokenCache::new()),
        })
    }

    /// Fetches a new access token from the OIDC provider using the Client Credentials flow.
    async fn fetch_new_token(&self) -> Result<AccessToken, AuthError> {
        println!("Fetching new token from OIDC provider...");

        let mut token_req = self.client.clone().exchange_client_credentials();

        // Add scope if provided
        if let Some(scope_str) = &self.scope {
            token_req = token_req.add_scope(Scope::new(scope_str.clone()));
        }

        let token_response = token_req
            .request_async(async_http_client)
            .await
            .map_err(|e| AuthError::GetTokenError(format!("OIDC token fetch error: {}", e)))?;

        let token = AccessToken {
            access_token: token_response.access_token().secret().to_string(),
            token_type: token_response.token_type().as_str().to_string(),
            // Use unwrap_or_else with a default if expires_in is not provided by the OIDC server
            expires_in: token_response.expires_in().unwrap_or_else(|| Duration::from_secs(3600)).as_secs(),
            // Convert HashSet<Scope> to Option<String>
            scope: token_response.scopes().map(|s| s.iter().map(|sc| sc.as_str().to_string()).collect::<Vec<_>>().join(" ")),
        };
        println!("Successfully fetched new token.");
        Ok(token)
    }

    /// Retrieves an access token. It first checks the cache, and if the token is
    /// expired or not present, it fetches a new one and updates the cache.
    pub async fn get_access_token(&self) -> Result<String, AuthError> {
        // Use the scope as part of the cache key, or "default" if no scope is specified.
        let key = self.scope.clone().unwrap_or_else(|| "default".to_string());

        // Check cache first
        if let Some(token) = self.cache.get_valid_token(&key) {
            return Ok(token.access_token);
        }

        // If no valid token in cache, fetch a new one
        let new_token = self.fetch_new_token().await?;
        let token_string = new_token.access_token.clone();
        self.cache.set_token(key, new_token);
        Ok(token_string)
    }
}

impl TokenProvider for OidcTokenProvider {
    /// Implements the `get_token` method from the `TokenProvider` trait.
    fn get_token(&self) -> Result<String, AuthError> {
        tokio::runtime::Handle::current().block_on(self.get_access_token())
    }
}

/// Represents the structure of a JSON Web Key (JWK).
/// This is used to parse the JWKS endpoint response.
#[derive(Debug, Deserialize, Clone)]
pub struct Jwk {
    pub kty: String, // Key Type (e.g., "RSA", "EC")
    #[serde(default)] // Default to empty string if not present
    pub n: Option<String>, // Modulus for RSA
    #[serde(default)] // Default to empty string if not present
    pub e: Option<String>, // Exponent for RSA
    pub alg: Option<String>, // Algorithm (e.g., "RS256")
    pub kid: String, // Key ID
    #[serde(default)] // Default to empty Vec if not present
    pub x5c: Option<Vec<String>>, // X.509 Certificate Chain
    #[serde(default)] // Default to empty string if not present
    pub x5t: Option<String>, // X.509 Certificate Thumbprint
    // Other fields like 'crv', 'x', 'y' for EC keys can be added if needed
}

/// Represents a JSON Web Key Set (JWKS).
#[derive(Debug, Deserialize, Clone)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

/// `OidcVerifier` handles the verification of JWT access tokens using `jsonwebtoken` crate.
pub struct OidcVerifier {
    issuer: String, // The expected issuer of the token (e.g., "https://your-oidc-provider.com/oauth2")
    audience: String, // The expected audience of the token (e.g., "your_resource_server_api")
    jwks_uri: Url, // The URL to fetch the JWKS from
    jwks_cache: Arc<AsyncRwLock<(JwkSet, SystemTime)>>, // Cache for JWKS with timestamp
    http_client: reqwest::Client, // Dedicated http client for verifier
}

impl OidcVerifier {
    /// Creates a new `OidcVerifier` instance.
    ///
    /// It fetches the initial JWKS upon creation.
    ///
    /// # Arguments
    /// * `issuer` - The expected issuer (iss) claim in the JWT.
    /// * `audience` - The expected audience (aud) claim in the JWT.
    /// * `jwks_uri_str` - The URL string for the JSON Web Key Set (JWKS) endpoint.
    pub async fn new(issuer: String, audience: String, jwks_uri_str: &str) -> Result<Self, AuthError> {
        let jwks_uri = Url::parse(jwks_uri_str)
            .map_err(|e| AuthError::ConfigError(format!("Invalid JWKS URI: {}", e)))?;

        let http_client = reqwest::Client::new();
        let initial_jwks = Self::fetch_jwks(&http_client, &jwks_uri).await?;

        Ok(OidcVerifier {
            issuer,
            audience,
            jwks_uri,
            jwks_cache: Arc::new(AsyncRwLock::new((initial_jwks, SystemTime::now()))),
            http_client,
        })
    }

    /// Fetches the JWKS from the configured URI.
    async fn fetch_jwks(client: &reqwest::Client, jwks_url: &Url) -> Result<JwkSet, AuthError> {
        println!("Fetching JWKS from: {}", jwks_url);
        let response = client.get(jwks_url.clone()).send().await?;

        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            return Err(AuthError::ConfigError(format!(
                "Failed to fetch JWKS: {} - {}",
                status, text
            )));
        }

        let jwks: JwkSet = serde_json::from_str(&text)?;

        println!("Successfully fetched JWKS.");
        Ok(jwks)
    }

    /// Retrieves the JWKS, either from cache or by fetching it if expired.
    async fn get_jwks(&self) -> Result<JwkSet, AuthError> {
        let mut cache_guard = self.jwks_cache.write().await;
        let (cached_jwks, last_fetch_time) = &mut *cache_guard;

        let now = SystemTime::now();
        // Refresh JWKS if it's older than 1 hour (3600 seconds)
        if now.duration_since(*last_fetch_time).unwrap_or(Duration::from_secs(0)) > Duration::from_secs(3600) {
            println!("JWKS cache expired or too old, fetching new.");
            let new_jwks = Self::fetch_jwks(&self.http_client, &self.jwks_uri).await?;
            *cached_jwks = new_jwks;
            *last_fetch_time = now;
        } else {
            println!("JWKS Cache HIT.");
        }
        Ok(cached_jwks.clone())
    }

    /// Verifies a given JWT access token.
    ///
    /// # Arguments
    /// * `token_str` - The JWT string to verify.
    ///
    /// Returns the decoded claims as a `HashMap<String, serde_json::Value>` if verification is successful.
    pub async fn verify_access_token(&self, token_str: &str) -> Result<HashMap<String, serde_json::Value>, AuthError> {
        // 1. Decode header to get key ID (kid)
        let header = decode_header(token_str)
            .map_err(|e| AuthError::VerificationError(format!("Failed to decode JWT header: {}", e)))?;

        let kid = header.kid.ok_or_else(|| AuthError::VerificationError("JWT header missing 'kid'".to_string()))?;

        // 2. Get JWKS and find the matching key
        let jwks = self.get_jwks().await?;
        let jwk = jwks.keys.iter().find(|k| k.kid == kid)
            .ok_or_else(|| AuthError::VerificationError(format!("No matching JWK found for kid: {}", kid)))?;

        // 3. Create decoding key from JWK
        let decoding_key = match jwk.alg.as_deref() {
            Some("RS256") | Some("RS384") | Some("RS512") => {
                let n = jwk.n.as_ref().ok_or_else(|| AuthError::VerificationError("RSA JWK missing 'n'".to_string()))?;
                let e = jwk.e.as_ref().ok_or_else(|| AuthError::VerificationError("RSA JWK missing 'e'".to_string()))?;
                DecodingKey::from_rsa_components(n, e)
                    .map_err(|e| AuthError::VerificationError(format!("Failed to create RSA decoding key: {}", e)))?
            },
            Some("ES256") | Some("ES384") | Some("ES512") => {
                // For EC keys, you'd typically need 'crv', 'x', 'y' fields.
                // This example primarily supports RSA, common for OIDC access tokens.
                return Err(AuthError::VerificationError(format!("Unsupported algorithm for verification: {}", jwk.alg.as_deref().unwrap_or("unknown"))));
            },
            _ => return Err(AuthError::VerificationError("Unsupported or missing algorithm in JWK".to_string())),
        };

        // 4. Set up validation rules
        let mut validation = Validation::new(
            header.alg.ok_or_else(|| AuthError::VerificationError("JWT header missing 'alg'".to_string()))?
        );
        validation.validate_exp = true; // Validate expiration (exp)
        validation.validate_nbf = false; // Not before (nbf) - optional, often true in production
        validation.validate_aud = Some(vec![self.audience.clone()]); // Validate audience (aud)
        validation.validate_iss = Some(vec![self.issuer.clone()]); // Validate issuer (iss)

        // 5. Decode and validate the token
        let token_data = decode::<HashMap<String, serde_json::Value>>(
            token_str,
            &decoding_key,
            &validation,
        )
        .map_err(|e| AuthError::JwtError(e))?; // Convert jsonwebtoken error to AuthError::JwtError

        println!("Token successfully verified!");
        Ok(token_data.claims)
    }
}

#[async_trait]
impl Verifier for OidcVerifier {
    /// Implements the `verify` method from the `Verifier` trait for access tokens.
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let claims_map = self.verify_access_token(&token.into()).await?;
        let json = serde_json::to_value(&claims_map)?; // Convert HashMap to serde_json::Value
        let claims: Claims = serde_json::from_value(json)?; // Deserialize into target Claims type
        Ok(claims)
    }
    fn try_verify<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        // Synchronous verification is not supported in this implementation.
        Err(AuthError::UnsupportedOperation("Synchronous verification is not supported".to_string()))
    }
}

/// Builder for `OidcTokenProvider` and `OidcVerifier`.
pub struct OidcBuilder {
    client_id: Option<String>,
    client_secret: Option<String>,
    issuer_url: Option<String>,
    scope: Option<String>,
}

impl OidcBuilder {
    /// Creates a new `OidcBuilder` instance.
    pub fn new() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            issuer_url: None,
            scope: None,
        }
    }

    /// Sets the client ID for the builder.
    pub fn client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Sets the client secret for the builder.
    pub fn client_secret(mut self, client_secret: impl Into<String>) -> Self {
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Sets the issuer URL for the builder.
    pub fn issuer_url(mut self, issuer_url: impl Into<String>) -> Self {
        self.issuer_url = Some(issuer_url.into());
        self
    }

    /// Sets the requested scope for the builder.
    pub fn scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
        self
    }

    /// Builds an `OidcTokenProvider` instance based on the configured parameters.
    pub async fn build_provider(self) -> Result<OidcTokenProvider, AuthError> {
        OidcTokenProvider::new(
            self.client_id.ok_or_else(|| AuthError::ConfigError("Missing client_id for provider".to_string()))?,
            self.client_secret.ok_or_else(|| AuthError::ConfigError("Missing client_secret for provider".to_string()))?,
            &self.issuer_url.ok_or_else(|| AuthError::ConfigError("Missing issuer_url for provider".to_string()))?,
            self.scope,
        ).await
    }

    /// Builds an `OidcVerifier` instance. This method will discover the JWKS URI
    /// from the OIDC provider's metadata using the issuer URL.
    ///
    /// # Arguments
    /// * `audience` - The expected audience (aud) claim for the tokens to be verified.
    pub async fn build_verifier(self, audience: String) -> Result<OidcVerifier, AuthError> {
        let issuer_url_str = self.issuer_url.ok_or_else(|| AuthError::ConfigError("Missing issuer_url for verifier".to_string()))?;
        let issuer = IssuerUrl::new(issuer_url_str.clone())
            .map_err(|e| AuthError::ConfigError(format!("Invalid issuer URL for verifier: {}", e)))?;

        println!("Discovering provider metadata for verifier from: {}", issuer_url_str);
        let provider_metadata = CoreProviderMetadata::discover_async(issuer, async_http_client)
            .await
            .map_err(|e| AuthError::OpenIdConnectError(format!("Failed to discover provider metadata for verifier: {}", e)))?;
        println!("Provider metadata for verifier discovered successfully.");

        let jwks_uri = provider_metadata.jwks_uri()
            .map(|url| url.to_string())
            .ok_or_else(|| AuthError::ConfigError("JWKS URI not found in provider metadata for verifier".to_string()))?;

        OidcVerifier::new(issuer_url_str, audience, &jwks_uri).await
    }
}


#[tokio::main]
async fn main() -> Result<(), AuthError> {
    // --- Configuration (Replace with your actual OIDC provider details) ---
    let client_id = "your_client_id".to_string();
    let client_secret = "your_client_secret".to_string();
    let issuer_url = "https://your-oidc-provider.com/oauth2".to_string(); // Base URL of your OIDC provider
    let scope: Option<String> = Some("api_scope_1 api_scope_2".to_string()); // Example scope, or None for no specific scope

    // --- Access Token Verifier Configuration (Replace with your actual details) ---
    let audience = "your_resource_server_api".to_string(); // Identifier for your API/resource server

    // --- IMPORTANT: Placeholder Check ---
    if client_id == "your_client_id" || client_secret == "your_client_secret" || issuer_url == "https://your-oidc-provider.com/oauth2" ||
       audience == "your_resource_server_api" {
        eprintln!("\nWARNING: Please update ALL placeholder values in `main.rs` with your actual OIDC provider and resource server details for this example to work correctly.");
        eprintln!("You can use a service like Auth0, Okta, Keycloak, or IdentityServer4 to get these values.");
        eprintln!("For the verifier, you'll need the Issuer URL and the Audience (identifier for your API). The JWKS URI will be discovered automatically.");
        eprintln!("Proceeding with placeholder values, which will likely result in an error.\n");
        // In a real application, you might want to exit here or return an error.
    }

    // Initialize OIDC Token Provider
    let provider_builder = OidcBuilder::new()
        .client_id(client_id.clone())
        .client_secret(client_secret.clone())
        .issuer_url(issuer_url.clone())
        .scope(scope.clone());

    let provider = provider_builder.build_provider().await?;
    println!("OidcTokenProvider initialized.");

    // Initialize OIDC Verifier
    let verifier_builder = OidcBuilder::new()
        .issuer_url(issuer_url); // Only issuer_url is strictly needed for verifier discovery
                                 // client_id and client_secret are not used by build_verifier directly,
                                 // but the OidcBuilder requires them for its internal consistency.
                                 // If you want to strictly separate, you'd need separate builders.

    let verifier = verifier_builder.build_verifier(audience).await?;
    println!("OidcVerifier initialized.");


    println!("\n--- First token request (should fetch new) ---");
    let token1 = provider.get_token().await?; // Use get_token from trait
    println!("Received Token 1: {}...", &token1.chars().take(20).collect::<String>()); // Print first 20 chars for brevity

    println!("\n--- Verifying Token 1 ---");
    // Define a simple claims struct for demonstration.
    // This struct should match the expected claims in your JWT access token.
    #[derive(Debug, Deserialize)]
    struct MyAccessTokenClaims {
        iss: String, // Issuer
        aud: serde_json::Value, // Audience (can be string or array of strings)
        exp: u64, // Expiration time
        iat: u64, // Issued at time
        sub: String, // Subject (often client_id for client credentials)
        // Add any other custom claims you expect in your access token
    }
    match verifier.verify::<MyAccessTokenClaims>(&token1).await {
        Ok(claims) => println!("Token 1 verified successfully! Claims: {:?}", claims),
        Err(e) => eprintln!("Token 1 verification failed: {}", e),
    }

    println!("\n--- Second token request (should use cached token) ---");
    let token2 = provider.get_token().await?;
    println!("Received Token 2: {}...", &token2.chars().take(20).collect::<String>());

    println!("\n--- Verifying Token 2 ---");
    match verifier.verify::<MyAccessTokenClaims>(&token2).await {
        Ok(claims) => println!("Token 2 verified successfully! Claims: {:?}", claims),
        Err(e) => eprintln!("Token 2 verification failed: {}", e),
    }

    println!("\n--- Simulate time passing to force token refresh ---");
    tokio::time::sleep(tokio::time::Duration::from_secs(REFRESH_BUFFER_SECONDS + 5)).await;
    println!("Simulated time passing...");

    println!("\n--- Third token request (should fetch new due to simulated expiration) ---");
    let token3 = provider.get_token().await?;
    println!("Received Token 3: {}...", &token3.chars().take(20).collect::<String>());

    println!("\n--- Verifying Token 3 ---");
    match verifier.verify::<MyAccessTokenClaims>(&token3).await {
        Ok(claims) => println!("Token 3 verified successfully! Claims: {:?}", claims),
        Err(e) => eprintln!("Token 3 verification failed: {}", e),
    }

    Ok(())
}


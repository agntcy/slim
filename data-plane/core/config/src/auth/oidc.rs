// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use slim_auth::oidc::{OidcProviderConfig, OidcTokenProvider, OidcVerifier};
use slim_auth::traits::TokenProvider; // bring trait into scope for initialize()

/// Unified OIDC Configuration that can act as both provider and verifier
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    /// OIDC issuer URL (e.g., https://auth.example.com)
    pub issuer_url: String,

    /// OAuth2 client ID (required for provider functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// OAuth2 client secret (required for provider functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,

    /// Expected audience for JWT tokens (required for verifier functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,

    /// Optional scope parameter for the token request (provider only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// HTTP timeout for token requests (default: 30s, provider only)
    #[serde(default = "default_timeout")]
    #[schemars(with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,

    /// JWKS cache TTL (default: 1 hour, verifier only)
    #[serde(default = "default_jwks_ttl")]
    #[schemars(with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwks_ttl: Option<Duration>,
}

fn default_timeout() -> Option<Duration> {
    Some(Duration::from_secs(30))
}

fn default_jwks_ttl() -> Option<Duration> {
    Some(Duration::from_secs(3600)) // 1 hour
}

impl Config {
    /// Create a new OIDC configuration with the issuer URL
    pub fn new(issuer_url: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: None,
            client_secret: None,
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
        }
    }

    /// Create a provider-only configuration
    pub fn provider(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        issuer_url: impl Into<String>,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
        }
    }

    /// Create a verifier-only configuration
    pub fn verifier(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: None,
            client_secret: None,
            audience: Some(audience.into()),
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
        }
    }

    /// Create a combined configuration that can work as both provider and verifier
    pub fn combined(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            audience: Some(audience.into()),
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
        }
    }

    /// Set the client credentials for provider functionality
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Set the audience for verifier functionality
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Set the scope for the OIDC token request (provider functionality)
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Set the HTTP timeout for token requests (provider functionality)
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the JWKS cache TTL (verifier functionality)
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_ttl = Some(ttl);
        self
    }

    /// Check if this configuration can act as a provider
    pub fn can_provide(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }

    /// Check if this configuration can act as a verifier
    pub fn can_verify(&self) -> bool {
        self.audience.is_some()
    }

    /// Convert to the auth crate's OidcProviderConfig
    fn to_auth_config(&self) -> Result<OidcProviderConfig, AuthError> {
        let client_id = self.client_id.as_ref().ok_or_else(|| {
            AuthError::ConfigError("client_id is required for provider functionality".to_string())
        })?;
        let client_secret = self.client_secret.as_ref().ok_or_else(|| {
            AuthError::ConfigError(
                "client_secret is required for provider functionality".to_string(),
            )
        })?;

        Ok(OidcProviderConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            issuer_url: self.issuer_url.clone(),
            scope: self.scope.clone(),
            timeout: self.timeout,
        })
    }

    /// Create an OIDC token provider from this configuration
    pub async fn create_provider(&self) -> Result<OidcTokenProvider, AuthError> {
        let config = self.to_auth_config()?;
        let mut provider = OidcTokenProvider::new(config).map_err(|e| {
            AuthError::ConfigError(format!("Failed to create OIDC provider: {}", e))
        })?;
        provider.initialize().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize OIDC provider: {}", e))
        })?;
        Ok(provider)
    }

    /// Create an OIDC verifier from this configuration
    pub fn create_verifier(&self) -> Result<OidcVerifier, AuthError> {
        let audience = self.audience.as_ref().ok_or_else(|| {
            AuthError::ConfigError("audience is required for verifier functionality".to_string())
        })?;

        let verifier = OidcVerifier::new(&self.issuer_url, audience);
        Ok(if let Some(ttl) = self.jwks_ttl {
            verifier.with_jwks_ttl(ttl)
        } else {
            verifier
        })
    }
}

// Simple wrapper types for the middleware layers
pub struct OidcProviderLayer {
    provider: OidcTokenProvider,
}

impl OidcProviderLayer {
    pub fn new(provider: OidcTokenProvider) -> Self {
        Self { provider }
    }

    pub fn provider(&self) -> &OidcTokenProvider {
        &self.provider
    }
}

pub struct OidcVerifierLayer {
    verifier: OidcVerifier,
}

impl OidcVerifierLayer {
    pub fn new(verifier: OidcVerifier) -> Self {
        Self { verifier }
    }

    pub fn verifier(&self) -> &OidcVerifier {
        &self.verifier
    }
}

// Implement ClientAuthenticator for Config
impl ClientAuthenticator for Config {
    type ClientLayer = OidcProviderLayer;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        if !self.can_provide() {
            return Err(AuthError::ConfigError(
                "Configuration missing client credentials for provider functionality".to_string(),
            ));
        }

        // OidcTokenProvider::new is now sync, but initialization is async
        Err(AuthError::ConfigError(
            "OIDC provider requires async initialization. Use create_provider() instead."
                .to_string(),
        ))
    }
}

// Implement ServerAuthenticator for Config
impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    type ServerLayer = OidcVerifierLayer;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        if !self.can_verify() {
            return Err(AuthError::ConfigError(
                "Configuration missing audience for verifier functionality".to_string(),
            ));
        }

        let verifier = self.create_verifier()?;
        Ok(OidcVerifierLayer::new(verifier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_provider_config_creation() {
        let config = Config::provider(
            "test-client-id",
            "test-client-secret",
            "https://auth.example.com",
        )
        .with_scope("api:read")
        .with_timeout(Duration::from_secs(45));

        assert_eq!(config.client_id, Some("test-client-id".to_string()));
        assert_eq!(config.client_secret, Some("test-client-secret".to_string()));
        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.timeout, Some(Duration::from_secs(45)));
        assert!(config.can_provide());
        assert!(!config.can_verify());
    }

    #[test]
    fn test_verifier_config_creation() {
        let config = Config::verifier("https://auth.example.com", "test-audience")
            .with_jwks_ttl(Duration::from_secs(1800));

        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.audience, Some("test-audience".to_string()));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800)));
        assert!(!config.can_provide());
        assert!(config.can_verify());
    }

    #[test]
    fn test_combined_config_creation() {
        let config = Config::combined(
            "https://auth.example.com",
            "client-id",
            "client-secret",
            "audience",
        )
        .with_scope("api:read")
        .with_jwks_ttl(Duration::from_secs(1800));

        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.client_id, Some("client-id".to_string()));
        assert_eq!(config.client_secret, Some("client-secret".to_string()));
        assert_eq!(config.audience, Some("audience".to_string()));
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800)));
        assert!(config.can_provide());
        assert!(config.can_verify());
    }

    #[test]
    fn test_config_builder_pattern() {
        let config = Config::new("https://auth.example.com")
            .with_client_credentials("client-id", "client-secret")
            .with_audience("test-audience")
            .with_scope("api:read")
            .with_timeout(Duration::from_secs(45))
            .with_jwks_ttl(Duration::from_secs(1800));

        assert!(config.can_provide());
        assert!(config.can_verify());
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.timeout, Some(Duration::from_secs(45)));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800)));
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::combined(
            "https://auth.example.com",
            "client-id",
            "client-secret",
            "audience",
        )
        .with_scope("api:read");

        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: Config = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_server_layer_creation() {
        // Initialize crypto provider for HTTPS requests
        crate::tls::provider::initialize_crypto_provider();

        let config = Config::verifier("https://auth.example.com", "test-audience");

        let layer: Result<OidcVerifierLayer, AuthError> =
            <Config as ServerAuthenticator<()>>::get_server_layer(&config);
        assert!(layer.is_ok(), "Should be able to create server layer");
    }

    #[test]
    fn test_server_layer_creation_fails_without_audience() {
        let config = Config::provider("client-id", "client-secret", "https://auth.example.com");

        let layer: Result<OidcVerifierLayer, AuthError> =
            <Config as ServerAuthenticator<()>>::get_server_layer(&config);
        assert!(
            layer.is_err(),
            "Should fail to create server layer without audience"
        );
    }

    #[test]
    fn test_client_layer_creation_fails_sync() {
        let config = Config::provider("client-id", "client-secret", "https://auth.example.com");

        let layer = config.get_client_layer();
        assert!(
            layer.is_err(),
            "Should fail to create client layer synchronously"
        );
    }

    #[test]
    fn test_client_layer_creation_fails_without_credentials() {
        let config = Config::verifier("https://auth.example.com", "test-audience");

        let layer = config.get_client_layer();
        assert!(
            layer.is_err(),
            "Should fail to create client layer without credentials"
        );
    }

    #[test]
    fn test_can_provide_and_verify_methods() {
        let provider_only = Config::provider("id", "secret", "https://auth.example.com");
        assert!(provider_only.can_provide());
        assert!(!provider_only.can_verify());

        let verifier_only = Config::verifier("https://auth.example.com", "audience");
        assert!(!verifier_only.can_provide());
        assert!(verifier_only.can_verify());

        let combined = Config::combined("https://auth.example.com", "id", "secret", "audience");
        assert!(combined.can_provide());
        assert!(combined.can_verify());
    }
}

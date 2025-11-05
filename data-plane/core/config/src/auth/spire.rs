// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#![cfg(not(target_family = "windows"))]

//! Unified SPIRE authentication configuration for SLIM.
//!
//! This module exposes a single configuration struct `SpireConfig` that can be
//! used to build a unified `SpireIdentityManager` for both providing and
//! verifying SPIFFE identities (X.509 & JWT SVIDs).
//!
//! Features:
//! - Configure socket path, target SPIFFE ID (for JWT acquisition), audiences,
//!   and optional trust domain override.
//! - Create a provider (`create_provider`) that fetches & rotates X.509 and JWT SVIDs.
//! - Create a verifier (`create_jwt_verifier`) that focuses on verification; target
//!   SPIFFE ID is cleared automatically.
//! - Optional `trust_domain` forces bundle retrieval for a specific trust domain.
//!
//! This configuration is local to the config crate. When constructing runtime SPIRE
//! components it is converted into the auth crate's `slim_auth::spire::SpireConfig`.

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::jwt_middleware::{AddJwtLayer, ValidateJwtLayer};
use slim_auth::spire::SpireIdentityManager;

/// SPIRE authentication configuration
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct SpireConfig {
    /// Path to the SPIFFE Workload API socket (None => use SPIFFE_ENDPOINT_SOCKET env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
    /// Optional target SPIFFE ID when requesting JWT SVIDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_spiffe_id: Option<String>,
    /// Audiences to request / verify for JWT SVIDs
    #[serde(default = "default_audiences")]
    pub jwt_audiences: Vec<String>,
    /// Optional trust domains override for X.509 bundle retrieval. If set,
    /// `get_x509_bundle()` uses this instead of deriving from the current SVID.
    #[serde(default)]
    pub trust_domains: Vec<String>,
}

fn default_audiences() -> Vec<String> {
    vec!["slim".to_string()]
}

impl Default for SpireConfig {
    fn default() -> Self {
        Self {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: default_audiences(),
            trust_domains: Vec::new(),
        }
    }
}

// removed unused AuthSpiffeConfig import

impl SpireConfig {
    /// Create a new SPIFFE configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the socket path
    pub fn with_socket_path(mut self, socket_path: impl Into<String>) -> Self {
        self.socket_path = Some(socket_path.into());
        self
    }

    /// Set the target SPIFFE ID
    pub fn with_target_spiffe_id(mut self, spiffe_id: impl Into<String>) -> Self {
        self.target_spiffe_id = Some(spiffe_id.into());
        self
    }

    /// Set the JWT audiences
    pub fn with_jwt_audiences(mut self, audiences: Vec<String>) -> Self {
        self.jwt_audiences = audiences;
        self
    }

    /// Add a single trust domain override for X.509 bundle retrieval
    pub fn with_trust_domain(mut self, trust_domain: impl Into<String>) -> Self {
        self.trust_domains.push(trust_domain.into());
        self
    }

    /// Replace all trust domains overrides at once
    pub fn with_trust_domains(mut self, trust_domains: Vec<String>) -> Self {
        self.trust_domains = trust_domains;
        self
    }
    // Removed: direct conversion now superseded by builder pattern.

    /// Internal helper to build & initialize a spire identity manager.
    /// If `include_target` is true and `target_spiffe_id` is set, the target is included.
    async fn build_identity_manager(&self) -> Result<SpireIdentityManager, AuthError> {
        let mut builder =
            SpireIdentityManager::builder().with_jwt_audiences(self.jwt_audiences.clone());

        if let Some(ref socket) = self.socket_path {
            builder = builder.with_socket_path(socket.clone());
        }

        if let Some(ref target) = self.target_spiffe_id {
            builder = builder.with_target_spiffe_id(target.clone());
        }

        let mut mgr = builder.build();
        mgr.initialize()
            .await
            .map_err(|e| AuthError::ConfigError(e.to_string()))?;
        Ok(mgr)
    }

    /// Create a spire provider from this configuration using the builder.
    /// Returns an initialized SpireIdentityManager that will rotate X.509 & JWT SVIDs.
    pub async fn create_provider(&self) -> Result<SpireIdentityManager, AuthError> {
        self.build_identity_manager().await
    }

    /// Create a spire verifier (identity manager used only for verification).
    /// The target SPIFFE ID (if configured) is intentionally not set.
    pub async fn create_jwt_verifier(&self) -> Result<SpireIdentityManager, AuthError> {
        self.build_identity_manager().await
    }
}

impl ClientAuthenticator for SpireConfig {
    type ClientLayer = AddJwtLayer<SpireIdentityManager>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        // Creation requires async context due to initialization
        Err(AuthError::ConfigError(
            "spire client layer creation requires async context".to_string(),
        ))
    }
}

impl<Response> ServerAuthenticator<Response> for SpireConfig
where
    Response: Default + Send + Sync + 'static,
{
    type ServerLayer = ValidateJwtLayer<serde_json::Value, SpireIdentityManager>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        Err(AuthError::ConfigError(
            "spire server layer creation requires async context".to_string(),
        ))
    }
}

/// Async helper functions for creating spire authentication layers
impl SpireConfig {
    /// Create a client authentication layer asynchronously
    pub async fn get_client_layer_async(
        &self,
    ) -> Result<AddJwtLayer<SpireIdentityManager>, AuthError> {
        let provider = self.create_provider().await?;
        let duration = 3600; // 1 hour token duration
        Ok(AddJwtLayer::new(provider, duration))
    }

    /// Create a server authentication layer asynchronously
    pub async fn get_server_layer_async<Response>(
        &self,
    ) -> Result<ValidateJwtLayer<serde_json::Value, SpireIdentityManager>, AuthError>
    where
        Response: Default + Send + Sync + 'static,
    {
        let verifier = self.create_jwt_verifier().await?;
        let claims = serde_json::json!({});
        Ok(ValidateJwtLayer::new(verifier, claims))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = SpireConfig::default();
        assert!(config.socket_path.is_none());
        assert!(config.target_spiffe_id.is_none());
        assert_eq!(config.jwt_audiences, vec!["slim"]);
        assert!(config.trust_domains.is_empty());
    }

    #[test]
    fn test_config_builder() {
        let config = SpireConfig::new()
            .with_socket_path("unix:/tmp/spire-agent/public/api.sock")
            .with_target_spiffe_id("spiffe://example.org/slim")
            .with_jwt_audiences(vec!["audience1".to_string(), "audience2".to_string()])
            .with_trust_domain("example.org");

        assert_eq!(
            config.socket_path,
            Some("unix:/tmp/spire-agent/public/api.sock".to_string())
        );
        assert_eq!(
            config.target_spiffe_id,
            Some("spiffe://example.org/slim".to_string())
        );
        assert_eq!(config.jwt_audiences, vec!["audience1", "audience2"]);
        assert_eq!(config.trust_domains, vec!["example.org".to_string()]);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let config = SpireConfig::new()
            .with_socket_path("unix:/tmp/spire-agent/public/api.sock")
            .with_jwt_audiences(vec!["test".to_string()])
            .with_trust_domain("example.org");

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: SpireConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }
}

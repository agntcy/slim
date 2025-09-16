// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SPIFFE authentication configuration for SLIM

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::jwt_middleware::{AddJwtLayer, ValidateJwtLayer};
use slim_auth::spiffe::{SpiffeConfig as AuthSpiffeConfig, SpiffeJwtVerifier, SpiffeProvider};
use std::time::Duration;

/// SPIFFE authentication configuration
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    /// Path to the SPIFFE Workload API socket (optional, defaults to SPIFFE_ENDPOINT_SOCKET env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,

    /// Target SPIFFE ID for JWT tokens (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_spiffe_id: Option<String>,

    /// JWT audiences for token requests
    #[serde(default = "default_audiences")]
    pub jwt_audiences: Vec<String>,

    /// Certificate refresh interval (optional, defaults to 1/3 of certificate lifetime)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub refresh_interval: Option<Duration>,

    /// Enable X.509 SVID certificate authentication
    #[serde(default = "default_true")]
    pub enable_x509: bool,

    /// Enable JWT SVID token authentication
    #[serde(default = "default_true")]
    pub enable_jwt: bool,
}

fn default_audiences() -> Vec<String> {
    vec!["slim".to_string()]
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: default_audiences(),
            refresh_interval: None,
            enable_x509: true,
            enable_jwt: true,
        }
    }
}

impl Config {
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

    /// Set the refresh interval
    pub fn with_refresh_interval(mut self, interval: Duration) -> Self {
        self.refresh_interval = Some(interval);
        self
    }

    /// Enable or disable X.509 SVID authentication
    pub fn with_x509_enabled(mut self, enabled: bool) -> Self {
        self.enable_x509 = enabled;
        self
    }

    /// Enable or disable JWT SVID authentication
    pub fn with_jwt_enabled(mut self, enabled: bool) -> Self {
        self.enable_jwt = enabled;
        self
    }

    /// Convert to the auth crate's SpiffeConfig
    pub fn to_auth_config(&self) -> AuthSpiffeConfig {
        AuthSpiffeConfig {
            socket_path: self.socket_path.clone(),
            target_spiffe_id: self.target_spiffe_id.clone(),
            jwt_audiences: self.jwt_audiences.clone(),
            refresh_interval: self.refresh_interval,
        }
    }

    /// Create a SPIFFE provider from this configuration
    pub async fn create_provider(&self) -> Result<SpiffeProvider, AuthError> {
        let auth_config = self.to_auth_config();
        let mut provider = SpiffeProvider::new(auth_config);
        provider.initialize().await?;
        Ok(provider)
    }

    /// Create a SPIFFE JWT verifier from this configuration
    pub async fn create_jwt_verifier(&self) -> Result<SpiffeJwtVerifier, AuthError> {
        let verifier = SpiffeJwtVerifier::new(self.jwt_audiences.clone());
        verifier.initialize().await?;
        Ok(verifier)
    }
}

impl ClientAuthenticator for Config {
    type ClientLayer = AddJwtLayer<SpiffeProvider>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        if !self.enable_jwt {
            return Err(AuthError::ConfigError(
                "JWT authentication is disabled".to_string(),
            ));
        }

        // This is a placeholder - in practice, we'd need to create the provider asynchronously
        // The actual implementation would need to be done in an async context
        Err(AuthError::ConfigError(
            "SPIFFE client layer creation requires async context".to_string(),
        ))
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + Sync + 'static,
{
    type ServerLayer = ValidateJwtLayer<serde_json::Value, SpiffeJwtVerifier>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        if !self.enable_jwt {
            return Err(AuthError::ConfigError(
                "JWT authentication is disabled".to_string(),
            ));
        }

        // This is a placeholder - in practice, we'd need to create the verifier asynchronously
        // The actual implementation would need to be done in an async context
        Err(AuthError::ConfigError(
            "SPIFFE server layer creation requires async context".to_string(),
        ))
    }
}

/// Async helper functions for creating SPIFFE authentication layers
impl Config {
    /// Create a client authentication layer asynchronously
    pub async fn get_client_layer_async(&self) -> Result<AddJwtLayer<SpiffeProvider>, AuthError> {
        if !self.enable_jwt {
            return Err(AuthError::ConfigError(
                "JWT authentication is disabled".to_string(),
            ));
        }

        let provider = self.create_provider().await?;
        // Default token duration of 1 hour
        let duration = 3600;
        Ok(AddJwtLayer::new(provider, duration))
    }

    /// Create a server authentication layer asynchronously
    pub async fn get_server_layer_async<Response>(
        &self,
    ) -> Result<ValidateJwtLayer<serde_json::Value, SpiffeJwtVerifier>, AuthError>
    where
        Response: Default + Send + Sync + 'static,
    {
        if !self.enable_jwt {
            return Err(AuthError::ConfigError(
                "JWT authentication is disabled".to_string(),
            ));
        }

        let verifier = self.create_jwt_verifier().await?;
        let claims = serde_json::json!({}); // Empty default claims
        Ok(ValidateJwtLayer::new(verifier, claims))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.socket_path.is_none());
        assert!(config.target_spiffe_id.is_none());
        assert_eq!(config.jwt_audiences, vec!["slim"]);
        assert!(config.refresh_interval.is_none());
        assert!(config.enable_x509);
        assert!(config.enable_jwt);
    }

    #[test]
    fn test_config_builder() {
        let config = Config::new()
            .with_socket_path("unix:/tmp/spire-agent/public/api.sock")
            .with_target_spiffe_id("spiffe://example.org/slim")
            .with_jwt_audiences(vec!["audience1".to_string(), "audience2".to_string()])
            .with_refresh_interval(Duration::from_secs(300))
            .with_x509_enabled(false)
            .with_jwt_enabled(true);

        assert_eq!(
            config.socket_path,
            Some("unix:/tmp/spire-agent/public/api.sock".to_string())
        );
        assert_eq!(
            config.target_spiffe_id,
            Some("spiffe://example.org/slim".to_string())
        );
        assert_eq!(config.jwt_audiences, vec!["audience1", "audience2"]);
        assert_eq!(config.refresh_interval, Some(Duration::from_secs(300)));
        assert!(!config.enable_x509);
        assert!(config.enable_jwt);
    }

    #[test]
    fn test_to_auth_config() {
        let config = Config::new()
            .with_socket_path("unix:/tmp/spire-agent/public/api.sock")
            .with_jwt_audiences(vec!["test".to_string()]);

        let auth_config = config.to_auth_config();
        assert_eq!(
            auth_config.socket_path,
            Some("unix:/tmp/spire-agent/public/api.sock".to_string())
        );
        assert_eq!(auth_config.jwt_audiences, vec!["test"]);
    }

    #[tokio::test]
    async fn test_config_serialization() {
        let config = Config::new()
            .with_socket_path("unix:/tmp/spire-agent/public/api.sock")
            .with_jwt_audiences(vec!["test".to_string()]);

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SPIFFE integration for SLIM authentication
//! This module provides direct integration with SPIFFE Workload API to retrieve
//! X.509 SVID certificates and JWT tokens.

use async_trait::async_trait;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

use spiffe::{JwtBundleSet, JwtSvid, SvidSource, WorkloadApiClient, X509Source, X509Svid};

use crate::errors::AuthError;
use crate::traits::{TokenProvider, Verifier};

/// Configuration for SPIFFE authentication
#[derive(Debug, Clone)]
pub struct SpiffeConfig {
    /// Path to the SPIFFE Workload API socket
    pub socket_path: Option<String>,
    /// Target SPIFFE ID for JWT tokens (optional)
    pub target_spiffe_id: Option<String>,
    /// JWT audiences for token requests
    pub jwt_audiences: Vec<String>,
    /// Certificate refresh interval (defaults to 1/3 of certificate lifetime)
    pub refresh_interval: Option<Duration>,
}

impl Default for SpiffeConfig {
    fn default() -> Self {
        Self {
            socket_path: None, // Will use SPIFFE_ENDPOINT_SOCKET env var
            target_spiffe_id: None,
            jwt_audiences: vec!["slim".to_string()],
            refresh_interval: None,
        }
    }
}

/// SPIFFE certificate and JWT provider that automatically rotates credentials
#[derive(Clone)]
pub struct SpiffeProvider {
    config: SpiffeConfig,
    x509_source: Arc<RwLock<Option<Arc<X509Source>>>>,
    client: Arc<RwLock<Option<WorkloadApiClient>>>,
}

impl SpiffeProvider {
    /// Create a new SpiffeProvider with the given configuration
    pub fn new(config: SpiffeConfig) -> Self {
        Self {
            config,
            x509_source: Arc::new(RwLock::new(None)),
            client: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the SPIFFE provider and start credential rotation
    pub async fn initialize(&self) -> Result<(), AuthError> {
        info!("Initializing SPIFFE provider");

        // Create WorkloadApiClient
        let client = if let Some(socket_path) = &self.config.socket_path {
            debug!("Connecting to SPIFFE Workload API at: {}", socket_path);
            WorkloadApiClient::new_from_path(socket_path)
                .await
                .map_err(|e| {
                    AuthError::ConfigError(format!(
                        "Failed to connect to SPIFFE Workload API: {}",
                        e
                    ))
                })?
        } else {
            debug!(
                "Connecting to SPIFFE Workload API using SPIFFE_ENDPOINT_SOCKET environment variable"
            );
            WorkloadApiClient::default().await.map_err(|e| {
                AuthError::ConfigError(format!("Failed to connect to SPIFFE Workload API: {}", e))
            })?
        };

        // Store the client
        {
            let mut client_guard = self.client.write().await;
            *client_guard = Some(client);
        }

        // Initialize X509Source for certificate management
        let x509_source = X509Source::default().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize X509Source: {}", e))
        })?;

        // Store the source
        {
            let mut x509_guard = self.x509_source.write().await;
            *x509_guard = Some(x509_source);
        }

        info!("SPIFFE provider initialized successfully");
        Ok(())
    }

    /// Get the current X.509 SVID certificate
    pub async fn get_x509_svid(&self) -> Result<X509Svid, AuthError> {
        let x509_source_guard = self.x509_source.read().await;
        let x509_source = x509_source_guard
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("X509Source not initialized".to_string()))?;

        let svid = x509_source
            .get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get X509 SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No X509 SVID available".to_string()))?;

        debug!("Retrieved X509 SVID with SPIFFE ID: {}", svid.spiffe_id());
        Ok(svid)
    }

    /// Get a JWT SVID token using the WorkloadApiClient
    pub async fn get_jwt_svid(&self) -> Result<JwtSvid, AuthError> {
        let mut client_guard = self.client.write().await;
        let client_ref = client_guard.as_mut().ok_or_else(|| {
            AuthError::ConfigError("WorkloadApiClient not initialized".to_string())
        })?;

        // Parse target SPIFFE ID if provided
        let target_spiffe_id = if let Some(spiffe_id_str) = &self.config.target_spiffe_id {
            Some(
                spiffe_id_str
                    .parse()
                    .map_err(|e| AuthError::ConfigError(format!("Invalid SPIFFE ID: {}", e)))?,
            )
        } else {
            None
        };

        let jwt_svid = client_ref
            .fetch_jwt_svid(&self.config.jwt_audiences, target_spiffe_id.as_ref())
            .await
            .map_err(|e| AuthError::ConfigError(format!("Failed to fetch JWT SVID: {}", e)))?;

        debug!(
            "Retrieved JWT SVID with SPIFFE ID: {}, audiences: {:?}",
            jwt_svid.spiffe_id(),
            self.config.jwt_audiences
        );
        Ok(jwt_svid)
    }

    /// Get the X.509 certificate in PEM format
    pub async fn get_x509_cert_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid().await?;
        let cert_chain = svid.cert_chain();

        if cert_chain.is_empty() {
            return Err(AuthError::ConfigError(
                "Empty certificate chain".to_string(),
            ));
        }

        // Convert the first certificate to PEM format
        let cert_der = &cert_chain[0];
        let engine = base64::engine::general_purpose::STANDARD;
        let cert_pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
            engine.encode(cert_der.as_ref())
        );

        Ok(cert_pem)
    }

    /// Get the X.509 private key in PEM format
    pub async fn get_x509_key_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid().await?;
        let private_key = svid.private_key();

        // Convert private key to PEM format
        let engine = base64::engine::general_purpose::STANDARD;
        let key_pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            engine.encode(private_key.as_ref())
        );

        Ok(key_pem)
    }

    /// Get the JWT token as a string
    pub async fn get_jwt_token_string(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid().await?;
        Ok(jwt_svid.token().to_string())
    }

    /// Check if the provider is initialized
    pub async fn is_initialized(&self) -> bool {
        let x509_guard = self.x509_source.read().await;
        let client_guard = self.client.read().await;
        x509_guard.is_some() && client_guard.is_some()
    }

    /// Start the credential rotation background task
    pub async fn start_rotation_task(&self) -> Result<(), AuthError> {
        if !self.is_initialized().await {
            return Err(AuthError::ConfigError(
                "Provider not initialized".to_string(),
            ));
        }

        // The X509Source handles rotation automatically
        // We just need to log when credentials are updated
        info!("SPIFFE credential rotation is handled automatically by X509Source");
        Ok(())
    }
}

impl TokenProvider for SpiffeProvider {
    fn get_token(&self) -> Result<String, AuthError> {
        // Since TokenProvider is sync but we need async operations,
        // we'll return an error suggesting to use the async method
        Err(AuthError::ConfigError(
            "Use get_jwt_token_string() async method instead".to_string(),
        ))
    }
}

/// SPIFFE JWT Verifier that uses the JWT bundles from SPIFFE Workload API
#[derive(Clone)]
pub struct SpiffeJwtVerifier {
    client: Arc<RwLock<Option<WorkloadApiClient>>>,
    audiences: Vec<String>,
}

impl SpiffeJwtVerifier {
    /// Create a new SPIFFE JWT verifier
    pub fn new(audiences: Vec<String>) -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            audiences,
        }
    }

    /// Initialize the verifier with a WorkloadApiClient
    pub async fn initialize(&self) -> Result<(), AuthError> {
        let client = WorkloadApiClient::default().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize WorkloadApiClient: {}", e))
        })?;

        let mut guard = self.client.write().await;
        *guard = Some(client);
        Ok(())
    }

    /// Get JWT bundles for validation
    async fn get_jwt_bundles(&self) -> Result<JwtBundleSet, AuthError> {
        let mut client_guard = self.client.write().await;
        let client_ref = client_guard.as_mut().ok_or_else(|| {
            AuthError::ConfigError("WorkloadApiClient not initialized".to_string())
        })?;

        client_ref
            .fetch_jwt_bundles()
            .await
            .map_err(|e| AuthError::ConfigError(format!("Failed to fetch JWT bundles: {}", e)))
    }
}

#[async_trait]
impl Verifier for SpiffeJwtVerifier {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        let token_str = token.into();
        let jwt_bundles = self.get_jwt_bundles().await?;

        // Parse and validate the JWT token
        JwtSvid::parse_and_validate(&token_str, &jwt_bundles, &self.audiences)
            .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;

        debug!("Successfully verified JWT token");
        Ok(())
    }

    fn try_verify(&self, _token: impl Into<String>) -> Result<(), AuthError> {
        // SPIFFE verification requires async operations, so this is not supported
        Err(AuthError::ConfigError(
            "SPIFFE JWT verification requires async context. Use verify() instead".to_string(),
        ))
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        let token_str = token.into();
        let jwt_bundles = self.get_jwt_bundles().await?;

        // Parse and validate the JWT token
        let jwt_svid = JwtSvid::parse_and_validate(&token_str, &jwt_bundles, &self.audiences)
            .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;

        debug!(
            "Successfully verified JWT token for SPIFFE ID: {}",
            jwt_svid.spiffe_id()
        );

        // Create a simple claims structure from available JwtSvid methods
        let claims_json = serde_json::json!({
            "sub": jwt_svid.spiffe_id().to_string(),
            "aud": jwt_svid.audience().clone(),
            "exp": jwt_svid.expiry().to_string(),
        });

        // Deserialize the claims
        serde_json::from_value(claims_json)
            .map_err(|e| AuthError::ConfigError(format!("Failed to deserialize JWT claims: {}", e)))
    }

    fn try_get_claims<Claims>(&self, _token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        // SPIFFE verification requires async operations, so this is not supported
        Err(AuthError::ConfigError(
            "SPIFFE JWT verification requires async context. Use get_claims() instead".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spiffe_config_default() {
        let config = SpiffeConfig::default();
        assert!(config.socket_path.is_none());
        assert!(config.target_spiffe_id.is_none());
        assert_eq!(config.jwt_audiences, vec!["slim"]);
        assert!(config.refresh_interval.is_none());
    }

    #[tokio::test]
    async fn test_spiffe_provider_creation() {
        let config = SpiffeConfig::default();
        let provider = SpiffeProvider::new(config);
        assert!(!provider.is_initialized().await);
    }

    #[tokio::test]
    async fn test_spiffe_jwt_verifier_creation() {
        let verifier = SpiffeJwtVerifier::new(vec!["test-audience".to_string()]);
        assert_eq!(verifier.audiences, vec!["test-audience"]);
    }
}

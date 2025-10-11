// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! SPIFFE integration for SLIM authentication
//! This module provides direct integration with SPIFFE Workload API to retrieve
//! X.509 SVID certificates and JWT tokens.

use crate::errors::AuthError;
use crate::traits::{TokenProvider, Verifier};
use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt; // for .next() on the JWT bundle stream
use parking_lot::RwLock; // switched to parking_lot for sync RwLock
use serde::de::DeserializeOwned;
use serde_json;
use spiffe::{JwtBundleSet, JwtSvid, SvidSource, WorkloadApiClient, X509Source, X509Svid};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep; // for backoff between reconnect attempts
use tracing::{debug, info}; // for sync access in TokenProvider impl

/// Configuration for SPIFFE authentication
#[derive(Debug, Clone)]
pub struct SpiffeProviderConfig {
    /// Path to the SPIFFE Workload API socket
    pub socket_path: Option<String>,
    /// Target SPIFFE ID for JWT tokens (optional)
    pub target_spiffe_id: Option<String>,
    /// JWT audiences for token requests
    pub jwt_audiences: Vec<String>,
    /// Certificate refresh interval (defaults to 1/3 of certificate lifetime)
    pub refresh_interval: Option<Duration>,
}

impl Default for SpiffeProviderConfig {
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
    config: SpiffeProviderConfig,
    x509_source: Option<Arc<X509Source>>,
    jwt_source: Option<Arc<JwtSource>>,
    client: Arc<RwLock<Option<WorkloadApiClient>>>,
}

impl SpiffeProvider {
    /// Create a new SpiffeProvider with the given configuration
    pub fn new(config: SpiffeProviderConfig) -> Self {
        Self {
            config,
            x509_source: None,
            jwt_source: None,
            client: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the SPIFFE provider and start credential rotation
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
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
            let mut client_guard = self.client.write();
            *client_guard = Some(client);
        }

        // Initialize X509Source for certificate management
        let x509_source = X509Source::default().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize X509Source: {}", e))
        })?;

        self.x509_source = Some(x509_source);

        // Initialize JwtSource (background refresh) so we can offer a sync token provider
        let jwt_source = JwtSource::new(
            self.config.jwt_audiences.clone(),
            self.config.target_spiffe_id.clone(),
            self.config.socket_path.clone(),
            self.config.refresh_interval, // reuse refresh interval if provided
        )
        .await
        .map_err(|e| AuthError::ConfigError(format!("Failed to initialize JwtSource: {}", e)))?;
        self.jwt_source = Some(jwt_source);
        info!("SPIFFE provider initialized successfully");
        Ok(())
    }

    /// Get the current X.509 SVID certificate
    pub fn get_x509_svid(&self) -> Result<X509Svid, AuthError> {
        let x509_source_guard = &self.x509_source;
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

    /// Get the X.509 certificate in PEM format
    pub fn get_x509_cert_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid()?;
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
    pub fn get_x509_key_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid()?;
        let private_key = svid.private_key();

        // Convert private key to PEM format
        let engine = base64::engine::general_purpose::STANDARD;
        let key_pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            engine.encode(private_key.as_ref())
        );

        Ok(key_pem)
    }

    /// Get a cached JWT SVID via the background-refreshing JwtSource (sync)
    pub fn get_jwt_svid(&self) -> Result<JwtSvid, AuthError> {
        let src = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;
        src.get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get JWT SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No JWT SVID available".to_string()))
    }
}

impl TokenProvider for SpiffeProvider {
    fn get_token(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid()?;
        Ok(jwt_svid.token().to_string())
    }
}

// JwtSource: background-refreshing source of JWT SVIDs modeled after X509Source APIs
struct JwtSourceConfigInternal {
    refresh_interval: Option<Duration>,
    refresh_buffer: Duration, // time before expiry to refresh if expiry can be determined
    min_retry_backoff: Duration,
    max_retry_backoff: Duration,
}

impl Default for JwtSourceConfigInternal {
    fn default() -> Self {
        Self {
            refresh_interval: None,
            refresh_buffer: Duration::from_secs(10),
            min_retry_backoff: Duration::from_secs(1),
            max_retry_backoff: Duration::from_secs(30),
        }
    }
}

/// A background-refreshing source of JWT SVIDs providing a sync `get_svid()` similar to `X509Source`.
pub struct JwtSource {
    _audiences: Vec<String>,
    _target_spiffe_id: Option<String>,
    current: Arc<RwLock<Option<JwtSvid>>>,
    _task_handle: tokio::task::JoinHandle<()>, // kept for lifecycle (drop cancels)
}

impl JwtSource {
    pub async fn new(
        _audiences: Vec<String>,
        _target_spiffe_id: Option<String>,
        socket_path: Option<String>,
        refresh_interval: Option<Duration>,
    ) -> Result<Arc<Self>, AuthError> {
        let cfg = JwtSourceConfigInternal {
            refresh_interval,
            ..Default::default()
        };

        // Create a dedicated client (separate from provider's client so we can own mutably in task)
        let client_res = if let Some(path) = socket_path {
            WorkloadApiClient::new_from_path(&path).await
        } else {
            WorkloadApiClient::default().await
        };
        let mut client = client_res.map_err(|e| {
            AuthError::ConfigError(format!(
                "Failed to create WorkloadApiClient for JwtSource: {}",
                e
            ))
        })?;

        let current = Arc::new(RwLock::new(None));
        let current_clone = current.clone();
        let audiences_clone = _audiences.clone();
        let target_clone = _target_spiffe_id.clone();

        let task_handle = tokio::spawn(async move {
            let mut backoff = cfg.min_retry_backoff;
            loop {
                // Fetch
                match fetch_once(&mut client, &audiences_clone, target_clone.as_ref()).await {
                    Ok(svid) => {
                        // Store
                        {
                            let mut w = current_clone.write();
                            *w = Some(svid.clone());
                        }
                        // Reset backoff on success
                        backoff = cfg.min_retry_backoff;

                        // Determine next refresh
                        let sleep_dur = cfg
                            .refresh_interval
                            .unwrap_or_else(|| guess_sleep_until_expiry(&svid, cfg.refresh_buffer));
                        tracing::debug!(?sleep_dur, "jwt_source: sleeping until next refresh");
                        tokio::time::sleep(sleep_dur).await;
                    }
                    Err(err) => {
                        tracing::warn!(error=%err, "jwt_source: failed to fetch JWT SVID; backing off");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(cfg.max_retry_backoff);
                    }
                }
            }
        });

        Ok(Arc::new(Self {
            _audiences,
            _target_spiffe_id,
            current,
            _task_handle: task_handle,
        }))
    }

    /// Sync access to the current JWT SVID (if any). Returns Ok(Some) if present.
    pub fn get_svid(&self) -> Result<Option<JwtSvid>, AuthError> {
        // Use try_read for non-blocking sync access
        let guard = self.current.read();
        Ok(guard.clone())
    }
}

// Helper: single fetch operation
async fn fetch_once(
    client: &mut WorkloadApiClient,
    audiences: &[String],
    target_spiffe_id: Option<&String>,
) -> Result<JwtSvid, AuthError> {
    let parsed_target = if let Some(t) = target_spiffe_id {
        Some(
            t.parse()
                .map_err(|e| AuthError::ConfigError(format!("Invalid SPIFFE ID: {}", e)))?,
        )
    } else {
        None
    };
    client
        .fetch_jwt_svid(audiences, parsed_target.as_ref())
        .await
        .map_err(|e| AuthError::ConfigError(format!("Failed to fetch JWT SVID: {}", e)))
}

// Heuristic: attempt to compute sleep duration until expiry - buffer; fallback to 30s
fn guess_sleep_until_expiry(svid: &JwtSvid, buffer: Duration) -> Duration {
    // We don't know the exact type of `expiry()` (string formatting earlier), so use a defensive approach.
    // Try to parse the `expiry().to_string()` as an epoch seconds integer; if that fails default.
    let default = Duration::from_secs(30);
    let expiry_str = svid.expiry().to_string();
    if let Ok(epoch) = expiry_str.parse::<u64>()
        && let Ok(now_secs) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        && epoch > now_secs.as_secs()
    {
        let remaining = Duration::from_secs(epoch - now_secs.as_secs());
        return remaining.saturating_sub(buffer).max(Duration::from_secs(5));
    }
    default
}

#[derive(Clone)]
pub struct SpiffeVerifierConfig {
    /// Path to the SPIFFE Workload API socket
    pub socket_path: Option<String>,
    /// JWT audiences expected in tokens
    pub jwt_audiences: Vec<String>,
}

/// SPIFFE JWT Verifier that uses the JWT bundles from SPIFFE Workload API
#[derive(Clone)]
pub struct SpiffeJwtVerifier {
    config: SpiffeVerifierConfig,
    client: Arc<RwLock<Option<WorkloadApiClient>>>,
    bundles: Arc<RwLock<Option<JwtBundleSet>>>,
}

impl SpiffeJwtVerifier {
    /// Create a new SPIFFE JWT verifier
    pub fn new(config: SpiffeVerifierConfig) -> Self {
        Self {
            config,
            client: Arc::new(RwLock::new(None)),
            bundles: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the verifier with a WorkloadApiClient
    pub async fn initialize(&self) -> Result<(), AuthError> {
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

        let mut guard = self.client.write();
        *guard = Some(client);
        drop(guard); // release lock before spawning background task

        // Start background task that maintains an in-memory cache of JWT bundles using the streaming API.
        let bundles_cache = self.bundles.clone();
        tokio::spawn(async move {
            // Exponential backoff parameters
            let mut backoff_secs: u64 = 1;
            const MAX_BACKOFF: u64 = 30;
            loop {
                // Create a fresh client each attempt (avoids mut borrow conflicts with other operations)
                match WorkloadApiClient::default().await {
                    Ok(mut streaming_client) => {
                        tracing::debug!("spiffe_jwt: starting JWT bundle stream");
                        match streaming_client.stream_jwt_bundles().await {
                            Ok(mut stream) => {
                                backoff_secs = 1; // reset backoff after successful connection
                                while let Some(next_item) = stream.next().await {
                                    match next_item {
                                        Ok(update) => {
                                            // simple sanity check
                                            let mut w = bundles_cache.write();
                                            *w = Some(update.clone());
                                            tracing::trace!(
                                                "spiffe_jwt: updated in-memory JWT bundle set"
                                            );
                                        }
                                        Err(err) => {
                                            tracing::warn!(error=%err, "spiffe_jwt: stream item error; restarting stream");
                                            break; // break inner while to restart outer loop
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                tracing::warn!(error=%err, "spiffe_jwt: failed to obtain stream; will retry");
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error=%err, "spiffe_jwt: failed to create streaming client; will retry");
                    }
                }
                // Backoff before retrying
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
            }
        });
        Ok(())
    }

    /// Get JWT bundles for validation
    async fn get_jwt_bundles(&self) -> Result<JwtBundleSet, AuthError> {
        // Fast path: cached bundles present
        if let Some(cached) = { self.bundles.read().clone() } {
            return Ok(cached);
        }

        // Slow path: attempt a one-shot stream fetch to seed cache
        let client_opt = { self.client.read().clone() };
        let mut client = client_opt.ok_or_else(|| {
            AuthError::ConfigError("WorkloadApiClient not initialized".to_string())
        })?;

        let mut stream = client.stream_jwt_bundles().await.map_err(|e| {
            AuthError::ConfigError(format!(
                "Unable to start JWT bundle stream for on-demand retrieval: {}",
                e
            ))
        })?;

        if let Some(first) = stream.next().await {
            match first {
                Ok(update) => {
                    {
                        let mut w = self.bundles.write();
                        *w = Some(update.clone());
                    }
                    return Ok(update);
                }
                Err(e) => {
                    return Err(AuthError::ConfigError(format!(
                        "Error reading first JWT bundle update: {}",
                        e
                    )));
                }
            }
        }

        Err(AuthError::ConfigError(
            "JWT bundles unavailable and no update received from on-demand stream".to_string(),
        ))
    }
}

#[async_trait]
impl Verifier for SpiffeJwtVerifier {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        let token_str = token.into();
        let bundles = self.get_jwt_bundles().await?;
        JwtSvid::parse_and_validate(&token_str, &bundles, &self.config.jwt_audiences)
            .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;
        debug!("Successfully verified JWT token");
        Ok(())
    }

    fn try_verify(&self, _token: impl Into<String>) -> Result<(), AuthError> {
        let bundles = self.bundles.read().clone();
        match bundles {
            Some(bundles) => {
                JwtSvid::parse_and_validate(&_token.into(), &bundles, &self.config.jwt_audiences)
                    .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;
                debug!("Successfully verified JWT token");
                Ok(())
            }
            None => Err(AuthError::ConfigError(
                "No JWT bundles cached; cannot verify token".to_string(),
            )),
        }
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        let token_str = token.into();
        let bundles = self.get_jwt_bundles().await?;
        let jwt_svid =
            JwtSvid::parse_and_validate(&token_str, &bundles, &self.config.jwt_audiences)
                .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;

        debug!(
            "Successfully extracted claims for SPIFFE ID: {}",
            jwt_svid.spiffe_id()
        );

        let claims_json = serde_json::json!({
            "sub": jwt_svid.spiffe_id().to_string(),
            "aud": jwt_svid.audience().clone(),
            "exp": jwt_svid.expiry().to_string(),
        });

        serde_json::from_value(claims_json)
            .map_err(|e| AuthError::ConfigError(format!("Failed to deserialize JWT claims: {}", e)))
    }

    fn try_get_claims<Claims>(&self, _token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        let bundles = self.bundles.read().clone();
        match bundles {
            Some(bundles) => {
                let jwt_svid = JwtSvid::parse_and_validate(
                    &_token.into(),
                    &bundles,
                    &self.config.jwt_audiences,
                )
                .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;
                debug!("Successfully verified JWT token");
                let claims_json = serde_json::json!({
                    "sub": jwt_svid.spiffe_id().to_string(),
                    "aud": jwt_svid.audience().clone(),
                    "exp": jwt_svid.expiry().to_string(),
                });
                serde_json::from_value(claims_json).map_err(|e| {
                    AuthError::ConfigError(format!("Failed to deserialize JWT claims: {}", e))
                })
            }
            None => Err(AuthError::ConfigError(
                "SPIFFE JWT claims retrieval requires async context. Use get_claims() instead"
                    .to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    #[tokio::test]
    async fn test_spiffe_config_default() {
        let config = SpiffeProviderConfig::default();
        assert!(config.socket_path.is_none());
        assert!(config.target_spiffe_id.is_none());
        assert_eq!(config.jwt_audiences, vec!["slim"]);
        assert!(config.refresh_interval.is_none());
    }

    #[tokio::test]
    async fn test_spiffe_jwt_verifier_creation() {
        let verifier_config = SpiffeVerifierConfig {
            socket_path: Some("unix:///tmp/fake.sock".to_string()),
            jwt_audiences: vec!["test-audience".to_string()],
        };
        let verifier = SpiffeJwtVerifier::new(verifier_config);
        assert_eq!(verifier.config.jwt_audiences, vec!["test-audience"]);
    }

    #[tokio::test]
    async fn test_spiffe_provider_creation() {
        let config = SpiffeProviderConfig::default();
        let provider = SpiffeProvider::new(config);
        assert!(provider.x509_source.is_none());
        assert!(provider.jwt_source.is_none());
    }

    #[test]
    fn test_spiffe_provider_get_x509_svid_not_initialized() {
        let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
        let res = provider.get_x509_svid();
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("X509Source not initialized"));
    }

    #[test]
    fn test_spiffe_provider_get_jwt_svid_not_initialized() {
        let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
        let res = provider.get_jwt_svid();
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("JwtSource not initialized"));
    }

    #[tokio::test]
    async fn test_jwt_source_creation_with_invalid_path_fails() {
        // Use a bogus socket path; current implementation surfaces the error immediately
        let bogus_path = Some("/tmp/non-existent-spiffe-socket".to_string());
        let src = JwtSource::new(
            vec!["aud".into()],
            None,
            bogus_path,
            Some(StdDuration::from_secs(1)),
        )
        .await;
        assert!(
            src.is_err(),
            "Expected JwtSource::new to fail with invalid socket path"
        );
    }

    #[tokio::test]
    async fn test_spiffe_jwt_verifier_try_verify_without_bundles() {
        let spiffe_config = SpiffeVerifierConfig {
            socket_path: None,
            jwt_audiences: vec!["aud".into()],
        };
        let verifier = SpiffeJwtVerifier::new(spiffe_config);
        let res = verifier.try_verify("token".to_string());
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("No JWT bundles cached"));
    }

    #[tokio::test]
    async fn test_spiffe_jwt_verifier_try_get_claims_without_bundles() {
        let spiffe_config = SpiffeVerifierConfig {
            socket_path: None,
            jwt_audiences: vec!["aud".into()],
        };
        let verifier = SpiffeJwtVerifier::new(spiffe_config);
        let claims_result: Result<serde_json::Value, AuthError> =
            verifier.try_get_claims("token".to_string());
        assert!(claims_result.is_err());
        let err = format!("{}", claims_result.unwrap_err());
        assert!(err.contains("SPIFFE JWT claims retrieval requires async context"));
    }
}

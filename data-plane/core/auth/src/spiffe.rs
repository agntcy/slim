// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#![cfg(not(target_family = "windows"))]

//! SPIFFE integration for SLIM authentication
//! This module provides direct integration with SPIFFE Workload API to retrieve
//! X.509 SVID certificates and JWT tokens.

use crate::errors::AuthError;
use crate::traits::{TokenProvider, Verifier};
use crate::utils::bytes_to_pem;
use async_trait::async_trait;
use futures::StreamExt; // for .next() on the JWT bundle stream
use parking_lot::RwLock; // switched to parking_lot for sync RwLock
use serde::de::DeserializeOwned;
use serde_json;
use spiffe::{
    JwtBundleSet, JwtSvid, SvidSource, WorkloadApiClient, X509Source, X509SourceBuilder, X509Svid,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
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
}

impl Default for SpiffeProviderConfig {
    fn default() -> Self {
        Self {
            socket_path: None, // Will use SPIFFE_ENDPOINT_SOCKET env var
            target_spiffe_id: None,
            jwt_audiences: vec!["slim".to_string()],
        }
    }
}

/// SPIFFE certificate and JWT provider that automatically rotates credentials
#[derive(Clone)]
pub struct SpiffeProvider {
    config: SpiffeProviderConfig,
    client: Option<WorkloadApiClient>,
    x509_source: Option<Arc<X509Source>>,
    jwt_source: Option<Arc<JwtSource>>,
}

impl SpiffeProvider {
    /// Create a new SpiffeProvider with the given configuration
    pub fn new(config: SpiffeProviderConfig) -> Self {
        Self {
            config,
            client: None,
            x509_source: None,
            jwt_source: None,
        }
    }

    /// Helper method to create a WorkloadApiClient based on configuration
    async fn create_workload_client(
        socket_path: Option<&String>,
    ) -> Result<WorkloadApiClient, AuthError> {
        if let Some(path) = socket_path {
            WorkloadApiClient::new_from_path(path).await.map_err(|e| {
                AuthError::ConfigError(format!("Failed to connect to SPIFFE Workload API: {}", e))
            })
        } else {
            WorkloadApiClient::default().await.map_err(|e| {
                AuthError::ConfigError(format!("Failed to connect to SPIFFE Workload API: {}", e))
            })
        }
    }

    /// Initialize the SPIFFE provider and start credential rotation
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        info!("Initializing SPIFFE provider");

        // Create WorkloadApiClient
        let client = Self::create_workload_client(self.config.socket_path.as_ref()).await?;

        // Initialize X509Source for certificate management
        let x509_source = X509SourceBuilder::new()
            .with_client(client.clone())
            .build()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to initialize X509Source: {}", e))
            })?;

        self.x509_source = Some(x509_source);

        // Initialize JwtSource for JWT token management
        let mut jwt_builder = JwtSourceBuilder::new()
            .with_audiences(self.config.jwt_audiences.clone())
            .with_client(client.clone());

        if let Some(ref target_id) = self.config.target_spiffe_id {
            jwt_builder = jwt_builder.with_target_spiffe_id(target_id.clone());
        }

        let jwt_source = jwt_builder.build().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize JwtSource: {}", e))
        })?;

        self.jwt_source = Some(jwt_source);

        info!("SPIFFE provider initialized successfully");

        self.client = Some(client);

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

        // Convert the first certificate to PEM format using shared utility
        let cert_der = &cert_chain[0];
        Ok(bytes_to_pem(
            cert_der.as_ref(),
            "-----BEGIN CERTIFICATE-----\n",
            "\n-----END CERTIFICATE-----",
        ))
    }

    /// Get the X.509 private key in PEM format
    pub fn get_x509_key_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid()?;
        let private_key = svid.private_key();

        // Convert private key to PEM format using shared utility
        Ok(bytes_to_pem(
            private_key.as_ref(),
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----",
        ))
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

#[async_trait]
impl TokenProvider for SpiffeProvider {
    fn get_token(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid()?;
        Ok(jwt_svid.token().to_string())
    }

    async fn get_token_with_claims(
        &self,
        custom_claims: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, AuthError> {
        if custom_claims.is_empty() {
            return self.get_token();
        }

        // Encode custom claims as base64 JSON
        let claims_json = serde_json::to_string(&custom_claims).map_err(|e| {
            AuthError::ConfigError(format!("Failed to serialize custom claims: {}", e))
        })?;

        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        let claims_b64 = BASE64.encode(claims_json.as_bytes());

        // Create a special audience with the custom claims
        let claims_audience = format!("slim-claims:{}", claims_b64);

        // Build audiences list with custom claims audience
        let mut audiences = self.config.jwt_audiences.clone();
        audiences.push(claims_audience);

        // Get the jwt_source
        let jwt_source = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;

        jwt_source
            .fetch_with_custom_audiences(audiences, self.config.target_spiffe_id.clone())
            .await
            .map(|svid| svid.token().to_string())
    }

    fn get_id(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid()?;
        Ok(jwt_svid.spiffe_id().to_string())
    }
}

// JwtSource: background-refreshing source of JWT SVIDs modeled after X509Source APIs
struct JwtSourceConfigInternal {
    min_retry_backoff: Duration,
    max_retry_backoff: Duration,
}

/// Request to fetch JWT with custom audiences
struct CustomAudienceRequest {
    audiences: Vec<String>,
    target_spiffe_id: Option<String>,
    response_tx: oneshot::Sender<Result<JwtSvid, AuthError>>,
}

impl Default for JwtSourceConfigInternal {
    fn default() -> Self {
        Self {
            min_retry_backoff: Duration::from_secs(1),
            max_retry_backoff: Duration::from_secs(30),
        }
    }
}

/// A background-refreshing source of JWT SVIDs providing a sync `get_svid()` similar to `X509Source`.
/// Builder for creating a JwtSource
struct JwtSourceBuilder {
    audiences: Vec<String>,
    target_spiffe_id: Option<String>,
    client: Option<WorkloadApiClient>,
}

impl JwtSourceBuilder {
    /// Create a new JwtSourceBuilder with default values
    pub fn new() -> Self {
        Self {
            audiences: Vec::new(),
            target_spiffe_id: None,
            client: None,
        }
    }

    /// Set the JWT audiences
    pub fn with_audiences(mut self, audiences: Vec<String>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Set the target SPIFFE ID
    pub fn with_target_spiffe_id(mut self, target_spiffe_id: String) -> Self {
        self.target_spiffe_id = Some(target_spiffe_id);
        self
    }

    /// Set the WorkloadApiClient
    pub fn with_client(mut self, client: WorkloadApiClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Build and initialize the JwtSource
    pub async fn build(self) -> Result<Arc<JwtSource>, AuthError> {
        JwtSource::new(self.audiences, self.target_spiffe_id, self.client).await
    }
}

impl Default for JwtSourceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct JwtSource {
    _audiences: Vec<String>,
    _target_spiffe_id: Option<String>,
    current: Arc<RwLock<Option<JwtSvid>>>,
    cancellation_token: CancellationToken,
    custom_request_tx: mpsc::Sender<CustomAudienceRequest>,
}

impl JwtSource {
    pub async fn new(
        audiences: Vec<String>,
        target_spiffe_id: Option<String>,
        client: Option<WorkloadApiClient>,
    ) -> Result<Arc<Self>, AuthError> {
        let cfg = JwtSourceConfigInternal::default();

        let current = Arc::new(RwLock::new(None));
        let current_clone = current.clone();
        let audiences_clone = audiences.clone();
        let target_clone = target_spiffe_id.clone();
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        // Get an initial JWT SVID
        let mut client = Self::initialize_client(client).await;

        match fetch_once(&mut client, &audiences_clone, target_clone.as_ref()).await {
            Ok(svid) => {
                let mut w = current.write();
                *w = Some(svid);
            }
            Err(err) => {
                tracing::warn!(error=%err, "jwt_source: initial fetch failed; will retry in background");
            }
        }

        // Create channel for custom audience requests
        let (custom_request_tx, custom_request_rx) = mpsc::channel(16);

        tokio::spawn(async move {
            Self::background_refresh_task(
                client,
                audiences_clone,
                target_clone,
                current_clone,
                token_clone,
                custom_request_rx,
                cfg,
            )
            .await;
        });

        Ok(Arc::new(Self {
            _audiences: audiences,
            _target_spiffe_id: target_spiffe_id,
            current,
            cancellation_token,
            custom_request_tx,
        }))
    }

    /// Background task that handles JWT refresh and custom audience requests
    async fn background_refresh_task(
        mut client: WorkloadApiClient,
        audiences: Vec<String>,
        target_spiffe_id: Option<String>,
        current: Arc<RwLock<Option<JwtSvid>>>,
        cancellation_token: CancellationToken,
        mut custom_request_rx: mpsc::Receiver<CustomAudienceRequest>,
        cfg: JwtSourceConfigInternal,
    ) {
        let mut backoff = cfg.min_retry_backoff;
        let initial_duration = Duration::from_secs(30);
        let mut interval = tokio::time::interval(initial_duration);

        loop {
            tokio::select! {
                // Regular refresh interval
                _ = interval.tick() => {
                    match Self::handle_regular_refresh(
                        &mut client,
                        &audiences,
                        target_spiffe_id.as_ref(),
                        &current,
                        &mut backoff,
                        &cfg,
                        &mut interval,
                    ).await {
                        Ok(()) => {},
                        Err(err) => {
                            tracing::warn!(error=%err, "jwt_source: regular refresh failed");
                        }
                    }
                }

                // Custom audience request
                Some(request) = custom_request_rx.recv() => {
                    Self::handle_custom_request(&mut client, request).await;
                }

                // Cancellation
                _ = cancellation_token.cancelled() => {
                    tracing::debug!("jwt_source: cancellation token signaled, shutting down");
                    break;
                }
            }
        }
    }

    /// Initialize the WorkloadApiClient, retrying if necessary
    async fn initialize_client(client: Option<WorkloadApiClient>) -> WorkloadApiClient {
        if let Some(c) = client {
            return c;
        }

        loop {
            match WorkloadApiClient::default().await {
                Ok(client) => return client,
                Err(err) => {
                    tracing::warn!(error=%err, "jwt_source: failed to create WorkloadApiClient; retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Handle regular JWT refresh with default audiences
    async fn handle_regular_refresh(
        client: &mut WorkloadApiClient,
        audiences: &[String],
        target_spiffe_id: Option<&String>,
        current: &Arc<RwLock<Option<JwtSvid>>>,
        backoff: &mut Duration,
        cfg: &JwtSourceConfigInternal,
        interval: &mut tokio::time::Interval,
    ) -> Result<(), AuthError> {
        match fetch_once(client, audiences, target_spiffe_id).await {
            Ok(svid) => {
                // Store the new SVID
                {
                    let mut w = current.write();
                    *w = Some(svid.clone());
                }

                // Reset backoff on success
                *backoff = cfg.min_retry_backoff;

                // Calculate next refresh time based on token lifetime
                let next_duration = calculate_refresh_interval(&svid);
                *interval = tokio::time::interval(next_duration);

                tracing::debug!(
                    next_duration_secs = next_duration.as_secs(),
                    "jwt_source: next refresh in {} seconds",
                    next_duration.as_secs()
                );

                Ok(())
            }
            Err(err) => {
                tracing::warn!(error=%err, "jwt_source: failed to fetch JWT SVID; backing off");

                // Apply exponential backoff
                *interval = tokio::time::interval(*backoff);
                *backoff = (*backoff * 2).min(cfg.max_retry_backoff);

                Err(err)
            }
        }
    }

    /// Handle custom audience request
    async fn handle_custom_request(client: &mut WorkloadApiClient, request: CustomAudienceRequest) {
        let result = fetch_once(
            client,
            &request.audiences,
            request.target_spiffe_id.as_ref(),
        )
        .await;

        // Send response back (ignore if receiver dropped)
        let _ = request.response_tx.send(result);
    }

    /// Request a JWT with custom audiences
    async fn fetch_with_custom_audiences(
        &self,
        audiences: Vec<String>,
        target_spiffe_id: Option<String>,
    ) -> Result<JwtSvid, AuthError> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = CustomAudienceRequest {
            audiences,
            target_spiffe_id,
            response_tx,
        };

        self.custom_request_tx
            .send(request)
            .await
            .map_err(|_| AuthError::ConfigError("JWT source task has shut down".to_string()))?;

        response_rx.await.map_err(|e| {
            AuthError::SigningError(format!("Failed to receive response from JWT source: {}", e))
        })?
    }

    /// Sync access to the current JWT SVID (if any). Returns Ok(Some) if present.
    fn get_svid(&self) -> Result<Option<JwtSvid>, AuthError> {
        // Use try_read for non-blocking sync access
        let guard = self.current.read();
        Ok(guard.clone())
    }
}

impl Drop for JwtSource {
    fn drop(&mut self) {
        // Cancel the background task when JwtSource is dropped
        self.cancellation_token.cancel();
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

// Calculate refresh interval as 2/3 of the token's lifetime
fn calculate_refresh_interval(svid: &JwtSvid) -> Duration {
    const TWO_THIRDS: f64 = 2.0 / 3.0;
    let default = Duration::from_secs(30);

    let expiry_str = svid.expiry().to_string();
    if let Ok(epoch) = expiry_str.parse::<u64>()
        && let Ok(now_secs) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        && epoch > now_secs.as_secs()
    {
        let total_lifetime = Duration::from_secs(epoch - now_secs.as_secs());
        let refresh_at = Duration::from_secs_f64(total_lifetime.as_secs_f64() * TWO_THIRDS);

        // Use a minimum of 100ms to handle very short-lived tokens (like 1-4 seconds)
        // but still respect the 2/3 lifetime principle
        let min_refresh = Duration::from_millis(100);
        return refresh_at.max(min_refresh);
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

    client: Option<WorkloadApiClient>,
    bundles: Option<JwtBundleSet>,
    cancellation_token: CancellationToken,
}

impl SpiffeJwtVerifier {
    /// Create a new SPIFFE JWT verifier
    pub fn new(config: SpiffeVerifierConfig) -> Self {
        Self {
            config,
            client: None,
            bundles: None,
            cancellation_token: CancellationToken::new(),
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
        let socket_path = self.config.socket_path.clone();
        let cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move {
            // Create a single client for the background task
            let mut streaming_client = loop {
                let client_result = if let Some(ref socket_path) = socket_path {
                    WorkloadApiClient::new_from_path(socket_path).await
                } else {
                    WorkloadApiClient::default().await
                };

                match client_result {
                    Ok(client) => break client,
                    Err(err) => {
                        tracing::warn!(error=%err, "spiffe_jwt: failed to create WorkloadApiClient; retrying in 5s");

                        // Use tokio::select! for cancellation during initial client creation
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                                // Continue retry loop
                            }
                            _ = cancellation_token.cancelled() => {
                                tracing::debug!("spiffe_jwt: cancellation token signaled during client creation, shutting down");
                                return;
                            }
                        }
                    }
                }
            };

            // Exponential backoff parameters for stream operations
            let mut backoff_duration = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(30);

            // Use interval for consistent timing
            let mut retry_interval = tokio::time::interval(backoff_duration);
            retry_interval.tick().await; // consume the first immediate tick

            loop {
                tokio::select! {
                    _ = async {
                        tracing::debug!("spiffe_jwt: starting JWT bundle stream");
                        match streaming_client.stream_jwt_bundles().await {
                            Ok(mut stream) => {
                                // Reset backoff after successful stream creation
                                backoff_duration = Duration::from_secs(1);
                                retry_interval = tokio::time::interval(backoff_duration);
                                retry_interval.tick().await; // consume the first immediate tick

                                while let Some(next_item) = stream.next().await {
                                    match next_item {
                                        Ok(update) => {
                                            let mut w = bundles_cache.write();
                                            *w = Some(update.clone());
                                            tracing::trace!("spiffe_jwt: updated in-memory JWT bundle set");
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
                        // Backoff before retrying using interval
                        retry_interval.tick().await;
                        backoff_duration = (backoff_duration * 2).min(MAX_BACKOFF);
                        retry_interval = tokio::time::interval(backoff_duration);
                    } => {
                        // Stream operation completed, continue loop
                    }

                    // Cancellation token - break out of loop when cancelled
                    _ = cancellation_token.cancelled() => {
                        tracing::debug!("spiffe_jwt: cancellation token signaled, shutting down");
                        break;
                    }
                }
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

impl Drop for SpiffeJwtVerifier {
    fn drop(&mut self) {
        // Cancel the background task when SpiffeJwtVerifier is dropped
        self.cancellation_token.cancel();
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
            None => Err(AuthError::WouldBlockOn),
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
            None => Err(AuthError::WouldBlockOn),
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    // #[tokio::test]
    // async fn test_spiffe_config_default() {
    //     let config = SpiffeProviderConfig::default();
    //     assert!(config.socket_path.is_none());
    //     assert!(config.target_spiffe_id.is_none());
    //     assert_eq!(config.jwt_audiences, vec!["slim"]);
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_creation() {
    //     let verifier_config = SpiffeVerifierConfig {
    //         socket_path: Some("unix:///tmp/fake.sock".to_string()),
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(verifier_config);
    //     assert_eq!(verifier.config.jwt_audiences, vec!["test-audience"]);
    // }

    // #[tokio::test]
    // async fn test_spiffe_provider_creation() {
    //     let config = SpiffeProviderConfig::default();
    //     let provider = SpiffeProvider::new(config);
    //     assert!(provider.x509_source.is_none());
    //     assert!(provider.jwt_source.is_none());
    // }

    // #[test]
    // fn test_spiffe_provider_get_x509_svid_not_initialized() {
    //     let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
    //     let res = provider.get_x509_svid();
    //     assert!(res.is_err());
    //     let err = format!("{}", res.unwrap_err());
    //     assert!(err.contains("X509Source not initialized"));
    // }

    // #[test]
    // fn test_spiffe_provider_get_jwt_svid_not_initialized() {
    //     let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
    //     let res = provider.get_jwt_svid();
    //     assert!(res.is_err());
    //     let err = format!("{}", res.unwrap_err());
    //     assert!(err.contains("JwtSource not initialized"));
    // }

    // #[tokio::test]
    // async fn test_jwt_source_creation_with_invalid_path_succeeds() {
    //     let src = JwtSource::new(vec!["aud".into()], None, None).await;
    //     assert!(
    //         src.is_ok(),
    //         "JwtSource::new should succeed - errors happen in background task"
    //     );
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_try_verify_without_bundles() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["aud".into()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);
    //     let res = verifier.try_verify("token".to_string());
    //     assert!(res.is_err());
    //     assert!(matches!(res, Err(AuthError::WouldBlockOn)));
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_try_get_claims_without_bundles() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["aud".into()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);
    //     let claims_result: Result<serde_json::Value, AuthError> =
    //         verifier.try_get_claims("token".to_string());
    //     assert!(claims_result.is_err());
    //     assert!(matches!(claims_result, Err(AuthError::WouldBlockOn)));
    // }

    // #[tokio::test]
    // async fn test_jwt_source_cancellation_on_drop() {
    //     // This test verifies that the background task terminates when JwtSource is dropped
    //     // Since client initialization moved to background task, constructor succeeds
    //     // but the background task handles connection errors and can be cancelled

    //     // Create a scope where JwtSource exists
    //     {
    //         let jwt_source_result = JwtSource::new(
    //             vec!["test-audience".to_string()],
    //             None,
    //             None, // Will use default client and retry in background
    //         )
    //         .await;

    //         assert!(
    //             jwt_source_result.is_ok(),
    //             "JwtSource creation should succeed - errors handled in background"
    //         );
    //     }
    // }

    // #[tokio::test]
    // async fn test_jwt_source_drop_cancels_background_task() {
    //     use tokio::time::{Duration, sleep};

    //     // Create JwtSource in a limited scope
    //     let cancellation_token = {
    //         let jwt_source = JwtSource::new(vec!["test-audience".to_string()], None, None)
    //             .await
    //             .unwrap();

    //         // Get reference to the cancellation token before dropping
    //         jwt_source.cancellation_token.clone()
    //     }; // JwtSource is dropped here, which should cancel the token

    //     // Give a small delay for the drop to be processed
    //     sleep(Duration::from_millis(10)).await;

    //     // Verify that the cancellation token was triggered
    //     assert!(cancellation_token.is_cancelled());
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_drop_cancels_background_task() {
    //     use tokio::time::{Duration, sleep};

    //     // Create SpiffeJwtVerifier in a limited scope
    //     let cancellation_token = {
    //         let spiffe_config = SpiffeVerifierConfig {
    //             socket_path: Some("/tmp/non-existent-spiffe-socket".to_string()),
    //             jwt_audiences: vec!["test-audience".to_string()],
    //         };
    //         let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //         // Get reference to the cancellation token before dropping
    //         verifier.cancellation_token.clone()
    //     }; // SpiffeJwtVerifier is dropped here, which should cancel the token

    //     // Give a small delay for the drop to be processed
    //     sleep(Duration::from_millis(10)).await;

    //     // Verify that the cancellation token was triggered
    //     assert!(cancellation_token.is_cancelled());
    // }

    // #[tokio::test]
    // async fn test_multiple_jwt_sources_independent_cancellation() {
    //     use tokio::time::{Duration, sleep};

    //     // Create multiple JwtSources to verify they have independent cancellation tokens
    //     let jwt_source1 = JwtSource::new(vec!["audience1".to_string()], None, None)
    //         .await
    //         .unwrap();

    //     let jwt_source2 = JwtSource::new(vec!["audience2".to_string()], None, None)
    //         .await
    //         .unwrap();

    //     let token1 = jwt_source1.cancellation_token.clone();
    //     let token2 = jwt_source2.cancellation_token.clone();

    //     // Both should be active initially
    //     assert!(!token1.is_cancelled());
    //     assert!(!token2.is_cancelled());

    //     // Drop only the first one
    //     drop(jwt_source1);
    //     sleep(Duration::from_millis(10)).await;

    //     // Only the first token should be cancelled
    //     assert!(token1.is_cancelled());
    //     assert!(!token2.is_cancelled());

    //     // Drop the second one
    //     drop(jwt_source2);
    //     sleep(Duration::from_millis(10)).await;

    //     // Now both should be cancelled
    //     assert!(token1.is_cancelled());
    //     assert!(token2.is_cancelled());
    // }

    // #[test]
    // fn test_spiffe_provider_get_x509_cert_pem_not_initialized() {
    //     let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
    //     let res = provider.get_x509_cert_pem();
    //     assert!(res.is_err());
    //     let err = format!("{}", res.unwrap_err());
    //     assert!(err.contains("X509Source not initialized"));
    // }

    // #[test]
    // fn test_spiffe_provider_get_x509_key_pem_not_initialized() {
    //     let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
    //     let res = provider.get_x509_key_pem();
    //     assert!(res.is_err());
    //     let err = format!("{}", res.unwrap_err());
    //     assert!(err.contains("X509Source not initialized"));
    // }

    // #[test]
    // fn test_spiffe_provider_get_token_not_initialized() {
    //     let provider = SpiffeProvider::new(SpiffeProviderConfig::default());
    //     let res = provider.get_token();
    //     assert!(res.is_err());
    //     let err = format!("{}", res.unwrap_err());
    //     assert!(err.contains("JwtSource not initialized"));
    // }

    // #[test]
    // fn test_spiffe_provider_default_config() {
    //     let config = SpiffeProviderConfig::default();
    //     assert_eq!(config.socket_path, None);
    //     assert_eq!(config.jwt_audiences, vec!["slim".to_string()]); // Fixed: default includes "slim"
    //     assert_eq!(config.target_spiffe_id, None);

    //     let provider = SpiffeProvider::new(config);
    //     assert!(provider.x509_source.is_none());
    //     assert!(provider.jwt_source.is_none());
    // }

    // #[test]
    // fn test_spiffe_verifier_config_creation() {
    //     let config = SpiffeVerifierConfig {
    //         socket_path: Some("/custom/path".to_string()),
    //         jwt_audiences: vec!["aud1".to_string(), "aud2".to_string()],
    //     };

    //     assert_eq!(config.socket_path, Some("/custom/path".to_string()));
    //     assert_eq!(config.jwt_audiences, vec!["aud1", "aud2"]);
    // }

    // #[tokio::test]
    // async fn test_jwt_source_get_svid_initially_none() {
    //     let jwt_source = JwtSource::new(vec!["test-audience".to_string()], None, None)
    //         .await
    //         .unwrap();

    //     // Initially, before any successful fetch, SVID should be None
    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    //     assert!(svid_result.unwrap().is_none());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_get_svid_multiple_calls() {
    //     let jwt_source = JwtSource::new(vec!["test-audience".to_string()], None, None)
    //         .await
    //         .unwrap();

    //     // Multiple calls should not block and return consistently
    //     let svid1 = jwt_source.get_svid();
    //     let svid2 = jwt_source.get_svid();
    //     let svid3 = jwt_source.get_svid();

    //     assert!(svid1.is_ok());
    //     assert!(svid2.is_ok());
    //     assert!(svid3.is_ok());

    //     // All should be None initially (no successful fetch yet)
    //     assert!(svid1.unwrap().is_none());
    //     assert!(svid2.unwrap().is_none());
    //     assert!(svid3.unwrap().is_none());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_with_target_spiffe_id() {
    //     let target_id = "spiffe://example.org/service/backend".to_string();
    //     let jwt_source = JwtSource::new(
    //         vec!["test-audience".to_string()],
    //         Some(target_id.clone()),
    //         None,
    //     )
    //     .await
    //     .unwrap();

    //     // Should create successfully even with target SPIFFE ID
    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    //     assert!(svid_result.unwrap().is_none());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_builder_default() {
    //     let builder = JwtSourceBuilder::default();
    //     assert!(builder.audiences.is_empty());
    //     assert!(builder.target_spiffe_id.is_none());
    //     assert!(builder.client.is_none());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_builder_with_audiences() {
    //     let audiences = vec!["aud1".to_string(), "aud2".to_string()];
    //     let jwt_source = JwtSourceBuilder::new()
    //         .with_audiences(audiences.clone())
    //         .build()
    //         .await
    //         .unwrap();

    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_builder_with_target_spiffe_id() {
    //     let target_id = "spiffe://example.org/my-service".to_string();
    //     let jwt_source = JwtSourceBuilder::new()
    //         .with_audiences(vec!["test-aud".to_string()])
    //         .with_target_spiffe_id(target_id.clone())
    //         .build()
    //         .await
    //         .unwrap();

    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_builder_full_configuration() {
    //     let audiences = vec!["aud1".to_string(), "aud2".to_string()];
    //     let target_id = "spiffe://example.org/backend".to_string();

    //     let jwt_source = JwtSourceBuilder::new()
    //         .with_audiences(audiences.clone())
    //         .with_target_spiffe_id(target_id.clone())
    //         .build()
    //         .await
    //         .unwrap();

    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    //     assert!(svid_result.unwrap().is_none());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_builder_minimal_configuration() {
    //     // Minimal configuration - just audiences
    //     let jwt_source = JwtSourceBuilder::new()
    //         .with_audiences(vec!["minimal-aud".to_string()])
    //         .build()
    //         .await
    //         .unwrap();

    //     let svid_result = jwt_source.get_svid();
    //     assert!(svid_result.is_ok());
    // }

    // #[tokio::test]
    // async fn test_jwt_source_audiences_getter() {
    //     let audiences = vec!["aud1".to_string(), "aud2".to_string()];
    //     let jwt_source = JwtSource::new(audiences.clone(), None, None).await.unwrap();

    //     assert_eq!(jwt_source.audiences(), &audiences[..]);
    // }

    // #[tokio::test]
    // async fn test_jwt_source_target_spiffe_id_getter() {
    //     let target_id = "spiffe://example.org/service".to_string();
    //     let jwt_source =
    //         JwtSource::new(vec!["test-aud".to_string()], Some(target_id.clone()), None)
    //             .unwrap();

    //     assert_eq!(jwt_source.target_spiffe_id(), Some(target_id.as_str()));
    // }

    // #[tokio::test]
    // async fn test_jwt_source_target_spiffe_id_none() {
    //     let jwt_source = JwtSource::new(vec!["test-aud".to_string()], None, None)
    //         .unwrap();

    //     assert_eq!(jwt_source.target_spiffe_id(), None);
    // }

    // #[tokio::test]
    // async fn test_jwt_source_fetch_with_custom_audiences() {
    //     // Create JwtSource with default audiences
    //     let default_audiences = vec!["default-aud".to_string()];
    //     let jwt_source = JwtSource::new(default_audiences.clone(), None, None)
    //         .unwrap();

    //     // Request JWT with custom audiences
    //     let custom_audiences = vec!["custom-aud1".to_string(), "custom-aud2".to_string()];
    //     let custom_target = Some("spiffe://example.org/custom-service".to_string());

    //     // This will fail in test environment since there's no real SPIFFE workload API,
    //     // but it verifies the channel mechanism works
    //     let result = jwt_source
    //         .fetch_with_custom_audiences(custom_audiences, custom_target)
    //         .await;

    //     // We expect an error since there's no real workload API, but the important
    //     // thing is that the channel worked and we got a response
    //     assert!(result.is_err());

    //     // Verify default configuration is still accessible
    //     assert_eq!(jwt_source.audiences(), &default_audiences[..]);
    //     assert_eq!(jwt_source.target_spiffe_id(), None);
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_initialize_with_invalid_socket() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: Some("/tmp/completely-invalid-socket-path".to_string()),
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Initialize should fail with invalid socket path
    //     let result = verifier.initialize().await;
    //     assert!(result.is_err());
    //     let err = format!("{}", result.unwrap_err());
    //     assert!(err.contains("Failed to connect to SPIFFE Workload API"));
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_initialize_with_none_socket() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None, // Will use SPIFFE_ENDPOINT_SOCKET env var
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // This will likely fail unless SPIFFE_ENDPOINT_SOCKET is set to a valid socket
    //     let result = verifier.initialize().await;
    //     // We can't guarantee the environment has SPIFFE_ENDPOINT_SOCKET set,
    //     // so we just verify the method can be called
    //     assert!(result.is_err() || result.is_ok());
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_multiple_initialize_calls() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: Some("/tmp/test-socket".to_string()),
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Multiple initialization calls should be handled gracefully
    //     let result1 = verifier.initialize().await;
    //     let result2 = verifier.initialize().await;

    //     // Both should fail with the same socket error (no real SPIFFE server)
    //     assert!(result1.is_err());
    //     assert!(result2.is_err());
    // }

    // #[test]
    // fn test_spiffe_jwt_verifier_creation_with_multiple_audiences() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: Some("/custom/socket".to_string()),
    //         jwt_audiences: vec![
    //             "audience1".to_string(),
    //             "audience2".to_string(),
    //             "audience3".to_string(),
    //         ],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     assert_eq!(verifier.config.jwt_audiences.len(), 3);
    //     assert!(
    //         verifier
    //             .config
    //             .jwt_audiences
    //             .contains(&"audience1".to_string())
    //     );
    //     assert!(
    //         verifier
    //             .config
    //             .jwt_audiences
    //             .contains(&"audience2".to_string())
    //     );
    //     assert!(
    //         verifier
    //             .config
    //             .jwt_audiences
    //             .contains(&"audience3".to_string())
    //     );
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_verify_without_initialization() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Trying to verify without initialization should fail
    //     let result = verifier.verify("fake.jwt.token").await;
    //     assert!(result.is_err());
    //     let err = format!("{}", result.unwrap_err());
    //     assert!(err.contains("WorkloadApiClient not initialized"));
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_get_claims_without_initialization() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Trying to get claims without initialization should fail
    //     let result: Result<serde_json::Value, AuthError> =
    //         verifier.get_claims("fake.jwt.token").await;
    //     assert!(result.is_err());
    //     let err = format!("{}", result.unwrap_err());
    //     assert!(err.contains("WorkloadApiClient not initialized"));
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_verify_with_invalid_token() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Even with cached bundles (if any), invalid token should fail
    //     let result = verifier.try_verify("not.a.valid.jwt.token");
    //     assert!(result.is_err());
    //     // It can be either WouldBlockOn or TokenInvalid depending on bundle availability
    //     // Since we have not initialized verifier, it should be WouldBlockOn
    //     assert!(matches!(result, Err(AuthError::WouldBlockOn)));
    // }

    // #[tokio::test]
    // async fn test_spiffe_jwt_verifier_try_get_claims_invalid_token() {
    //     let spiffe_config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec!["test-audience".to_string()],
    //     };
    //     let verifier = SpiffeJwtVerifier::new(spiffe_config);

    //     // Should return specific error about async context requirement
    //     let result: Result<serde_json::Value, AuthError> = verifier.try_get_claims("invalid.token");
    //     assert!(result.is_err());
    //     // It can be either WouldBlockOn or TokenInvalid depending on bundle availability
    //     // Since we have not initialized verifier, it should be WouldBlockOn
    //     assert!(matches!(result, Err(AuthError::WouldBlockOn)));
    // }

    // #[test]
    // fn test_spiffe_jwt_verifier_config_default_values() {
    //     let config = SpiffeVerifierConfig {
    //         socket_path: None,
    //         jwt_audiences: vec![],
    //     };

    //     let verifier = SpiffeJwtVerifier::new(config);
    //     assert_eq!(verifier.config.socket_path, None);
    //     assert!(verifier.config.jwt_audiences.is_empty());
    //     assert!(!verifier.cancellation_token.is_cancelled());
    // }
}

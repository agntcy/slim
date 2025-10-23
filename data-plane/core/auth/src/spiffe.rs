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
    BundleSource, JwtBundleSet, JwtSvid, SvidSource, WorkloadApiClient, X509Bundle, X509Source,
    X509SourceBuilder, X509Svid,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info}; // for sync access in TokenProvider impl

/// Helper for encoding/decoding custom claims in JWT audiences
///
/// This codec provides a transparent mechanism to embed custom claims in JWT tokens
/// by encoding them as a special audience string. The verifier automatically extracts
/// and decodes these claims, making the process transparent to the caller.
///
/// ## Encoding Process
///
/// 1. Custom claims (HashMap) are serialized to JSON
/// 2. JSON is base64-encoded
/// 3. Encoded string is prefixed with "slim-claims:" and added to audiences
///
/// ## Decoding Process
///
/// 1. Audiences are scanned for "slim-claims:" prefix
/// 2. Base64 payload is decoded and parsed as JSON
/// 3. Custom claims are extracted and returned separately
/// 4. Special audience is removed from the audience list
///
/// ## Example
///
/// ```ignore
/// // Provider encodes custom claims
/// let mut claims = HashMap::new();
/// claims.insert("pubkey".to_string(), json!("abc123"));
/// let token = provider.get_token_with_claims(claims).await?;
///
/// // Verifier transparently extracts them
/// let extracted_claims = verifier.get_claims::<MyClaims>(token)?;
/// // extracted_claims.custom_claims contains the original claims
/// // extracted_claims.aud does NOT contain the special audience
/// ```
struct CustomClaimsCodec;

impl CustomClaimsCodec {
    const CLAIMS_PREFIX: &'static str = "slim-claims:";

    /// Encode custom claims as a special audience string
    ///
    /// Takes a HashMap of custom claims, serializes to JSON, base64-encodes it,
    /// and returns a string prefixed with "slim-claims:".
    ///
    /// # Returns
    ///
    /// A string in the format: `slim-claims:<base64-encoded-json>`
    fn encode_audience(
        custom_claims: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, AuthError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let claims_json = serde_json::to_string(custom_claims).map_err(|e| {
            AuthError::ConfigError(format!("Failed to serialize custom claims: {}", e))
        })?;

        let claims_b64 = BASE64.encode(claims_json.as_bytes());
        Ok(format!("{}{}", Self::CLAIMS_PREFIX, claims_b64))
    }

    /// Decode custom claims from audiences, returning (filtered_audiences, custom_claims)
    ///
    /// Scans through all audiences looking for the "slim-claims:" prefix. When found,
    /// decodes the base64-encoded JSON payload and extracts the custom claims.
    ///
    /// # Returns
    ///
    /// A tuple of:
    /// - `Vec<String>`: Filtered audience list with custom claims audience removed
    /// - `serde_json::Map`: Extracted custom claims (empty if none found)
    ///
    /// # Behavior
    ///
    /// - Non-custom audiences are preserved in the filtered list
    /// - Invalid base64 or JSON is logged and the audience is preserved
    /// - Multiple custom claim audiences are merged together
    fn decode_from_audiences(
        audiences: &[String],
    ) -> (Vec<String>, serde_json::Map<String, serde_json::Value>) {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let mut filtered_audiences = Vec::new();
        let mut custom_claims_map = serde_json::Map::new();

        for aud in audiences {
            if let Some(claims_b64) = aud.strip_prefix(Self::CLAIMS_PREFIX) {
                // Decode custom claims from audience
                match BASE64.decode(claims_b64.as_bytes()) {
                    Ok(claims_bytes) => {
                        match serde_json::from_slice::<serde_json::Value>(&claims_bytes) {
                            Ok(serde_json::Value::Object(claims)) => {
                                custom_claims_map.extend(claims);
                                tracing::debug!("Extracted custom claims from audience");
                            }
                            _ => {
                                tracing::warn!("Failed to parse custom claims as object");
                                filtered_audiences.push(aud.clone());
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode custom claims base64: {}", e);
                        filtered_audiences.push(aud.clone());
                    }
                }
            } else {
                filtered_audiences.push(aud.clone());
            }
        }

        (filtered_audiences, custom_claims_map)
    }
}

/// Helper function to create a WorkloadApiClient based on configuration
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

    /// Initialize the SPIFFE provider and start credential rotation
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        info!("Initializing SPIFFE provider");

        // Create WorkloadApiClient
        let client = create_workload_client(self.config.socket_path.as_ref()).await?;

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

        // Encode custom claims as a special audience
        let claims_audience = CustomClaimsCodec::encode_audience(&custom_claims)?;

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
    bundles: Arc<RwLock<Option<JwtBundleSet>>>,
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
        let bundles = Arc::new(RwLock::new(None));
        let audiences_clone = audiences.clone();
        let target_clone = target_spiffe_id.clone();
        let cancellation_token = CancellationToken::new();
        let token_clone = cancellation_token.clone();

        // Get an initial JWT SVID
        let mut workload_client = Self::initialize_client(client.clone()).await;

        match fetch_once(
            &mut workload_client,
            &audiences_clone,
            target_clone.as_ref(),
        )
        .await
        {
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

        // Spawn background task for JWT SVID refresh
        tokio::spawn(async move {
            Self::background_refresh_task(
                workload_client,
                audiences_clone,
                target_clone,
                current_clone,
                token_clone,
                custom_request_rx,
                cfg,
            )
            .await;
        });

        // Fetch initial JWT bundle before spawning background task
        let bundle_client = Self::initialize_client(client).await;
        match Self::fetch_jwt_bundle_once(bundle_client.clone(), &bundles).await {
            Ok(()) => {
                tracing::debug!("jwt_source: initial JWT bundle fetched successfully");
            }
            Err(err) => {
                tracing::warn!(error=%err, "jwt_source: initial JWT bundle fetch failed; will retry in background");
            }
        }

        // Spawn background task for JWT bundle streaming
        let bundles_for_task = bundles.clone();
        let token_for_bundles = CancellationToken::new();
        tokio::spawn(async move {
            Self::stream_jwt_bundles(bundle_client, bundles_for_task, token_for_bundles).await;
        });

        Ok(Arc::new(Self {
            _audiences: audiences,
            _target_spiffe_id: target_spiffe_id,
            current,
            bundles,
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
    fn get_svid(&self) -> Result<Option<JwtSvid>, String> {
        let guard = self.current.read();
        Ok(guard.clone())
    }

    /// Get the current JWT bundles for verification (synchronous)
    pub fn get_bundles(&self) -> Result<Option<JwtBundleSet>, String> {
        let guard = self.bundles.read();
        Ok(guard.clone())
    }

    /// Fetch JWT bundle once (helper for initialization)
    async fn fetch_jwt_bundle_once(
        mut client: WorkloadApiClient,
        bundles: &Arc<RwLock<Option<JwtBundleSet>>>,
    ) -> Result<(), String> {
        match client.stream_jwt_bundles().await {
            Ok(mut stream) => {
                if let Some(result) = stream.next().await {
                    match result {
                        Ok(bundle_set) => {
                            *bundles.write() = Some(bundle_set);
                            Ok(())
                        }
                        Err(e) => Err(format!("Failed to read JWT bundle: {}", e)),
                    }
                } else {
                    Err("JWT bundle stream ended without data".to_string())
                }
            }
            Err(e) => Err(format!("Failed to start JWT bundle stream: {}", e)),
        }
    }

    /// Background task to stream JWT bundles
    async fn stream_jwt_bundles(
        mut client: WorkloadApiClient,
        bundles: Arc<RwLock<Option<JwtBundleSet>>>,
        cancellation_token: CancellationToken,
    ) {
        loop {
            match client.stream_jwt_bundles().await {
                Ok(mut stream) => loop {
                    tokio::select! {
                        result = stream.next() => {
                            match result {
                                Some(Ok(bundle_set)) => {
                                    *bundles.write() = Some(bundle_set);
                                    tracing::trace!("jwt_source: updated JWT bundle cache");
                                }
                                Some(Err(e)) => {
                                    tracing::warn!(error=%e, "jwt_source: bundle stream error, restarting in 1s");
                                    tokio::select! {
                                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                                        _ = cancellation_token.cancelled() => {
                                            tracing::debug!("jwt_source: bundle streaming cancelled");
                                            return;
                                        }
                                    }
                                    break;
                                }
                                None => {
                                    tracing::debug!("jwt_source: bundle stream ended, restarting in 1s");
                                    tokio::select! {
                                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                                        _ = cancellation_token.cancelled() => {
                                            tracing::debug!("jwt_source: bundle streaming cancelled");
                                            return;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        _ = cancellation_token.cancelled() => {
                            tracing::debug!("jwt_source: bundle streaming cancelled");
                            return;
                        }
                    }
                },
                Err(e) => {
                    tracing::warn!(error=%e, "jwt_source: failed to start bundle stream, retrying in 5s");
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                        _ = cancellation_token.cancelled() => {
                            tracing::debug!("jwt_source: bundle streaming cancelled");
                            return;
                        }
                    }
                }
            }
        }
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

/// Configuration for SPIFFE verifier
#[derive(Debug, Clone)]
pub struct SpiffeVerifierConfig {
    /// Path to the SPIFFE Workload API socket
    pub socket_path: Option<String>,
    /// JWT audiences expected in tokens
    pub jwt_audiences: Vec<String>,
}

/// SPIFFE Verifier for validating X.509 certificates and JWT tokens
///
/// This verifier provides a simple, clean interface for verification using SPIFFE bundles.
///
/// ## Features
///
/// - **X.509 Bundle Management**: Uses `X509Source` to automatically maintain and rotate
///   X.509 trust bundles for certificate verification
/// - **JWT Bundle Caching**: Maintains a simple background task that streams JWT bundles
/// - **Automatic Rotation**: Both X.509 and JWT bundles are automatically updated as they rotate
/// - **Sync and Async Access**: Both `verify()` and `try_verify()` work with cached bundles
///
/// ## Architecture
///
/// The verifier:
/// 1. Initializes an `X509Source` for X.509 bundle management (handles rotation automatically)
/// 2. Spawns a simple background task to stream and cache JWT bundles
/// 3. Provides sync access to cached bundles, just like X509Source does for X.509 bundles
///
/// This mirrors how `X509Source` works internally - bundles are maintained in the background,
/// and verification methods have synchronous access to the cached bundles.
///
/// ## Usage
///
/// ```rust,no_run
/// # use slim_auth::spiffe::{SpiffeJwtVerifier, SpiffeVerifierConfig};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = SpiffeVerifierConfig {
///     socket_path: None, // Uses SPIFFE_ENDPOINT_SOCKET env var
///     jwt_audiences: vec!["my-service".to_string()],
/// };
///
/// let mut verifier = SpiffeJwtVerifier::new(config);
/// verifier.initialize().await?;
///
/// // Get X.509 bundle for certificate verification
/// let x509_bundle = verifier.get_x509_bundle()?;
///
/// // Or get the X509Source directly for more control
/// let x509_source = verifier.get_x509_source()?;
///
/// // Verify a JWT token (async or sync, both use cached bundles)
/// verifier.verify("eyJ...").await?;
/// verifier.try_verify("eyJ...")?;  // Synchronous version
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SpiffeJwtVerifier {
    config: SpiffeVerifierConfig,
    client: Option<WorkloadApiClient>,
    x509_source: Option<Arc<X509Source>>,
    jwt_source: Option<Arc<JwtSource>>,
}

impl SpiffeJwtVerifier {
    /// Create a new SPIFFE verifier
    pub fn new(config: SpiffeVerifierConfig) -> Self {
        Self {
            config,
            client: None,
            x509_source: None,
            jwt_source: None,
        }
    }

    /// Initialize the verifier and start bundle rotation
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        info!("Initializing SPIFFE verifier");

        // Create WorkloadApiClient
        let client = create_workload_client(self.config.socket_path.as_ref()).await?;

        // Initialize X509Source for certificate bundle management
        let x509_source = X509SourceBuilder::new()
            .with_client(client.clone())
            .build()
            .await
            .map_err(|e| {
                AuthError::ConfigError(format!("Failed to initialize X509Source: {}", e))
            })?;

        self.x509_source = Some(x509_source);

        // Initialize JwtSource for JWT bundle management
        let jwt_source = JwtSource::new(
            self.config.jwt_audiences.clone(),
            None, // No target SPIFFE ID needed for verification
            Some(client.clone()),
        )
        .await
        .map_err(|e| AuthError::ConfigError(format!("Failed to initialize JwtSource: {}", e)))?;

        self.jwt_source = Some(jwt_source);

        info!("SPIFFE verifier initialized successfully");

        self.client = Some(client);

        Ok(())
    }

    /// Get the X.509 bundle for a specific trust domain
    ///
    /// Returns the X.509 bundle (CA certificates) for the default trust domain.
    /// This bundle can be used to verify X.509 certificates from workloads in the trust domain.
    pub fn get_x509_bundle(&self) -> Result<X509Bundle, AuthError> {
        let x509_source = self
            .x509_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("X509Source not initialized".to_string()))?;

        // Get the SVID to determine the trust domain
        let svid = x509_source
            .get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get X509 SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No X509 SVID available".to_string()))?;

        let trust_domain = svid.spiffe_id().trust_domain();

        // Get the bundle for the trust domain
        x509_source
            .get_bundle_for_trust_domain(trust_domain)
            .map_err(|e| {
                AuthError::ConfigError(format!(
                    "Failed to get X509 bundle for trust domain {}: {}",
                    trust_domain, e
                ))
            })?
            .ok_or_else(|| {
                AuthError::ConfigError(format!(
                    "No X509 bundle available for trust domain {}",
                    trust_domain
                ))
            })
    }

    /// Get JWT bundles for token validation
    fn get_jwt_bundles(&self) -> Result<JwtBundleSet, AuthError> {
        let jwt_source = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;

        jwt_source
            .get_bundles()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get JWT bundles: {}", e)))?
            .ok_or_else(|| {
                AuthError::ConfigError(
                    "JWT bundles not yet available - background task still initializing"
                        .to_string(),
                )
            })
    }
}

#[async_trait]
impl Verifier for SpiffeJwtVerifier {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        let bundles = self.get_jwt_bundles()?;
        JwtSvid::parse_and_validate(&token.into(), &bundles, &self.config.jwt_audiences)
            .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;
        debug!("Successfully verified JWT token (sync)");
        Ok(())
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        self.try_get_claims(token)
    }

    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: DeserializeOwned + Send,
    {
        let bundles = self.get_jwt_bundles()?;
        let jwt_svid =
            JwtSvid::parse_and_validate(&token.into(), &bundles, &self.config.jwt_audiences)
                .map_err(|e| AuthError::TokenInvalid(format!("JWT validation failed: {}", e)))?;

        debug!(
            "Successfully extracted claims for SPIFFE ID: {}",
            jwt_svid.spiffe_id()
        );

        // Extract custom claims from audiences and filter them out
        let audiences = jwt_svid.audience();
        let (filtered_audiences, custom_claims_map) =
            CustomClaimsCodec::decode_from_audiences(audiences);

        // Build claims JSON with custom claims merged in
        let mut claims_json = serde_json::json!({
            "sub": jwt_svid.spiffe_id().to_string(),
            "aud": filtered_audiences,
            "exp": jwt_svid.expiry().to_string(),
        });

        // Merge custom claims into the claims object
        if let Some(obj) = claims_json.as_object_mut() {
            if !custom_claims_map.is_empty() {
                obj.insert(
                    "custom_claims".to_string(),
                    serde_json::Value::Object(custom_claims_map),
                );
            }
        }

        serde_json::from_value(claims_json)
            .map_err(|e| AuthError::ConfigError(format!("Failed to deserialize JWT claims: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    // tested in integratin tests
}

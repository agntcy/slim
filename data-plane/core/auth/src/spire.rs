// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#![cfg(not(target_family = "windows"))]

//! SPIRE integration for SLIM authentication
//!
//! Unified spire interface: the `SpireIdentityManager` encapsulates both
//! credential acquisition (X.509 & JWT SVIDs) and verification (JWT validation
//! plus access to X.509 trust bundles) using a single configuration struct
//! `SpireConfig`.
//!
//! Features:
//! - Single struct for providing and verifying identities (`SpireIdentityManager`)
//! - Automatic rotation of X.509 SVIDs and JWT SVIDs via background sources
//! - Access to private key & certificate PEM for mTLS
//! - Access to JWT tokens (optionally with custom claims encoded in audiences)
//! - Synchronous and asynchronous JWT verification (`try_verify` / `verify`)
//! - Claims extraction with transparent custom claim decoding
//! - Trust domain bundle retrieval (`get_x509_bundle`)
//!
//! Primary types:
//! - `SpireConfig`: configuration (socket path, target SPIFFE ID for JWT requests, audiences)
//! - `SpireIdentityManager`: unified provider + verifier
//!
//! Basic usage:
//! ```rust,no_run
//! use slim_auth::spire::{SpireIdentityManager, SpireConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut mgr = SpireIdentityManager::new(SpireConfig {
//!     socket_path: None,              // Use SPIFFE_ENDPOINT_SOCKET env var
//!     target_spiffe_id: None,         // Optional: specify a target for JWT SVID
//!     jwt_audiences: vec!["my-app".into()],
//! });
//! mgr.initialize().await?;
//!
//! // Obtain JWT token
//! let token = mgr.get_token()?;
//!
//! // Verify the token (async or sync)
//! mgr.verify(&token).await?;
//! mgr.try_verify(&token)?;
//!
//! // Extract claims
//! let claims: serde_json::Value = mgr.get_claims(&token).await?;
//!
//! // Access X.509 materials for TLS
//! let cert_pem = mgr.get_x509_cert_pem()?;
//! let key_pem  = mgr.get_x509_key_pem()?;
//!
//! // Access trust bundle for custom verification
//! let x509_bundle = mgr.get_x509_bundle()?;
//! # Ok(()) }
//! ```
//!
//! Custom claims:
//! Use `get_token_with_claims` to embed additional claims via a special audience
//! encoding. The verifier automatically decodes and exposes them under
//! the `custom_claims` field in returned claim structures.
//!
//! This unified design replaced the previous split between
//! `SpiffeProvider` and `SpiffeJwtVerifier`.

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::StreamExt;
use jsonwebtoken_aws_lc::TokenData;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde_json::{self, Value};
use spiffe::{
    BundleSource, JwtBundleSet, JwtSvid, SvidSource, TrustDomain, WorkloadApiClient, X509Bundle,
    X509Source, X509SourceBuilder, X509Svid,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::errors::AuthError;
use crate::metadata::MetadataMap;
use crate::traits::{TokenProvider, Verifier};
use crate::utils::bytes_to_pem;

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
    fn encode_audience(custom_claims: &MetadataMap) -> Result<String, AuthError> {
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
    ) -> (Vec<String>, serde_json::Map<String, Value>) {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let mut filtered_audiences = Vec::new();
        let mut custom_claims_map = serde_json::Map::new();

        for aud in audiences {
            if let Some(claims_b64) = aud.strip_prefix(Self::CLAIMS_PREFIX) {
                // Decode custom claims from audience
                match BASE64.decode(claims_b64.as_bytes()) {
                    Ok(claims_bytes) => match serde_json::from_slice::<Value>(&claims_bytes) {
                        Ok(Value::Object(claims)) => {
                            custom_claims_map.extend(claims);
                            tracing::debug!("Extracted custom claims from audience");
                        }
                        _ => {
                            tracing::warn!("Failed to parse custom claims as object");
                            filtered_audiences.push(aud.clone());
                        }
                    },
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

/// Builder for constructing a SpiffeIdentityManager
pub struct SpireIdentityManagerBuilder {
    socket_path: Option<String>,
    target_spiffe_id: Option<String>,
    jwt_audiences: Vec<String>,
}

impl Default for SpireIdentityManagerBuilder {
    fn default() -> Self {
        Self {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: vec!["slim".to_string()],
        }
    }
}

impl SpireIdentityManagerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_socket_path(mut self, socket_path: impl Into<String>) -> Self {
        let mut path = socket_path.into();
        if !path.starts_with("unix:") {
            path = format!("unix:{}", path);
        }
        self.socket_path = Some(path);
        self
    }

    pub fn with_target_spiffe_id(mut self, target_spiffe_id: impl Into<String>) -> Self {
        self.target_spiffe_id = Some(target_spiffe_id.into());
        self
    }

    pub fn with_jwt_audiences(mut self, audiences: Vec<String>) -> Self {
        self.jwt_audiences = audiences;
        self
    }

    pub fn build(self) -> SpireIdentityManager {
        SpireIdentityManager {
            socket_path: self.socket_path,
            target_spiffe_id: self.target_spiffe_id,
            jwt_audiences: self.jwt_audiences,
            client: None,
            x509_source: None,
            jwt_source: None,
        }
    }
}

/// SPIFFE certificate and JWT provider that automatically rotates credentials
#[derive(Clone)]
pub struct SpireIdentityManager {
    socket_path: Option<String>,
    target_spiffe_id: Option<String>,
    jwt_audiences: Vec<String>,
    client: Option<WorkloadApiClient>,
    x509_source: Option<Arc<X509Source>>,
    jwt_source: Option<Arc<JwtSource>>,
}

impl SpireIdentityManager {
    /// Convenience: start building a new SpireIdentityManager
    pub fn builder() -> SpireIdentityManagerBuilder {
        SpireIdentityManagerBuilder::new()
    }

    /// Initialize the spire identity manager (sources for X.509 & JWT)
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        info!("Initializing spire identity manager");

        // Create WorkloadApiClient
        let client = create_workload_client(self.socket_path.as_ref()).await?;

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
            .with_audiences(self.jwt_audiences.clone())
            .with_client(client.clone());

        if let Some(ref target_id) = self.target_spiffe_id {
            jwt_builder = jwt_builder.with_target_spiffe_id(target_id.clone());
        }

        let jwt_source = jwt_builder.build().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to initialize JwtSource: {}", e))
        })?;

        self.jwt_source = Some(jwt_source);

        info!("spire provider initialized successfully");

        self.client = Some(client);

        Ok(())
    }

    /// Get the current X.509 SVID (leaf cert + key)
    pub fn get_x509_svid(&self) -> Result<X509Svid, AuthError> {
        let x509_source = self
            .x509_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("X509Source not initialized".to_string()))?;
        let svid = x509_source
            .get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get X509 SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No X509 SVID available".to_string()))?;
        debug!("Retrieved X509 SVID with SPIFFE ID: {}", svid.spiffe_id());
        Ok(svid)
    }

    /// Get the X.509 certificate (leaf) in PEM format
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

    /// Get a cached JWT SVID (background refreshed)
    pub fn get_jwt_svid(&self) -> Result<JwtSvid, AuthError> {
        let src = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;
        src.get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get JWT SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No JWT SVID available".to_string()))
    }

    /// Get X.509 bundle for the trust domain of our SVID (for verification use-cases)
    pub fn get_x509_bundle(&self) -> Result<X509Bundle, AuthError> {
        let x509_source = self
            .x509_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("X509Source not initialized".to_string()))?;

        // Derive trust domain from current SVID
        let svid = x509_source
            .get_svid()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get X509 SVID: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("No X509 SVID available".to_string()))?;

        let td = svid.spiffe_id().trust_domain();

        x509_source
            .get_bundle_for_trust_domain(td)
            .map_err(|e| AuthError::ConfigError(format!("Failed to get X509 bundle: {}", e)))?
            .ok_or_else(|| {
                AuthError::ConfigError(format!("No X509 bundle for trust domain {}", td))
            })
    }

    /// Get the X.509 bundle for an explicit trust domain (ignores config override)
    pub async fn get_x509_bundle_for_trust_domain(
        &mut self,
        trust_domain: impl Into<String>,
    ) -> Result<X509Bundle, AuthError> {
        let td_str = trust_domain.into();

        let c = self.client.as_mut().ok_or_else(|| {
            AuthError::ConfigError("WorkloadApiClient not initialized".to_string())
        })?;

        let bundles = c.fetch_x509_bundles().await.map_err(|e| {
            AuthError::ConfigError(format!("Failed to fetch all X509 bundles: {}", e))
        })?;

        let td = TrustDomain::new(&td_str).map_err(|e| {
            AuthError::ConfigError(format!("Invalid trust domain {}: {}", td_str, e))
        })?;

        bundles
            .get_bundle(&td)
            .cloned()
            .ok_or(AuthError::ConfigError(format!(
                "No X509 bundle for trust domain {}",
                td_str
            )))
    }

    /// Internal helper to access JWT bundles
    fn get_jwt_bundles(&self) -> Result<JwtBundleSet, AuthError> {
        let jwt_source = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;
        jwt_source
            .get_bundles()
            .map_err(|e| AuthError::ConfigError(format!("Failed to get JWT bundles: {}", e)))?
            .ok_or_else(|| AuthError::ConfigError("JWT bundles not yet available".to_string()))
    }
}

#[async_trait]
impl TokenProvider for SpireIdentityManager {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        self.initialize().await
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid()?;
        Ok(jwt_svid.token().to_string())
    }

    async fn get_token_with_claims(&self, custom_claims: MetadataMap) -> Result<String, AuthError> {
        if custom_claims.is_empty() {
            return self.get_token();
        }

        // Encode custom claims as a special audience
        let claims_audience = CustomClaimsCodec::encode_audience(&custom_claims)?;

        // Build audiences list with custom claims audience
        let mut audiences = self.jwt_audiences.clone();
        audiences.push(claims_audience);

        // Get the jwt_source
        let jwt_source = self
            .jwt_source
            .as_ref()
            .ok_or_else(|| AuthError::ConfigError("JwtSource not initialized".to_string()))?;

        jwt_source
            .fetch_with_custom_audiences(audiences, self.target_spiffe_id.clone())
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
    // Helper: sleep for duration or return true if cancelled first.
    async fn backoff_with_cancel(
        duration: Duration,
        cancellation_token: &CancellationToken,
    ) -> bool {
        tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = cancellation_token.cancelled() => true,
        }
    }

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
        let token_clone = cancellation_token.clone();
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
        let token_clone = cancellation_token.clone();
        tokio::spawn(async move {
            Self::stream_jwt_bundles(bundle_client, bundles_for_task, token_clone).await;
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
        let mut refresh_timer: std::pin::Pin<Box<tokio::time::Sleep>> = Box::pin(
            tokio::time::sleep_until(tokio::time::Instant::now() + initial_duration),
        );

        loop {
            tokio::select! {
                // Regular refresh (scheduled)
                _ = &mut refresh_timer => {
                    match Self::handle_regular_refresh(
                        &mut client,
                        &audiences,
                        target_spiffe_id.as_ref(),
                        &current,
                        &mut backoff,
                        &cfg,
                        &mut refresh_timer,
                    ).await {
                        Ok(()) => {
                            tracing::debug!(
                                "jwt_source: performed regular JWT SVID refresh - next refresh in {} s",
                                refresh_timer.as_ref().deadline().duration_since(tokio::time::Instant::now()).as_secs()
                            );
                        },
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
        refresh_timer: &mut std::pin::Pin<Box<tokio::time::Sleep>>,
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
                let next_duration = calculate_refresh_interval(&svid)?;

                let deadline = tokio::time::Instant::now() + next_duration;
                refresh_timer.as_mut().reset(deadline);

                tracing::debug!(
                    next_duration_secs = next_duration.as_secs(),
                    "jwt_source: next refresh scheduled at {:?} in {} seconds",
                    deadline,
                    next_duration.as_secs()
                );

                Ok(())
            }
            Err(err) => {
                tracing::warn!(error=%err, "jwt_source: failed to fetch JWT SVID; backing off");

                // Calculate exponential backoff, but cap it to prevent current token expiration
                let next_backoff = calculate_backoff_with_token_expiry(
                    *backoff,
                    current.read().as_ref(),
                    cfg.min_retry_backoff,
                );

                let deadline = tokio::time::Instant::now() + next_backoff;
                refresh_timer.as_mut().reset(deadline);
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
                Ok(mut stream) => {
                    // Stream consumption loop with a single outer select that always
                    // listens for cancellation alongside stream progress.
                    loop {
                        tokio::select! {
                            _ = cancellation_token.cancelled() => {
                                tracing::debug!("jwt_source: bundle streaming cancelled");
                                return;
                            }
                            item = stream.next() => {
                                match item {
                                    Some(Ok(bundle_set)) => {
                                        *bundles.write() = Some(bundle_set);
                                        tracing::trace!("jwt_source: updated JWT bundle cache");
                                    }
                                    Some(Err(e)) => {
                                        tracing::warn!(error=%e, "jwt_source: bundle stream error, restarting in 1s");
                                        if Self::backoff_with_cancel(Duration::from_secs(1), &cancellation_token).await {
                                            tracing::debug!("jwt_source: bundle streaming cancelled");
                                            return;
                                        }
                                        break;
                                    }
                                    None => {
                                        tracing::debug!("jwt_source: bundle stream ended, restarting in 1s");
                                        if Self::backoff_with_cancel(Duration::from_secs(1), &cancellation_token).await {
                                            tracing::debug!("jwt_source: bundle streaming cancelled");
                                            return;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
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

// Decode JWT expiry (seconds since epoch) without verifying signature and audience.
// Extracted as a standalone helper for reuse and unit testing.
fn decode_jwt_expiry_unverified(token: &str) -> Result<u64, AuthError> {
    let mut validation = jsonwebtoken_aws_lc::Validation::default();
    validation.insecure_disable_signature_validation();
    validation.validate_aud = false;

    let key = jsonwebtoken_aws_lc::DecodingKey::from_secret(&[]);
    let claims: TokenData<serde_json::Value> =
        jsonwebtoken_aws_lc::decode(token, &key, &validation).map_err(|e| {
            AuthError::TokenInvalid(format!("failed to extract claims from SVID: {}", e))
        })?;

    let exp_val = claims
        .claims
        .get("exp")
        .ok_or_else(|| AuthError::TokenInvalid("JWT SVID missing 'exp' claim".to_string()))?;

    if let Some(num) = exp_val.as_u64() {
        Ok(num)
    } else {
        exp_val.to_string().parse::<u64>().map_err(|_| {
            AuthError::TokenInvalid("JWT SVID 'exp' claim not a valid u64".to_string())
        })
    }
}

trait JwtLike {
    fn token(&self) -> &str;
}

impl JwtLike for JwtSvid {
    fn token(&self) -> &str {
        self.token()
    }
}

/// Calculate the next backoff duration, capping it to prevent token expiration
///
/// Returns the appropriate backoff duration considering:
/// - If token is expired: returns min_retry_backoff for immediate retry
/// - If token expires soon: caps backoff to 90% of remaining lifetime
/// - Otherwise: returns the requested backoff unchanged
fn calculate_backoff_with_token_expiry<T: JwtLike>(
    requested_backoff: Duration,
    current_token: Option<&T>,
    min_retry_backoff: Duration,
) -> Duration {
    let Some(token) = current_token else {
        return requested_backoff;
    };

    let Ok(expiry) = decode_jwt_expiry_unverified(token.token()) else {
        return requested_backoff;
    };

    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return requested_backoff;
    };

    if expiry > now.as_secs() {
        // Token not expired - cap backoff to 90% of remaining lifetime
        let remaining_lifetime = Duration::from_secs(expiry - now.as_secs());
        let max_safe_backoff = Duration::from_secs_f64(remaining_lifetime.as_secs_f64() * 0.9);

        if requested_backoff > max_safe_backoff {
            tracing::debug!(
                "jwt_source: capping backoff to {}s to prevent token expiration ({}s remaining)",
                max_safe_backoff.as_secs(),
                remaining_lifetime.as_secs()
            );
            max_safe_backoff
        } else {
            requested_backoff
        }
    } else {
        // Token expired - use minimum backoff for immediate retry
        tracing::warn!("jwt_source: current JWT SVID is already expired");
        min_retry_backoff
    }
}

/// Calculate refresh interval as 2/3 of the token's lifetime
fn calculate_refresh_interval<T: JwtLike>(jwt: &T) -> Result<Duration, AuthError> {
    const TWO_THIRDS: f64 = 2.0 / 3.0;
    let default = Duration::from_secs(30);

    let expiry = match decode_jwt_expiry_unverified(jwt.token()) {
        Ok(e) => e,
        Err(_) => {
            return Ok(default);
        }
    };

    if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        && expiry > now.as_secs()
    {
        let total_lifetime = Duration::from_secs(expiry - now.as_secs());
        let refresh_in = Duration::from_secs_f64(total_lifetime.as_secs_f64() * TWO_THIRDS);

        // Use a minimum of 100ms to handle very short-lived tokens (like 1-4 seconds)
        // but still respect the 2/3 lifetime principle
        let min_refresh = Duration::from_millis(100);
        return Ok(refresh_in.max(min_refresh));
    }

    Ok(default)
}

#[async_trait]
impl Verifier for SpireIdentityManager {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        self.initialize().await
    }

    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        let bundles = self.get_jwt_bundles()?;
        JwtSvid::parse_and_validate(&token.into(), &bundles, &self.jwt_audiences)
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
        let jwt_svid = JwtSvid::parse_and_validate(&token.into(), &bundles, &self.jwt_audiences)
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
        if let Some(obj) = claims_json.as_object_mut()
            && !custom_claims_map.is_empty()
        {
            obj.insert(
                "custom_claims".to_string(),
                Value::Object(custom_claims_map),
            );
        }

        serde_json::from_value(claims_json)
            .map_err(|e| AuthError::ConfigError(format!("Failed to deserialize JWT claims: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::calculate_backoff_with_token_expiry;
    use super::calculate_refresh_interval;
    use super::decode_jwt_expiry_unverified;
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    // Helper to build a JWT with a specific exp claim (numeric or string) using jsonwebtoken_aws_lc.
    fn build_token_with_exp(exp_value: serde_json::Value) -> String {
        use jsonwebtoken_aws_lc::{EncodingKey, Header};
        use serde_json::Value;
        let mut payload_map = serde_json::Map::new();
        if exp_value != Value::Null {
            payload_map.insert("exp".to_string(), exp_value);
        }
        let payload = Value::Object(payload_map);
        jsonwebtoken_aws_lc::encode(&Header::default(), &payload, &EncodingKey::from_secret(&[]))
            .expect("token encoding should succeed")
    }

    #[test]
    fn test_decode_expiry_numeric() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = build_token_with_exp(json!(now + 60));
        let exp = decode_jwt_expiry_unverified(&token).expect("should decode numeric exp");
        assert_eq!(exp, now + 60);
    }

    #[test]
    fn test_decode_expiry_string() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = build_token_with_exp(json!((now + 120)));
        let exp = decode_jwt_expiry_unverified(&token).expect("should decode string exp");
        assert_eq!(exp, now + 120);
    }

    #[test]
    fn test_decode_expiry_missing() {
        let token = build_token_with_exp(serde_json::Value::Null); // omit exp
        assert!(
            decode_jwt_expiry_unverified(&token).is_err(),
            "missing exp should error"
        );
    }

    #[test]
    fn test_decode_expiry_invalid() {
        let token = build_token_with_exp(json!("not-a-number"));
        assert!(
            decode_jwt_expiry_unverified(&token).is_err(),
            "invalid exp should error"
        );
    }

    #[test]
    fn test_calculate_refresh_interval_basic() {
        use std::time::{SystemTime, UNIX_EPOCH};
        // token with 90s lifetime
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = build_token_with_exp(json!(now + 90));
        struct DummyJwt(String);
        impl super::JwtLike for DummyJwt {
            fn token(&self) -> &str {
                &self.0
            }
        }
        let dummy = DummyJwt(token);
        let dur = calculate_refresh_interval(&dummy).expect("interval");
        // Expect roughly 60s (2/3 of 90s) allowing small timing variance
        assert!(
            dur >= Duration::from_secs(58) && dur <= Duration::from_secs(61),
            "expected ~60s, got {:?}",
            dur
        );
    }

    #[test]
    fn test_calculate_refresh_interval_expired_defaults() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = build_token_with_exp(json!(now - 10));
        struct DummyJwt(String);
        impl super::JwtLike for DummyJwt {
            fn token(&self) -> &str {
                &self.0
            }
        }
        let dummy = DummyJwt(token);
        let dur = calculate_refresh_interval(&dummy).expect("interval");
        assert_eq!(
            dur,
            Duration::from_secs(30),
            "expired token should use default 30s"
        );
    }

    // Helper to build a token that JwtSvid::parse_insecure will accept (supported alg, kid, typ)
    fn build_svid_like_token(exp: u64, aud: Vec<String>, sub: &str) -> String {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use serde_json::json;

        let header = json!({"alg":"RS256","typ":"JWT","kid":"kid1"});
        let claims = json!({
            "sub": sub,
            "aud": aud,
            "exp": exp,
        });

        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let claims_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        // Empty signature part is fine because we disable signature validation for parse_insecure
        format!("{}.{}.", header_b64, claims_b64)
    }

    #[test]
    fn test_calculate_refresh_interval_real_jwtsvid() {
        use std::time::{SystemTime, UNIX_EPOCH};

        // Create a token with 90s lifetime
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let lifetime = 90u64;
        let token = build_svid_like_token(
            now + lifetime,
            vec!["audA".to_string(), "audB".to_string()],
            "spiffe://example.org/service",
        );

        // Parse insecurely into a real JwtSvid
        let svid = token
            .parse::<spiffe::JwtSvid>()
            .expect("JwtSvid::parse_insecure should succeed for crafted token");

        let dur = calculate_refresh_interval(&svid).expect("interval");
        // Expect roughly 60s (2/3 of 90s), allow a little drift due to test timing.
        assert!(
            dur >= Duration::from_secs(58) && dur <= Duration::from_secs(61),
            "expected ~60s refresh interval, got {:?}",
            dur
        );
    }

    #[test]
    fn test_calculate_refresh_interval_real_jwtsvid_expired() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Expired 10s ago
        let token = build_svid_like_token(
            now - 10,
            vec!["aud".to_string()],
            "spiffe://example.org/service",
        );

        // Parse insecurely into a real JwtSvid
        let svid = token
            .parse::<spiffe::JwtSvid>()
            .expect("JwtSvid::parse_insecure should succeed for crafted token");

        let dur = calculate_refresh_interval(&svid).expect("interval");
        assert_eq!(
            dur,
            Duration::from_secs(30),
            "expired token should return default 30s interval"
        );
    }

    #[test]
    fn test_backoff_with_expired_token_retries_immediately() {
        // Create an expired token (expired 10 seconds ago)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expired_token = build_svid_like_token(
            now - 10,
            vec!["aud".to_string()],
            "spiffe://example.org/service",
        );
        let expired_svid = expired_token
            .parse::<spiffe::JwtSvid>()
            .expect("JwtSvid::parse_insecure should succeed");

        // Simulate a large backoff that would normally be used
        let large_backoff = Duration::from_secs(60);

        // Simulate min_retry_backoff
        let min_backoff = Duration::from_secs(1);

        // Calculate what the next backoff should be given an expired token
        let next_backoff =
            calculate_backoff_with_token_expiry(large_backoff, Some(&expired_svid), min_backoff);

        // Verify that with an expired token, we retry with minimal backoff
        assert_eq!(
            next_backoff,
            min_backoff,
            "expired token should trigger immediate retry with min backoff, not {}s",
            large_backoff.as_secs()
        );
    }

    #[test]
    fn test_backoff_capped_to_token_lifetime() {
        // Create a token that expires in 10 seconds
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let short_lived_token = build_svid_like_token(
            now + 10,
            vec!["aud".to_string()],
            "spiffe://example.org/service",
        );
        let short_lived_svid = short_lived_token
            .parse::<spiffe::JwtSvid>()
            .expect("JwtSvid::parse_insecure should succeed");

        // Simulate a large backoff (60 seconds) that exceeds token lifetime
        let large_backoff = Duration::from_secs(60);
        let min_backoff = Duration::from_secs(1);

        // Calculate what the next backoff should be
        let next_backoff = calculate_backoff_with_token_expiry(
            large_backoff,
            Some(&short_lived_svid),
            min_backoff,
        );

        // Backoff should be capped to ~9 seconds (90% of 10 seconds remaining)
        assert!(
            next_backoff <= Duration::from_secs(9),
            "backoff should be capped to token lifetime, got {:?}",
            next_backoff
        );
        assert!(
            next_backoff >= Duration::from_secs(8),
            "backoff should be close to 90% of remaining lifetime, got {:?}",
            next_backoff
        );
    }
}

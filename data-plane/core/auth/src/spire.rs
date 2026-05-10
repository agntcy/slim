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
//! use slim_auth::spire::SpireIdentityManager;
//! use slim_auth::traits::{TokenProvider, Verifier};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut mgr = SpireIdentityManager::builder()
//!     .with_jwt_audiences(vec!["my-app".into()])
//!     .build();
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
//! This unified design replaced the previous split between
//! `SpiffeProvider` and `SpiffeJwtVerifier`.

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use display_error_chain::ErrorChainExt;
use jsonwebtoken::TokenData;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde_json::{self, Value};
use spiffe::{
    JwtBundleSet, JwtSource as SpiffeJwtSource, JwtSourceBuilder as SpiffeJwtSourceBuilder,
    JwtSvid, SpiffeId, TrustDomain, WorkloadApiClient, X509Bundle, X509Source, X509SourceBuilder,
    X509Svid, bundle::BundleSource,
};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, info, warn};

use crate::errors::AuthError;
use crate::identity_claims::IdentityClaims;
use crate::metadata::MetadataMap;
use crate::traits::{TokenProvider, Verifier};
use crate::utils::{bytes_to_pem, generate_mls_signature_keys};

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
        let claims_json = serde_json::to_string(custom_claims)?;

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
                        tracing::warn!(error = %e.chain(), "Failed to decode custom claims base64");
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

/// Thin caching wrapper around the library's [`SpiffeJwtSource`] that provides
/// **sync** access to a background-refreshed JWT SVID, mirroring how
/// [`X509Source`] exposes X.509 SVIDs.
///
/// The library's `JwtSource` handles:
/// - Workload API connection management with automatic reconnection
/// - Background streaming and caching of JWT bundles
/// - On-demand JWT SVID fetching with retry
///
/// This wrapper adds:
/// - A background task that periodically fetches a fresh JWT SVID and caches it
/// - Sync `get_svid()` for the `TokenProvider::get_token()` contract
#[derive(Clone)]
struct CachedJwtSvid {
    /// Library JWT source — manages bundles + on-demand SVID fetching.
    source: SpiffeJwtSource,
    /// Background-refreshed SVID cache for sync access.
    cached_svid: Arc<RwLock<Option<JwtSvid>>>,
    /// Cancellation token for the background refresh task.
    cancellation_token: CancellationToken,
}

impl CachedJwtSvid {
    /// Create a new `CachedJwtSvid` from a ready library `JwtSource`.
    ///
    /// Performs an initial SVID fetch, then spawns a background task that
    /// refreshes at ≈2/3 of the token lifetime.
    async fn new(
        source: SpiffeJwtSource,
        audiences: Vec<String>,
        target_spiffe_id: Option<String>,
    ) -> Result<Self, AuthError> {
        let cached_svid = Arc::new(RwLock::new(None));
        let cancellation_token = CancellationToken::new();

        let parsed_target: Option<SpiffeId> =
            target_spiffe_id.as_deref().map(|s| s.parse()).transpose()?;

        // Initial fetch — must succeed so callers can use get_token() immediately after
        // initialize() returns Ok. The background task will keep the cache fresh afterwards.
        match source
            .get_jwt_svid_with_id(&audiences, parsed_target.as_ref())
            .await
        {
            Ok(svid) => {
                *cached_svid.write() = Some(svid);
            }
            Err(err) => {
                return Err(AuthError::SpiffeJwtSourceError(err));
            }
        }

        // Spawn background SVID refresh task.
        // Propagate the current tracing span so that logs from the background task
        // remain associated with the caller's span (important for test log capture).
        let bg_source = source.clone();
        let bg_cache = cached_svid.clone();
        let bg_cancel = cancellation_token.clone();
        let bg_span = tracing::Span::current();
        tokio::spawn(
            async move {
                Self::background_refresh(bg_source, audiences, parsed_target, bg_cache, bg_cancel)
                    .await;
            }
            .instrument(bg_span),
        );

        Ok(Self {
            source,
            cached_svid,
            cancellation_token,
        })
    }

    /// Background task: periodically fetches a fresh JWT SVID and updates the
    /// cache.  Refresh interval is ≈2/3 of the token lifetime with exponential
    /// backoff on failure (capped to avoid letting the token expire).
    async fn background_refresh(
        source: SpiffeJwtSource,
        audiences: Vec<String>,
        target_spiffe_id: Option<SpiffeId>,
        cache: Arc<RwLock<Option<JwtSvid>>>,
        cancel: CancellationToken,
    ) {
        let min_backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);
        let mut backoff = min_backoff;
        let initial_delay = Duration::from_secs(30);

        // pin next refresh on the stack
        let mut next_refresh = std::pin::pin!(tokio::time::sleep_until(
            tokio::time::Instant::now() + initial_delay
        ));

        loop {
            tokio::select! {
                _ = &mut next_refresh => {}
                _ = cancel.cancelled() => {
                    debug!("jwt_source: background refresh cancelled");
                    return;
                }
            }

            match source
                .get_jwt_svid_with_id(&audiences, target_spiffe_id.as_ref())
                .await
            {
                Ok(svid) => {
                    let delay =
                        calculate_refresh_interval(&svid).unwrap_or(Duration::from_secs(30));
                    *cache.write() = Some(svid);
                    backoff = min_backoff;
                    let deadline = tokio::time::Instant::now() + delay;
                    next_refresh.as_mut().reset(deadline);
                    debug!(
                        next_refresh_secs = delay.as_secs(),
                        "jwt_source: SVID refreshed",
                    );
                }
                Err(err) => {
                    warn!(error = %err, "jwt_source: SVID refresh failed; backing off");
                    let capped = calculate_backoff_with_token_expiry(
                        backoff,
                        cache.read().as_ref(),
                        min_backoff,
                    );
                    let deadline = tokio::time::Instant::now() + capped;
                    next_refresh.as_mut().reset(deadline);
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }

    /// Sync access to the most recently cached JWT SVID.
    fn get_svid(&self) -> Option<JwtSvid> {
        self.cached_svid.read().clone()
    }

    /// Sync access to the current JWT bundle set (delegated to the library source).
    fn get_bundles(&self) -> Result<Arc<JwtBundleSet>, AuthError> {
        self.source
            .bundle_set()
            .map_err(AuthError::SpiffeJwtSourceError)
    }
}

impl Drop for CachedJwtSvid {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
    }
}

/// Builder for constructing a SpireIdentityManager
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

    pub fn build(self) -> Result<SpireIdentityManager, crate::errors::AuthError> {
        let signature_keys = generate_mls_signature_keys()?;
        let internal = SpireIdentityManagerInternal {
            socket_path: self.socket_path,
            target_spiffe_id: self.target_spiffe_id,
            jwt_audiences: self.jwt_audiences,
            x509_source: RwLock::new(None),
            jwt_source: RwLock::new(None),
        };
        Ok(SpireIdentityManager {
            inner: Arc::new(internal),
            signature_keys,
        })
    }
}

/// Shared internal state for [`SpireIdentityManager`].
/// Wrapped in `Arc` so cloning only increments the reference count.
/// Mutable sources use `RwLock` for interior mutability after construction.
struct SpireIdentityManagerInternal {
    socket_path: Option<String>,
    target_spiffe_id: Option<String>,
    jwt_audiences: Vec<String>,
    x509_source: RwLock<Option<X509Source>>,
    jwt_source: RwLock<Option<Arc<CachedJwtSvid>>>,
}

/// SPIFFE certificate and JWT provider that automatically rotates credentials.
///
/// Cloning shares all internal state (sources, config) via `Arc` but gives each
/// clone its own independent MLS signature key pair, mirroring `SharedSecret`.
#[derive(Clone)]
pub struct SpireIdentityManager {
    inner: Arc<SpireIdentityManagerInternal>,
    /// MLS Ed25519 signature key pair: (secret_key_bytes, public_key_bytes).
    /// Plain field so each clone owns an independent copy.
    signature_keys: (Vec<u8>, Vec<u8>),
}

impl SpireIdentityManager {
    /// Convenience: start building a new SpireIdentityManager
    pub fn builder() -> SpireIdentityManagerBuilder {
        SpireIdentityManagerBuilder::new()
    }

    /// Build the full JWT audience list, including the MLS public key encoded as a
    /// custom-claim audience. This ensures every cached SVID from the JwtSource
    /// has the pubkey embedded so `get_token()` returns a token that passes
    /// `IdentityClaims::from_json` (which looks for `custom_claims.pubkey`).
    fn jwt_audiences_with_pubkey(&self) -> Result<Vec<String>, AuthError> {
        let pubkey_claims = IdentityClaims::from_public_key_bytes(&self.signature_keys.1);
        let pubkey_audience = CustomClaimsCodec::encode_audience(&pubkey_claims)?;
        let mut audiences = self.inner.jwt_audiences.clone();
        audiences.push(pubkey_audience);
        Ok(audiences)
    }

    /// Build a fresh [`CachedJwtSvid`] backed by the library's [`SpiffeJwtSource`].
    async fn build_jwt_source(
        audiences: Vec<String>,
        target_spiffe_id: Option<String>,
        socket_path: Option<String>,
    ) -> Result<CachedJwtSvid, AuthError> {
        let mut builder = SpiffeJwtSourceBuilder::new();
        if let Some(path) = socket_path.as_ref() {
            builder = builder.endpoint(path);
        }
        let source = builder.build().await?;
        CachedJwtSvid::new(source, audiences, target_spiffe_id).await
    }

    /// Initialize the spire identity manager (sources for X.509 & JWT)
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        info!("Initializing spire identity manager");

        // Quick-fail: verify the Workload API is reachable before creating
        // long-lived sources whose internal retry loops never give up.
        // The client is intentionally dropped after the check.
        let _client = match self.inner.socket_path.as_ref() {
            Some(path) => WorkloadApiClient::connect_to(path).await?,
            None => WorkloadApiClient::connect_env().await?,
        };
        drop(_client);

        // Initialize X509Source for certificate management
        let mut x509_builder = X509SourceBuilder::new();
        if let Some(path) = self.inner.socket_path.as_ref() {
            x509_builder = x509_builder.endpoint(path);
        }
        let x509_source = x509_builder.build().await?;

        *self.inner.x509_source.write() = Some(x509_source);

        // Initialize JwtSource (library) + cached SVID wrapper with audiences
        // that include the MLS pubkey as a custom-claim audience so every
        // cached SVID carries it transparently.
        let jwt_audiences = self.jwt_audiences_with_pubkey()?;
        let jwt_source = Self::build_jwt_source(
            jwt_audiences,
            self.inner.target_spiffe_id.clone(),
            self.inner.socket_path.clone(),
        )
        .await?;

        *self.inner.jwt_source.write() = Some(Arc::new(jwt_source));

        info!("spire provider initialized successfully");

        Ok(())
    }

    /// Get the current X.509 SVID (leaf cert + key)
    pub fn get_x509_svid(&self) -> Result<X509Svid, AuthError> {
        let guard = self.inner.x509_source.read();
        let x509_source = guard
            .as_ref()
            .ok_or(AuthError::SpiffeX509SourceNotInitialized)?;
        let svid = x509_source.svid()?;
        debug!(spiffe_id = %svid.spiffe_id(), "Retrieved X509 SVID");
        Ok((*svid).clone())
    }

    /// Get the X.509 certificate (leaf) in PEM format
    pub fn get_x509_cert_pem(&self) -> Result<String, AuthError> {
        let svid = self.get_x509_svid()?;
        let cert_chain = svid.cert_chain();

        if cert_chain.is_empty() {
            return Err(AuthError::SpiffeX509EmptyCertChain);
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
        let guard = self.inner.jwt_source.read();
        let cached = guard
            .as_ref()
            .ok_or(AuthError::SpiffeJwtSourceNotInitialized)?;
        cached.get_svid().ok_or(AuthError::SpiffeJwtSvidMissing)
    }

    /// Get X.509 bundle for the trust domain of our SVID (for verification use-cases)
    pub fn get_x509_bundle(&self) -> Result<X509Bundle, AuthError> {
        let guard = self.inner.x509_source.read();
        let x509_source = guard
            .as_ref()
            .ok_or(AuthError::SpiffeX509SourceNotInitialized)?;

        // Derive trust domain from current SVID
        let svid = x509_source.svid()?;

        let td = svid.spiffe_id().trust_domain();

        x509_source
            .bundle_for_trust_domain(td)?
            .map(|arc| (*arc).clone())
            .ok_or(AuthError::SpiffeX509BundleMissing(td.clone()))
    }

    /// Get the X.509 bundle for an explicit trust domain.
    ///
    /// Uses the cached bundle set from the [`X509Source`], which streams
    /// bundles for all trust domains in the background.
    pub fn get_x509_bundle_for_trust_domain(
        &self,
        trust_domain: impl Into<String>,
    ) -> Result<X509Bundle, AuthError> {
        let td_str = trust_domain.into();
        let td = TrustDomain::new(&td_str)?;

        let guard = self.inner.x509_source.read();
        let x509_source = guard
            .as_ref()
            .ok_or(AuthError::SpiffeX509SourceNotInitialized)?;

        x509_source
            .bundle_for_trust_domain(&td)?
            .map(|arc| (*arc).clone())
            .ok_or(AuthError::SpiffeX509BundleMissing(td))
    }

    /// Internal helper to access JWT bundles from the library's `JwtSource`.
    fn get_jwt_bundles(&self) -> Result<Arc<JwtBundleSet>, AuthError> {
        let guard = self.inner.jwt_source.read();
        guard
            .as_ref()
            .ok_or(AuthError::SpiffeJwtSourceNotInitialized)?
            .get_bundles()
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

    fn get_id(&self) -> Result<String, AuthError> {
        let jwt_svid = self.get_jwt_svid()?;
        Ok(jwt_svid.spiffe_id().to_string())
    }

    fn get_signature_secret_key(&self) -> Result<Vec<u8>, AuthError> {
        Ok(self.signature_keys.0.clone())
    }

    fn get_signature_public_key(&self) -> Result<Vec<u8>, AuthError> {
        Ok(self.signature_keys.1.clone())
    }

    fn rotate_signature_keys(&mut self) -> Result<(), AuthError> {
        self.signature_keys = generate_mls_signature_keys()?;

        // Rebuild the CachedJwtSvid so the next get_token() call returns a fresh
        // SVID with the new pubkey embedded in its audiences.
        let new_audiences = self.jwt_audiences_with_pubkey()?;
        let target_spiffe_id = self.inner.target_spiffe_id.clone();
        let socket_path = self.inner.socket_path.clone();

        let new_jwt_source = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(Self::build_jwt_source(
                new_audiences,
                target_spiffe_id,
                socket_path,
            ))
        })?;

        *self.inner.jwt_source.write() = Some(Arc::new(new_jwt_source));

        Ok(())
    }
}

// Decode JWT expiry (seconds since epoch) without verifying signature and audience.
// Extracted as a standalone helper for reuse and unit testing.
fn decode_jwt_expiry_unverified(token: &str) -> Result<u64, AuthError> {
    let claims: TokenData<serde_json::Value> = jsonwebtoken::dangerous::insecure_decode(token)?;
    let exp_val = claims
        .claims
        .get("exp")
        .ok_or(AuthError::TokenInvalidMissingExp)?;

    if let Some(num) = exp_val.as_u64() {
        Ok(num)
    } else {
        exp_val
            .to_string()
            .parse::<u64>()
            .map_err(|_| AuthError::TokenInvalidMissingExp)
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
                max_safe_backoff = %max_safe_backoff.as_secs(),
                remaining_lifetime = %remaining_lifetime.as_secs(),
                "jwt_source: capping backoff to prevent token expiration",
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
        JwtSvid::parse_and_validate(&token.into(), &*bundles, &self.inner.jwt_audiences)?;
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
            JwtSvid::parse_and_validate(&token.into(), &*bundles, &self.inner.jwt_audiences)?;

        debug!(
            spiffe_id = %jwt_svid.spiffe_id(),
            "Successfully extracted claims"        );

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

        let res = serde_json::from_value(claims_json)?;

        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::calculate_backoff_with_token_expiry;
    use super::calculate_refresh_interval;
    use super::decode_jwt_expiry_unverified;
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    // Helper to build a JWT with a specific exp claim (numeric or string) using jsonwebtoken.
    fn build_token_with_exp(exp_value: serde_json::Value) -> String {
        use jsonwebtoken::{EncodingKey, Header};
        use serde_json::Value;
        let mut payload_map = serde_json::Map::new();
        if exp_value != Value::Null {
            payload_map.insert("exp".to_string(), exp_value);
        }
        let payload = Value::Object(payload_map);
        jsonwebtoken::encode(&Header::default(), &payload, &EncodingKey::from_secret(&[]))
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

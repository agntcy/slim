/*
Copyright AGNTCY Contributors (https://github.com/agntcy)
SPDX-License-Identifier: Apache-2.0
*/

//! Shared-secret based token generation and verification.
//!
//! This implementation now supports an optionally enabled replay cache.
//! IMPORTANT: Replay prevention is DISABLED by default. You must explicitly
//! enable it using `with_replay_cache_enabled(max_entries)` if you require
//! replay detection.
//!
//! A token encodes: `<id>:<unix_timestamp>:<nonce>:<claims_b64url>:<mac_b64url>`
//! Where claims_b64url is empty if no custom claims are present.
//!
//! Security properties enforced (when replay cache enabled):
//! * Authenticity & integrity: HMAC over `id:timestamp:nonce:claims` (claims can be empty).
//! * Expiration: bounded by `validity_window`.
//! * Clock skew tolerance: bounded by `clock_skew`.
//! * Replay prevention: (nonce, timestamp) cached until expiration.
//!
//! Design notes:
//! * `id` is randomized per construction (`<base_id>_<random_suffix>`).
//! * Replay cache stores only (nonce, timestamp) for memory efficiency.
//! * HMAC via `aws-lc-rs` for constant-time primitives.
//! * `SharedSecret` is cheap to clone (Arc increment) and cloning preserves
//!   replay cache state (when enabled).
//!
//! Typical usage (no replay protection):
//! ```ignore
//! let auth = SharedSecret::new("service", secret_string)?;
//! let token = auth.get_token()?;
//! auth.try_verify(&token)?;
//! ```
//!
//! Enabling replay protection:
//! ```ignore
//! let auth = SharedSecret::new("service", secret_string)?
//!     .with_replay_cache_enabled(4096);
//! let token = auth.get_token()?;
//! auth.try_verify(&token)?; // second verify of same token will fail
//! ```
//!
//! Builder-style adjustments (`with_*`) return a new `SharedSecret` whose
//! replay cache state is preserved IF replay protection is enabled. When
//! disabled, replay-related builders either enable (`with_replay_cache_enabled`)
//! or leave it disabled (`with_replay_cache_disabled`).
//!
//! Thread safety:
//! * Interior mutability only for the replay cache (parking_lot::Mutex).
//! * All other fields are immutable after construction.

use async_trait::async_trait;
use aws_lc_rs::hmac;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as STANDARD_BASE64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use parking_lot::Mutex;
use rand::{Rng, distr::Alphanumeric};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
    utils::generate_mls_signature_keys,
};

/// Minimum length (in bytes) required for the shared secret (baseline 256 bits).
const MIN_SECRET_LEN: usize = 32;
/// Raw nonce byte length before base64url encoding.
const NONCE_LEN: usize = 12;
/// Default validity window (seconds).
const DEFAULT_VALIDITY_WINDOW: u64 = 3600;
/// Default tolerated forward clock skew (seconds).
const DEFAULT_CLOCK_SKEW: u64 = 5;
/// Default maximum replay cache entries (used if enabled without override).
const DEFAULT_REPLAY_CACHE_MAX: usize = 4096;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct ReplayEntry {
    nonce: String,
    timestamp: u64,
}

#[derive(Debug, Clone)]
struct ReplayCache {
    entries: HashSet<ReplayEntry>,
    order: VecDeque<ReplayEntry>,
    max_size: usize,
}

impl ReplayCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: HashSet::with_capacity(max_size),
            order: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    fn clone_preserving(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            order: self.order.clone(),
            max_size: self.max_size,
        }
    }

    /// Insert new (nonce, timestamp) and enforce:
    /// 1. Expire old entries (age > validity_window).
    /// 2. Reject replays.
    /// 3. Evict oldest if at capacity.
    fn insert(
        &mut self,
        entry: ReplayEntry,
        now: u64,
        validity_window: u64,
    ) -> Result<(), AuthError> {
        // Purge expired
        while let Some(front) = self.order.front() {
            if now.saturating_sub(front.timestamp) > validity_window {
                if let Some(expired) = self.order.pop_front() {
                    self.entries.remove(&expired);
                }
            } else {
                break;
            }
        }

        // Replay detection
        if self.entries.contains(&entry) {
            return Err(AuthError::TokenInvalidReplay);
        }

        // Eviction at capacity
        if self.entries.len() >= self.max_size
            && let Some(old) = self.order.pop_front()
        {
            self.entries.remove(&old);
        }

        self.entries.insert(entry.clone());
        self.order.push_back(entry);
        Ok(())
    }
}

struct SharedSecretInternal {
    base_id: String,
    id: String,
    shared_secret: String,
    /// Precomputed HMAC key derived once from `shared_secret`.
    hmac_key: hmac::Key,
    validity_window: std::time::Duration,
    clock_skew: std::time::Duration,
    replay_cache_enabled: bool,
    replay_cache: Mutex<ReplayCache>,
}

impl std::fmt::Debug for SharedSecretInternal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSecretInternal")
            .field("base_id", &self.base_id)
            .field("id", &self.id)
            .field("validity_window", &self.validity_window)
            .field("clock_skew", &self.clock_skew)
            .field("replay_cache_enabled", &self.replay_cache_enabled)
            .finish()
    }
}

/// Public wrapper holding an Arc to internal implementation.
/// Cloning shares the replay cache and config but gives each clone
/// its own independent MLS signature key pair.
#[derive(Clone)]
pub struct SharedSecret {
    inner: Arc<SharedSecretInternal>,
    /// MLS Ed25519 signature key pair: (secret_key_bytes, public_key_bytes).
    /// Plain field so each clone owns an independent copy.
    signature_keys: (Vec<u8>, Vec<u8>),
    /// Precomputed URL_SAFE_NO_PAD(base64) of the claims JSON embedding the MLS public key.
    /// Only changes when the signature keys are rotated.
    claims_b64: String,
}

impl std::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSecret")
            .field("base_id", &self.inner.base_id)
            .field("id", &self.inner.id)
            .field(
                "validity_window_secs",
                &self.inner.validity_window.as_secs(),
            )
            .field("clock_skew_secs", &self.inner.clock_skew.as_secs())
            .field("replay_cache_enabled", &self.inner.replay_cache_enabled)
            .field("replay_cache_max", &self.inner.replay_cache.lock().max_size)
            .field("has_signature_keys", &true)
            .finish()
    }
}

impl SharedSecret {
    /// Build the URL_SAFE_NO_PAD(base64) of the claims JSON embedding the MLS pubkey.
    fn compute_claims_b64(pub_key: &[u8]) -> String {
        let pub_key_b64 = STANDARD_BASE64.encode(pub_key);
        let claims_json = serde_json::json!({ "pubkey": pub_key_b64 }).to_string();
        URL_SAFE_NO_PAD.encode(claims_json.as_bytes())
    }

    /// Construct a new shared secret instance with randomized `id` suffix.
    /// Replay protection starts DISABLED.
    pub fn new(id: &str, shared_secret: &str) -> Result<Self, AuthError> {
        Self::validate_id(id)?;
        Self::validate_secret(shared_secret)?;

        let random_suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let full_id = format!("{}_{}", id, random_suffix);

        let signature_keys = generate_mls_signature_keys()?;
        let claims_b64 = Self::compute_claims_b64(&signature_keys.1);
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, shared_secret.as_bytes());
        let internal = SharedSecretInternal {
            base_id: id.to_owned(),
            id: full_id,
            shared_secret: shared_secret.to_owned(),
            hmac_key,
            validity_window: std::time::Duration::from_secs(DEFAULT_VALIDITY_WINDOW),
            clock_skew: std::time::Duration::from_secs(DEFAULT_CLOCK_SKEW),
            replay_cache_enabled: false,
            replay_cache: Mutex::new(ReplayCache::new(DEFAULT_REPLAY_CACHE_MAX)),
        };
        Ok(SharedSecret {
            inner: Arc::new(internal),
            signature_keys,
            claims_b64,
        })
    }

    /// Enable replay cache with specified maximum size.
    /// If already enabled, updates capacity while preserving existing entries
    /// (evicting oldest if shrinking).
    pub fn with_replay_cache_enabled(&self, max_size: usize) -> Self {
        self.rebuild(None, None, Some(max_size), Some(true))
    }

    /// Disable replay cache (replay detection no longer enforced).
    /// Existing cached entries are retained internally but ignored.
    pub fn with_replay_cache_disabled(&self) -> Self {
        self.rebuild(None, None, None, Some(false))
    }

    /// Returns a new instance with updated validity window.
    pub fn with_validity_window(&self, window: std::time::Duration) -> Self {
        self.rebuild(Some(window), None, None, None)
    }

    /// Returns a new instance with updated clock skew.
    pub fn with_clock_skew(&self, skew: std::time::Duration) -> Self {
        self.rebuild(None, Some(skew), None, None)
    }

    /// Returns a new instance with updated replay cache max capacity (only if enabled).
    pub fn with_replay_cache_max(&self, max_size: usize) -> Self {
        if !self.inner.replay_cache_enabled {
            // Replay protection disabled; capacity change has no effect.
            return self.clone();
        }
        self.rebuild(None, None, Some(max_size), None)
    }

    /// Internal rebuild helper preserving replay cache state when enabled.
    fn rebuild(
        &self,
        validity_window: Option<std::time::Duration>,
        clock_skew: Option<std::time::Duration>,
        replay_cache_max: Option<usize>,
        replay_cache_enabled: Option<bool>,
    ) -> Self {
        let current = &self.inner;
        let enable_flag = replay_cache_enabled.unwrap_or(current.replay_cache_enabled);

        // Snapshot existing cache
        let cache_guard = current.replay_cache.lock();
        let mut cloned_cache = if current.replay_cache_enabled {
            cache_guard.clone_preserving()
        } else {
            // If we are enabling now and previously disabled, start empty.
            if enable_flag {
                ReplayCache::new(replay_cache_max.unwrap_or(DEFAULT_REPLAY_CACHE_MAX))
            } else {
                cache_guard.clone_preserving() // unused but keep structure
            }
        };
        drop(cache_guard);

        // If capacity changed, enforce size limit by evicting oldest
        if let Some(new_max) = replay_cache_max {
            cloned_cache.max_size = new_max;
            while cloned_cache.entries.len() > new_max {
                if let Some(front) = cloned_cache.order.pop_front() {
                    cloned_cache.entries.remove(&front);
                } else {
                    break;
                }
            }
        }

        let internal = SharedSecretInternal {
            base_id: current.base_id.clone(),
            id: current.id.clone(),
            shared_secret: current.shared_secret.clone(),
            hmac_key: hmac::Key::new(hmac::HMAC_SHA256, current.shared_secret.as_bytes()),
            validity_window: validity_window.unwrap_or(current.validity_window),
            clock_skew: clock_skew.unwrap_or(current.clock_skew),
            replay_cache_enabled: enable_flag,
            replay_cache: Mutex::new(cloned_cache),
        };
        SharedSecret {
            inner: Arc::new(internal),
            signature_keys: self.signature_keys.clone(),
            claims_b64: self.claims_b64.clone(),
        }
    }

    /// Issue a token into a caller-owned buffer, avoiding the per-call allocation
    /// of `get_token`. The buffer is cleared first and grown to fit.
    ///
    /// Useful in tight token-issuance loops where the same buffer can be reused.
    pub fn get_token_into(&self, out: &mut String) -> Result<(), AuthError> {
        if self.inner.shared_secret.is_empty() {
            return Err(AuthError::HmacKeyMissing);
        }

        let ts = self.get_current_timestamp();

        // Generate nonce directly on the stack (no intermediate String).
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut nonce_bytes);

        let id = self.id();
        let claims_b64 = self.claims_b64.as_str();

        // Pre-sized buffer: id + ':' + ts(<=20) + ':' + nonce_b64(16) + ':' + claims_b64 + ':' + mac_b64(43).
        let cap = id.len() + 1 + 20 + 1 + 16 + 1 + claims_b64.len() + 1 + 43;
        out.clear();
        out.reserve(cap.saturating_sub(out.capacity()));
        out.push_str(id);
        out.push(':');
        let mut itoa_buf = itoa::Buffer::new();
        out.push_str(itoa_buf.format(ts));
        out.push(':');
        URL_SAFE_NO_PAD.encode_string(nonce_bytes, out);
        out.push(':');
        out.push_str(claims_b64);

        // Sign the canonical message "id:ts:nonce:claims_b64" which is the current buffer.
        let tag = hmac::sign(&self.inner.hmac_key, out.as_bytes());
        out.push(':');
        URL_SAFE_NO_PAD.encode_string(tag.as_ref(), out);

        Ok(())
    }

    /// Get the randomized unique identifier.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Base identifier (without random suffix).
    pub fn base_id(&self) -> &str {
        &self.inner.base_id
    }

    /// Raw shared secret (avoid logging).
    pub fn shared_secret(&self) -> &str {
        &self.inner.shared_secret
    }

    /// Validity window duration.
    pub fn validity_window(&self) -> std::time::Duration {
        self.inner.validity_window
    }

    /// Validity window in seconds (helper for tests / metrics).
    pub fn validity_window_secs(&self) -> u64 {
        self.inner.validity_window.as_secs()
    }

    /// Clock skew duration.
    pub fn clock_skew(&self) -> std::time::Duration {
        self.inner.clock_skew
    }

    /// Replay cache enabled?
    pub fn replay_cache_enabled(&self) -> bool {
        self.inner.replay_cache_enabled
    }

    /// Replay cache max size (even if disabled).
    pub fn replay_cache_max(&self) -> usize {
        self.inner.replay_cache.lock().max_size
    }

    /// Validate identifier format.
    fn validate_id(id: &str) -> Result<(), AuthError> {
        if id.is_empty() {
            return Err(AuthError::TokenMalformed);
        }
        if id.contains(':') {
            return Err(AuthError::TokenMalformed);
        }
        if id.chars().any(|c| c.is_whitespace()) {
            return Err(AuthError::TokenMalformed);
        }

        Ok(())
    }

    /// Validate secret length.
    fn validate_secret(secret: &str) -> Result<(), AuthError> {
        if secret.len() < MIN_SECRET_LEN {
            return Err(AuthError::HmacKeyTooShort);
        }
        Ok(())
    }

    fn get_current_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn parse_token(&self, token: &str) -> Result<(String, u64, String, String, String), AuthError> {
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 5 {
            return Err(AuthError::TokenMalformed);
        }
        let id = parts[0].to_string();
        let ts = parts[1]
            .parse::<u64>()
            .map_err(|_e| AuthError::TokenMalformed)?;
        let nonce = parts[2].to_string();
        let claims_b64 = parts[3].to_string();
        let mac = parts[4].to_string();

        Ok((id, ts, nonce, claims_b64, mac))
    }

    fn validate_timestamp(&self, now: u64, ts: u64) -> Result<(), AuthError> {
        if ts > now {
            let diff = ts - now;
            if diff > self.inner.clock_skew.as_secs() {
                return Err(AuthError::TokenInvalid);
            }
        } else {
            let age = now - ts;
            if age > self.inner.validity_window.as_secs() {
                return Err(AuthError::TokenInvalid);
            }
        }
        Ok(())
    }

    fn record_replay(&self, nonce: &str, ts: u64, now: u64) -> Result<(), AuthError> {
        if !self.inner.replay_cache_enabled {
            // Replay protection disabled.
            return Ok(());
        }
        let entry = ReplayEntry {
            nonce: nonce.to_string(),
            timestamp: ts,
        };
        let mut cache = self.inner.replay_cache.lock();
        cache.insert(entry, now, self.inner.validity_window.as_secs())
    }
}

#[async_trait]
impl TokenProvider for SharedSecret {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // SharedSecret has no async initialization steps.
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let mut buf = String::new();
        self.get_token_into(&mut buf)?;
        Ok(buf)
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok(self.id().to_string())
    }

    fn get_signature_secret_key(&self) -> Result<Vec<u8>, AuthError> {
        Ok(self.signature_keys.0.clone())
    }

    fn get_signature_public_key(&self) -> Result<Vec<u8>, AuthError> {
        Ok(self.signature_keys.1.clone())
    }

    fn rotate_signature_keys(&mut self) -> Result<(), AuthError> {
        self.signature_keys = generate_mls_signature_keys()?;
        self.claims_b64 = Self::compute_claims_b64(&self.signature_keys.1);
        Ok(())
    }
}

#[async_trait]
impl Verifier for SharedSecret {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    async fn verify(&self, token: impl AsRef<str> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
        let token_str = token.as_ref();
        let now = self.get_current_timestamp();

        // The canonical signed message is exactly the token prefix up to the last ':'.
        // Splitting here avoids parsing into owned Strings and reconstructing the message.
        let last_colon = token_str.rfind(':').ok_or(AuthError::TokenMalformed)?;
        let message = &token_str[..last_colon];
        let mac_b64 = &token_str[last_colon + 1..];

        // Validate the 5-field shape and extract ts + nonce as &str slices.
        let mut it = message.splitn(4, ':');
        let _id = it.next().ok_or(AuthError::TokenMalformed)?;
        let ts_s = it.next().ok_or(AuthError::TokenMalformed)?;
        let nonce = it.next().ok_or(AuthError::TokenMalformed)?;
        let claims_b64 = it.next().ok_or(AuthError::TokenMalformed)?;
        if claims_b64.contains(':') {
            return Err(AuthError::TokenMalformed);
        }
        let ts: u64 = ts_s.parse().map_err(|_e| AuthError::TokenMalformed)?;

        self.validate_timestamp(now, ts)?;

        // Happy path: 32-byte HMAC-SHA256 tag is exactly 43 unpadded base64 chars.
        // Decode straight into a stack buffer — no heap allocation.
        let mac_bytes = mac_b64.as_bytes();
        let mut mac_buf = [0u8; 32];
        let expected: &[u8] = if mac_bytes.len() == 43 {
            let n = URL_SAFE_NO_PAD
                .decode_slice(mac_bytes, &mut mac_buf)
                .map_err(|e| match e {
                    base64::DecodeSliceError::DecodeError(de) => AuthError::Base64DecodeError(de),
                    base64::DecodeSliceError::OutputSliceTooSmall => AuthError::TokenMalformed,
                })?;
            if n != 32 {
                return Err(AuthError::TokenMalformed);
            }
            &mac_buf
        } else {
            // Wrong-length MAC: keep prior semantics (Base64DecodeError on bad b64).
            let v = URL_SAFE_NO_PAD.decode(mac_bytes)?;
            if v.len() != 32 {
                return Err(AuthError::TokenMalformed);
            }
            mac_buf.copy_from_slice(&v);
            &mac_buf
        };
        hmac::verify(&self.inner.hmac_key, message.as_bytes(), expected)
            .map_err(|_e| AuthError::TokenInvalid)?;

        self.record_replay(nonce, ts, now)
    }

    async fn get_claims<Claims>(&self, token: impl AsRef<str> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.try_get_claims(token)
    }

    fn try_get_claims<Claims>(&self, token: impl AsRef<str>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let token_str = token.as_ref();
        self.try_verify(token_str)?;
        let (token_id, ts, _, claims_b64, _) = self.parse_token(token_str)?;
        let exp = ts + self.inner.validity_window.as_secs();

        // Decode custom claims if present
        let custom_claims: serde_json::Value = if !claims_b64.is_empty() {
            let claims_json = URL_SAFE_NO_PAD.decode(claims_b64.as_bytes())?;
            serde_json::from_slice(&claims_json)?
        } else {
            serde_json::json!({})
        };

        // Build claims JSON with standard fields and custom_claims under its own key
        let claims_json = serde_json::json!({
            "sub": token_id,
            "iat": ts,
            "exp": exp,
            "custom_claims": custom_claims
        });

        let ret = serde_json::from_value(claims_json)?;

        Ok(ret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{TokenProvider, Verifier};
    use serde::Deserialize;
    use std::thread;
    use std::time::Duration;

    #[derive(Debug, Deserialize)]
    struct BasicClaims {
        sub: String,
        iat: u64,
        exp: u64,
    }

    fn valid_secret() -> String {
        "abcdefghijklmnopqrstuvwxyz012345".to_string()
    }

    /// Test-only helpers for forging tokens to exercise negative paths.
    fn gen_nonce() -> String {
        let mut bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut bytes);
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn build_message(id: &str, timestamp: u64, nonce: &str, claims_b64: &str) -> String {
        format!("{}:{}:{}:{}", id, timestamp, nonce, claims_b64)
    }

    fn forge_mac(s: &SharedSecret, message: &str) -> String {
        let tag = hmac::sign(&s.inner.hmac_key, message.as_bytes());
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    }

    #[test]
    fn test_secret_too_short() {
        let result = SharedSecret::new("svc", "shortsecret");
        assert!(result.is_err_and(|e| matches!(e, AuthError::HmacKeyTooShort)));
    }

    #[test]
    fn test_id_validation() {
        assert!(SharedSecret::new("good-id", &valid_secret()).is_ok());
        assert!(SharedSecret::new("bad:id", &valid_secret()).is_err());
        assert!(SharedSecret::new("bad id", &valid_secret()).is_err());
        assert!(SharedSecret::new("", &valid_secret()).is_err());
    }

    #[test]
    fn test_token_generation_format() {
        let s = SharedSecret::new("app", &valid_secret()).unwrap();
        let token = s.get_token().unwrap();
        let parts: Vec<_> = token.split(':').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[0].starts_with("app_"));
        assert!(parts[1].parse::<u64>().is_ok());
        assert!(!parts[2].is_empty());
        assert!(!parts[3].is_empty()); // claims field contains the embedded MLS public key
        assert!(URL_SAFE_NO_PAD.decode(parts[4]).is_ok());
    }

    #[test]
    fn test_verify_valid_token() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_cross_instance_verification() {
        let a = SharedSecret::new("svc", &valid_secret()).unwrap();
        let b = SharedSecret::new("svc", &valid_secret()).unwrap();
        let token = a.get_token().unwrap();
        assert!(b.try_verify(token).is_ok());
    }

    #[test]
    fn test_future_timestamp_exceeds_skew() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_clock_skew(Duration::from_secs(2));
        let future_ts = s.get_current_timestamp() + 10;
        let nonce = gen_nonce();
        let message = build_message(s.id(), future_ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_future_timestamp_within_skew() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_clock_skew(Duration::from_secs(10));
        let future_ts = s.get_current_timestamp() + 5;
        let nonce = gen_nonce();
        let message = build_message(s.id(), future_ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_expired_token() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_validity_window(Duration::from_secs(1));
        let past_ts = s.get_current_timestamp().saturating_sub(10);
        let nonce = gen_nonce();
        let message = build_message(s.id(), past_ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}:{}::{}", s.id(), past_ts, nonce, mac);
        let res = s.try_verify(token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenInvalid)));
    }

    #[test]
    fn test_replay_detection_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let replay = s.try_verify(token);
        assert!(replay.is_err_and(|e| matches!(e, AuthError::TokenInvalidReplay)));
    }

    #[test]
    fn test_replay_allowed_when_disabled() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap(); // default disabled
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        // Second verify should also succeed because replay protection off.
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_wrong_mac() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        let bad_mac = "!!notbase64";
        let token = format!("{}:{}:{}:{}", s.id(), ts, nonce, bad_mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_invalid_token_format_parts() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        assert!(s.try_verify("only:two:parts").is_err());
        assert!(s.try_verify("a:b:c:d:e").is_err());
    }

    #[test]
    fn test_invalid_timestamp_parse() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let nonce = gen_nonce();
        let mac = forge_mac(
            &s,
            &build_message(s.id(), s.get_current_timestamp(), &nonce, ""),
        );
        let token = format!("{}:{}:{}::{}", s.id(), "notanumber", nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_hmac_verification_failure() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        let message = build_message(s.id(), ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let truncated = &mac[..mac.len() / 2];
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, truncated);
        let res = s.try_verify(token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::Base64DecodeError(_))));
    }

    #[test]
    fn test_replay_after_expiration_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_validity_window(Duration::from_secs(1))
            .with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        thread::sleep(Duration::from_secs(2));
        let res = s.try_verify(token);
        // Expiration still trumps; token expired.
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenInvalid)));
    }

    #[test]
    fn test_nonce_uniqueness_and_length() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let mut nonces = std::collections::HashSet::new();
        for _ in 0..50 {
            let t = s.get_token().unwrap();
            let parts: Vec<_> = t.split(':').collect();
            assert_eq!(parts.len(), 5);
            let nonce = parts[2];
            assert!(nonce.len() >= NONCE_LEN);
            assert!(
                nonces.insert(nonce.to_string()),
                "nonce repeated unexpectedly"
            );
        }
    }

    #[test]
    fn test_mac_encoding_error() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        let bad_mac = "*invalid*mac*";
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, bad_mac);
        let res = s.try_verify(token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::Base64DecodeError(_))));
    }

    #[test]
    fn test_replay_detection_multiple_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(256);
        let t1 = s.get_token().unwrap();
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t1.clone()).is_ok());
        assert!(s.try_verify(t2.clone()).is_ok());
        assert!(s.try_verify(t1).is_err());
        assert!(s.try_verify(t2).is_err());
    }

    #[test]
    fn test_claims_disabled() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap(); // replay disabled
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_claims_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(64);
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_replay_cache_capacity_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(2);
        let t1 = s.get_token().unwrap();
        thread::sleep(Duration::from_millis(10));
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t1.clone()).is_ok());
        assert!(s.try_verify(t2.clone()).is_ok());
        thread::sleep(Duration::from_millis(10));
        let t3 = s.get_token().unwrap();
        assert!(s.try_verify(t3.clone()).is_ok());
        // t1 may have been evicted, so verifying again could succeed or fail based on eviction timing.
        let _ = s.try_verify(t1);
    }

    #[test]
    fn test_clone_preserves_replay_cache_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let cloned = s.clone();
        let res = cloned.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("replay"));
    }

    #[test]
    fn test_builder_preserves_replay_cache_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        // Adjust validity window; replay state preserved.
        let s2 = s.with_validity_window(Duration::from_secs(600));
        let res = s2.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("replay"));
    }

    #[test]
    fn test_disable_replay_cache_builder() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(64);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok()); // first verify ok
        // Disabling replay cache allows re-verification.
        let s_disabled = s.with_replay_cache_disabled();
        assert!(!s_disabled.replay_cache_enabled());
        assert!(s_disabled.try_verify(token).is_ok());
    }

    #[test]
    fn test_replay_cache_max_noop_when_disabled() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let original_max = s.replay_cache_max();
        let s2 = s.with_replay_cache_max(original_max * 2);
        assert_eq!(original_max, s2.replay_cache_max());
        assert!(!s2.replay_cache_enabled());
    }

    #[test]
    fn test_debug_shared_secret_internal() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        // Directly format the private inner struct to exercise Debug for SharedSecretInternal.
        let debug_str = format!("{:?}", *s.inner);
        assert!(debug_str.contains("SharedSecretInternal"));
        assert!(debug_str.contains("svc"));
    }

    #[test]
    fn test_with_replay_cache_max_when_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(128);
        let s2 = s.with_replay_cache_max(64);
        assert_eq!(64, s2.replay_cache_max());
        assert!(s2.replay_cache_enabled());
    }

    #[test]
    fn test_rebuild_evicts_when_shrinking() {
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_replay_cache_enabled(128);
        // Fill the replay cache with 3 entries.
        let t1 = s.get_token().unwrap();
        let t2 = s.get_token().unwrap();
        let t3 = s.get_token().unwrap();
        assert!(s.try_verify(t1).is_ok());
        assert!(s.try_verify(t2).is_ok());
        assert!(s.try_verify(t3).is_ok());
        // Shrink capacity to 1 — rebuild must evict the two oldest entries.
        let s_small = s.with_replay_cache_max(1);
        assert_eq!(1, s_small.replay_cache_max());
    }

    #[test]
    fn test_replay_cache_purges_expired_entries() {
        // Short validity window so entries expire quickly.
        let s = SharedSecret::new("svc", &valid_secret())
            .unwrap()
            .with_validity_window(Duration::from_secs(1))
            .with_replay_cache_enabled(128);
        let t1 = s.get_token().unwrap();
        assert!(s.try_verify(t1).is_ok());
        // Sleep until t1's timestamp is older than the validity window.
        thread::sleep(Duration::from_secs(2));
        // A fresh token triggers the expiry-cleanup loop inside ReplayCache::insert.
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t2).is_ok());
    }

    #[test]
    fn test_accessors() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        assert_eq!(s.base_id(), "svc");
        assert_eq!(
            s.validity_window(),
            Duration::from_secs(DEFAULT_VALIDITY_WINDOW)
        );
        assert_eq!(s.clock_skew(), Duration::from_secs(DEFAULT_CLOCK_SKEW));
        assert!(!s.shared_secret().is_empty());
    }

    #[tokio::test]
    async fn test_token_provider_trait_methods() {
        let mut s = SharedSecret::new("svc", &valid_secret()).unwrap();
        // TokenProvider::initialize (no-op, explicit disambiguation).
        <SharedSecret as TokenProvider>::initialize(&mut s)
            .await
            .unwrap();
        // get_id
        let id = s.get_id().unwrap();
        assert!(id.starts_with("svc_"));
        // get_signature_secret_key / get_signature_public_key
        let sk = s.get_signature_secret_key().unwrap();
        assert!(!sk.is_empty());
        let pk_before = s.get_signature_public_key().unwrap();
        assert!(!pk_before.is_empty());
        // rotate_signature_keys: public key must change after rotation.
        s.rotate_signature_keys().unwrap();
        let pk_after = s.get_signature_public_key().unwrap();
        assert_ne!(pk_before, pk_after);
        // Token issued after rotation must still verify with the same shared secret.
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token).is_ok());
    }

    #[tokio::test]
    async fn test_verifier_trait_methods() {
        let mut s = SharedSecret::new("svc", &valid_secret()).unwrap();
        // Verifier::initialize (no-op, explicit disambiguation).
        <SharedSecret as Verifier>::initialize(&mut s)
            .await
            .unwrap();
        // Verifier::verify (async wrapper around try_verify).
        let token = s.get_token().unwrap();
        s.verify(&token).await.unwrap();
    }

    #[tokio::test]
    async fn test_get_claims_async() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.get_claims(&token).await.unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_token_with_colon_in_claims_field() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        // claims_b64 field containing ':' must be rejected before HMAC verification.
        let token = format!("{}:{}:{}:claims:with:colon:bad_mac", s.id(), ts, nonce);
        let res = s.try_verify(&token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenMalformed)));
    }

    #[test]
    fn test_mac_43_chars_invalid_base64() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        // Exactly 43 chars (fast path) but '!' is not valid URL-safe base64.
        let invalid_43_mac = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA!";
        assert_eq!(invalid_43_mac.len(), 43);
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, invalid_43_mac);
        let res = s.try_verify(&token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::Base64DecodeError(_))));
    }

    #[test]
    fn test_mac_valid_base64_wrong_byte_length() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        // 33 bytes → 44 base64url chars (not 43), decodes to 33 bytes (not 32).
        let wrong_len_mac = URL_SAFE_NO_PAD.encode([0u8; 33]);
        assert_eq!(wrong_len_mac.len(), 44);
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, wrong_len_mac);
        let res = s.try_verify(&token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenMalformed)));
    }

    #[test]
    fn test_get_claims_empty_claims_field() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let ts = s.get_current_timestamp();
        let nonce = gen_nonce();
        // Forge a valid token with an empty claims_b64 field ("id:ts:nonce::mac").
        let message = build_message(s.id(), ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}", message, mac);
        // try_get_claims must fall through to the empty-claims branch (json!({})).
        let claims: BasicClaims = s.try_get_claims(&token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, ts + s.validity_window_secs());
    }

    #[test]
    fn test_get_token_into_buffer_reuse() {
        let s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let mut buf = String::with_capacity(4); // intentionally small initial capacity
        s.get_token_into(&mut buf).unwrap();
        assert!(!buf.is_empty());
        let first_token = buf.clone();
        // A second call must clear the buffer and write a fresh token.
        s.get_token_into(&mut buf).unwrap();
        assert!(!buf.is_empty());
        assert_ne!(first_token, buf); // different nonces
        assert!(s.try_verify(&first_token).is_ok());
        assert!(s.try_verify(&buf).is_ok());
    }
}

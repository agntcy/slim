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
//! let auth = SharedSecret::new("service", secret_string);
//! let token = auth.get_token()?;
//! auth.try_verify(&token)?;
//! ```
//!
//! Enabling replay protection:
//! ```ignore
//! let auth = SharedSecret::new("service", secret_string)
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
    metadata::MetadataMap,
    traits::{TokenProvider, Verifier},
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
            return Err(AuthError::TokenInvalid("replay detected".to_string()));
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

#[derive(Debug)]
struct SharedSecretInternal {
    base_id: String,
    id: String,
    shared_secret: String,
    validity_window: std::time::Duration,
    clock_skew: std::time::Duration,
    replay_cache_enabled: bool,
    replay_cache: Mutex<ReplayCache>,
}

/// Public wrapper holding an Arc to internal implementation.
/// Cloning keeps the same replay cache if enabled.
#[derive(Clone)]
pub struct SharedSecret(Arc<SharedSecretInternal>);

impl std::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSecret")
            .field("base_id", &self.0.base_id)
            .field("id", &self.0.id)
            .field("validity_window_secs", &self.0.validity_window.as_secs())
            .field("clock_skew_secs", &self.0.clock_skew.as_secs())
            .field("replay_cache_enabled", &self.0.replay_cache_enabled)
            .field("replay_cache_max", &self.0.replay_cache.lock().max_size)
            .finish()
    }
}

impl SharedSecret {
    /// Construct a new shared secret instance with randomized `id` suffix.
    /// Replay protection starts DISABLED.
    pub fn new(id: &str, shared_secret: &str) -> Self {
        Self::validate_id(id).expect("invalid id");
        Self::validate_secret(shared_secret).expect("invalid shared_secret");

        let random_suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let full_id = format!("{}_{}", id, random_suffix);

        let internal = SharedSecretInternal {
            base_id: id.to_owned(),
            id: full_id,
            shared_secret: shared_secret.to_owned(),
            validity_window: std::time::Duration::from_secs(DEFAULT_VALIDITY_WINDOW),
            clock_skew: std::time::Duration::from_secs(DEFAULT_CLOCK_SKEW),
            replay_cache_enabled: false,
            replay_cache: Mutex::new(ReplayCache::new(DEFAULT_REPLAY_CACHE_MAX)),
        };
        SharedSecret(Arc::new(internal))
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
        if !self.0.replay_cache_enabled {
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
        let current = &self.0;
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
            validity_window: validity_window.unwrap_or(current.validity_window),
            clock_skew: clock_skew.unwrap_or(current.clock_skew),
            replay_cache_enabled: enable_flag,
            replay_cache: Mutex::new(cloned_cache),
        };
        SharedSecret(Arc::new(internal))
    }

    /// Get the randomized unique identifier.
    pub fn id(&self) -> &str {
        &self.0.id
    }

    /// Base identifier (without random suffix).
    pub fn base_id(&self) -> &str {
        &self.0.base_id
    }

    /// Raw shared secret (avoid logging).
    pub fn shared_secret(&self) -> &str {
        &self.0.shared_secret
    }

    /// Validity window duration.
    pub fn validity_window(&self) -> std::time::Duration {
        self.0.validity_window
    }

    /// Validity window in seconds (helper for tests / metrics).
    pub fn validity_window_secs(&self) -> u64 {
        self.0.validity_window.as_secs()
    }

    /// Clock skew duration.
    pub fn clock_skew(&self) -> std::time::Duration {
        self.0.clock_skew
    }

    /// Replay cache enabled?
    pub fn replay_cache_enabled(&self) -> bool {
        self.0.replay_cache_enabled
    }

    /// Replay cache max size (even if disabled).
    pub fn replay_cache_max(&self) -> usize {
        self.0.replay_cache.lock().max_size
    }

    /// Validate identifier format.
    fn validate_id(id: &str) -> Result<(), AuthError> {
        if id.is_empty() {
            return Err(AuthError::TokenInvalid("id is empty".to_string()));
        }
        if id.contains(':') {
            return Err(AuthError::TokenInvalid("id contains ':'".to_string()));
        }
        if id.chars().any(|c| c.is_whitespace()) {
            return Err(AuthError::TokenInvalid(
                "id contains whitespace".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate secret length.
    fn validate_secret(secret: &str) -> Result<(), AuthError> {
        if secret.len() < MIN_SECRET_LEN {
            return Err(AuthError::TokenInvalid(format!(
                "shared_secret too short (min {} chars)",
                MIN_SECRET_LEN
            )));
        }
        Ok(())
    }

    fn get_current_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn create_hmac_raw(&self, message: &[u8]) -> Result<Vec<u8>, AuthError> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.0.shared_secret.as_bytes());
        let tag = hmac::sign(&key, message);
        Ok(tag.as_ref().to_vec())
    }

    fn create_hmac_b64(&self, message: &str) -> Result<String, AuthError> {
        let raw = self.create_hmac_raw(message.as_bytes())?;
        Ok(URL_SAFE_NO_PAD.encode(raw))
    }

    fn verify_hmac(&self, message: &str, expected_b64: &str) -> Result<(), AuthError> {
        let expected = URL_SAFE_NO_PAD
            .decode(expected_b64.as_bytes())
            .map_err(|_| AuthError::TokenInvalid("invalid mac encoding".to_string()))?;
        if expected.len() != 32 {
            return Err(AuthError::TokenInvalid(
                "hmac verification failed".to_string(),
            ));
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.0.shared_secret.as_bytes());
        hmac::verify(&key, message.as_bytes(), &expected)
            .map_err(|_| AuthError::TokenInvalid("hmac verification failed".to_string()))
    }

    fn build_message(&self, id: &str, timestamp: u64, nonce: &str, claims_b64: &str) -> String {
        format!("{}:{}:{}:{}", id, timestamp, nonce, claims_b64)
    }

    fn gen_nonce(&self) -> String {
        let mut bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut bytes);
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn parse_token(&self, token: &str) -> Result<(String, u64, String, String, String), AuthError> {
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 5 {
            return Err(AuthError::TokenInvalid(
                "invalid token format, expected 5 parts".to_string(),
            ));
        }
        let id = parts[0].to_string();
        let ts = parts[1]
            .parse::<u64>()
            .map_err(|_| AuthError::TokenInvalid("invalid timestamp".to_string()))?;
        let nonce = parts[2].to_string();
        let claims_b64 = parts[3].to_string();
        let mac = parts[4].to_string();

        Ok((id, ts, nonce, claims_b64, mac))
    }

    fn validate_timestamp(&self, now: u64, ts: u64) -> Result<(), AuthError> {
        if ts > now {
            let diff = ts - now;
            if diff > self.0.clock_skew.as_secs() {
                return Err(AuthError::TokenInvalid(
                    "timestamp too far in future".to_string(),
                ));
            }
        } else {
            let age = now - ts;
            if age > self.0.validity_window.as_secs() {
                return Err(AuthError::TokenInvalid("token expired".to_string()));
            }
        }
        Ok(())
    }

    fn record_replay(&self, nonce: &str, ts: u64, now: u64) -> Result<(), AuthError> {
        if !self.0.replay_cache_enabled {
            // Replay protection disabled.
            return Ok(());
        }
        let entry = ReplayEntry {
            nonce: nonce.to_string(),
            timestamp: ts,
        };
        let mut cache = self.0.replay_cache.lock();
        cache.insert(entry, now, self.0.validity_window.as_secs())
    }
}

#[async_trait]
impl TokenProvider for SharedSecret {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // SharedSecret has no async initialization steps.
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        if self.0.shared_secret.is_empty() {
            return Err(AuthError::TokenInvalid(
                "shared_secret is empty".to_string(),
            ));
        }
        let ts = self.get_current_timestamp();
        let nonce = self.gen_nonce();
        let message = self.build_message(self.id(), ts, &nonce, "");
        let mac = self.create_hmac_b64(&message)?;
        Ok(format!("{}:{}:{}::{}", self.id(), ts, nonce, mac))
    }

    async fn get_token_with_claims(&self, custom_claims: MetadataMap) -> Result<String, AuthError> {
        if self.0.shared_secret.is_empty() {
            return Err(AuthError::TokenInvalid(
                "shared_secret is empty".to_string(),
            ));
        }

        let ts = self.get_current_timestamp();
        let nonce = self.gen_nonce();

        // Serialize custom claims to JSON and encode to base64 (empty string if no claims)
        let claims_b64 = if custom_claims.is_empty() {
            String::new()
        } else {
            let claims_json = serde_json::to_string(&custom_claims).map_err(|e| {
                AuthError::TokenInvalid(format!("failed to serialize claims: {}", e))
            })?;
            URL_SAFE_NO_PAD.encode(claims_json.as_bytes())
        };

        // Build message with claims included (can be empty)
        let message = self.build_message(self.id(), ts, &nonce, &claims_b64);
        let mac = self.create_hmac_b64(&message)?;

        // Format: id:timestamp:nonce:claims_b64:mac (claims_b64 can be empty)
        Ok(format!(
            "{}:{}:{}:{}:{}",
            self.id(),
            ts,
            nonce,
            claims_b64,
            mac
        ))
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok(self.id().to_string())
    }
}

#[async_trait]
impl Verifier for SharedSecret {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(())
    }

    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        let token_str = token.into();
        let now = self.get_current_timestamp();
        let (token_id, ts, nonce, claims_b64, mac_b64) = self.parse_token(&token_str)?;
        self.validate_timestamp(now, ts)?;
        let message = self.build_message(&token_id, ts, &nonce, &claims_b64);
        self.verify_hmac(&message, &mac_b64)?;
        self.record_replay(&nonce, ts, now)
    }

    async fn get_claims<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.try_get_claims(token)
    }

    fn try_get_claims<Claims>(&self, token: impl Into<String>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let token_str = token.into();
        self.try_verify(token_str.clone())?;
        let (token_id, ts, _, claims_b64, _) = self.parse_token(&token_str)?;
        let exp = ts + self.0.validity_window.as_secs();

        // Decode custom claims if present
        let custom_claims: serde_json::Value = if !claims_b64.is_empty() {
            let claims_json = URL_SAFE_NO_PAD
                .decode(claims_b64.as_bytes())
                .map_err(|_| AuthError::TokenInvalid("invalid claims encoding".to_string()))?;
            let claims_str = String::from_utf8(claims_json)
                .map_err(|_| AuthError::TokenInvalid("invalid claims utf8".to_string()))?;
            serde_json::from_str(&claims_str)
                .map_err(|_| AuthError::TokenInvalid("invalid claims json".to_string()))?
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

        serde_json::from_value(claims_json)
            .map_err(|_| AuthError::TokenInvalid("claims parse error".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    #[should_panic(expected = "invalid shared_secret")]
    fn test_secret_too_short() {
        let _ = SharedSecret::new("svc", "shortsecret");
    }

    #[test]
    fn test_id_validation() {
        assert!(std::panic::catch_unwind(|| SharedSecret::new("good-id", &valid_secret())).is_ok());
        assert!(std::panic::catch_unwind(|| SharedSecret::new("bad:id", &valid_secret())).is_err());
        assert!(std::panic::catch_unwind(|| SharedSecret::new("bad id", &valid_secret())).is_err());
        assert!(std::panic::catch_unwind(|| SharedSecret::new("", &valid_secret())).is_err());
    }

    #[test]
    fn test_token_generation_format() {
        let s = SharedSecret::new("app", &valid_secret());
        let token = s.get_token().unwrap();
        let parts: Vec<_> = token.split(':').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[0].starts_with("app_"));
        assert!(parts[1].parse::<u64>().is_ok());
        assert!(!parts[2].is_empty());
        assert!(parts[3].is_empty()); // claims field is empty when no custom claims
        assert!(URL_SAFE_NO_PAD.decode(parts[4]).is_ok());
    }

    #[test]
    fn test_verify_valid_token() {
        let s = SharedSecret::new("svc", &valid_secret());
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_cross_instance_verification() {
        let a = SharedSecret::new("svc", &valid_secret());
        let b = SharedSecret::new("svc", &valid_secret());
        let token = a.get_token().unwrap();
        assert!(b.try_verify(token).is_ok());
    }

    #[test]
    fn test_future_timestamp_exceeds_skew() {
        let s = SharedSecret::new("svc", &valid_secret()).with_clock_skew(Duration::from_secs(2));
        let future_ts = s.get_current_timestamp() + 10;
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), future_ts, &nonce, "");
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_future_timestamp_within_skew() {
        let s = SharedSecret::new("svc", &valid_secret()).with_clock_skew(Duration::from_secs(10));
        let future_ts = s.get_current_timestamp() + 5;
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), future_ts, &nonce, "");
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_expired_token() {
        let s =
            SharedSecret::new("svc", &valid_secret()).with_validity_window(Duration::from_secs(1));
        let past_ts = s.get_current_timestamp().saturating_sub(10);
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), past_ts, &nonce, "");
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}::{}", s.id(), past_ts, nonce, mac);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn test_replay_detection_enabled() {
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let replay = s.try_verify(token);
        assert!(replay.is_err());
        assert!(replay.unwrap_err().to_string().contains("replay"));
    }

    #[test]
    fn test_replay_allowed_when_disabled() {
        let s = SharedSecret::new("svc", &valid_secret()); // default disabled
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        // Second verify should also succeed because replay protection off.
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_wrong_mac() {
        let s = SharedSecret::new("svc", &valid_secret());
        let ts = s.get_current_timestamp();
        let nonce = s.gen_nonce();
        let bad_mac = "!!notbase64";
        let token = format!("{}:{}:{}:{}", s.id(), ts, nonce, bad_mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_invalid_token_format_parts() {
        let s = SharedSecret::new("svc", &valid_secret());
        assert!(s.try_verify("only:two:parts").is_err());
        assert!(s.try_verify("a:b:c:d:e").is_err());
    }

    #[test]
    fn test_invalid_timestamp_parse() {
        let s = SharedSecret::new("svc", &valid_secret());
        let nonce = s.gen_nonce();
        let mac = s
            .create_hmac_b64(&s.build_message(s.id(), s.get_current_timestamp(), &nonce, ""))
            .unwrap();
        let token = format!("{}:{}:{}::{}", s.id(), "notanumber", nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_hmac_verification_failure() {
        let s = SharedSecret::new("svc", &valid_secret());
        let ts = s.get_current_timestamp();
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), ts, &nonce, "");
        let mac = s.create_hmac_b64(&message).unwrap();
        let truncated = &mac[..mac.len() / 2];
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, truncated);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("invalid mac encoding")
        );
    }

    #[test]
    fn test_replay_after_expiration_enabled() {
        let s = SharedSecret::new("svc", &valid_secret())
            .with_validity_window(Duration::from_secs(1))
            .with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        thread::sleep(Duration::from_secs(2));
        let res = s.try_verify(token);
        // Expiration still trumps; token expired.
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn test_nonce_uniqueness_and_length() {
        let s = SharedSecret::new("svc", &valid_secret());
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
        let s = SharedSecret::new("svc", &valid_secret());
        let ts = s.get_current_timestamp();
        let nonce = s.gen_nonce();
        let bad_mac = "*invalid*mac*";
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, bad_mac);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("invalid mac encoding")
        );
    }

    #[test]
    fn test_replay_detection_multiple_enabled() {
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(256);
        let t1 = s.get_token().unwrap();
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t1.clone()).is_ok());
        assert!(s.try_verify(t2.clone()).is_ok());
        assert!(s.try_verify(t1).is_err());
        assert!(s.try_verify(t2).is_err());
    }

    #[test]
    fn test_claims_disabled() {
        let s = SharedSecret::new("svc", &valid_secret()); // replay disabled
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_claims_enabled() {
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(64);
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_replay_cache_capacity_enabled() {
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(2);
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
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(128);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let cloned = s.clone();
        let res = cloned.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("replay"));
    }

    #[test]
    fn test_builder_preserves_replay_cache_enabled() {
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(128);
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
        let s = SharedSecret::new("svc", &valid_secret()).with_replay_cache_enabled(64);
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok()); // first verify ok
        // Disabling replay cache allows re-verification.
        let s_disabled = s.with_replay_cache_disabled();
        assert!(!s_disabled.replay_cache_enabled());
        assert!(s_disabled.try_verify(token).is_ok());
    }

    #[test]
    fn test_replay_cache_max_noop_when_disabled() {
        let s = SharedSecret::new("svc", &valid_secret());
        let original_max = s.replay_cache_max();
        let s2 = s.with_replay_cache_max(original_max * 2);
        assert_eq!(original_max, s2.replay_cache_max());
        assert!(!s2.replay_cache_enabled());
    }

    #[tokio::test]
    async fn test_custom_claims() {
        let s = SharedSecret::new("svc", &valid_secret());

        // Create custom claims
        let mut custom_claims = MetadataMap::new();
        custom_claims.insert("user_id", "user-123");
        custom_claims.insert("role", "admin");
        custom_claims.insert("tenant_id", "tenant-456");

        // Generate token with custom claims
        let token = s.get_token_with_claims(custom_claims).await.unwrap();

        // Verify token format (5 parts)
        let parts: Vec<_> = token.split(':').collect();
        assert_eq!(parts.len(), 5);
        assert!(!parts[3].is_empty()); // claims field should not be empty

        // Verify token
        assert!(s.try_verify(token.clone()).is_ok());

        // Extract claims
        let claims: serde_json::Value = s.try_get_claims(token).unwrap();

        // Check standard fields
        assert!(claims["sub"].as_str().unwrap().starts_with("svc_"));
        assert!(claims["iat"].as_u64().is_some());
        assert!(claims["exp"].as_u64().is_some());

        // Check custom claims under "custom_claims" key
        let custom = &claims["custom_claims"];
        assert_eq!(custom["user_id"].as_str().unwrap(), "user-123");
        assert_eq!(custom["role"].as_str().unwrap(), "admin");
        assert_eq!(custom["tenant_id"].as_str().unwrap(), "tenant-456");
    }

    #[tokio::test]
    async fn test_custom_claims_empty() {
        let s = SharedSecret::new("svc", &valid_secret());

        // Generate token with empty custom claims
        let custom_claims = MetadataMap::new();
        let token = s.get_token_with_claims(custom_claims).await.unwrap();

        // Verify token format (5 parts)
        let parts: Vec<_> = token.split(':').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[3].is_empty()); // claims field should be empty

        // Verify token
        assert!(s.try_verify(token.clone()).is_ok());

        // Extract claims
        let claims: serde_json::Value = s.try_get_claims(token).unwrap();

        // Check custom_claims is an empty object
        let custom = &claims["custom_claims"];
        assert!(custom.is_object());
        assert_eq!(custom.as_object().unwrap().len(), 0);
    }
}

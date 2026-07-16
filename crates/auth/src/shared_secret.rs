/*
Copyright AGNTCY Contributors (https://github.com/agntcy)
SPDX-License-Identifier: Apache-2.0
*/

//! Shared-secret based token generation and verification.
//!
//! This implementation supports an optionally enabled replay cache.
//! IMPORTANT: Replay prevention is DISABLED by default. Enable it at
//! construction with `SharedSecret::builder(id, secret).replay_cache(max)` if
//! you require replay detection.
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
//! * `SharedSecret` is cheap to clone (Arc increment). Cloning shares the replay
//!   cache and one MLS signing identity, so an app and every session cloned from
//!   it present the same identity.
//! * Configuration is applied up front via [`SharedSecretBuilder`]; there is no
//!   post-construction reconfiguration.
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
//! let auth = SharedSecret::builder("service", secret_string)
//!     .replay_cache(4096)
//!     .build()?;
//! let token = auth.get_token()?;
//! auth.try_verify(&token)?; // second verify of same token will fail
//! ```
//!
//! Thread safety:
//! * Interior mutability only for the replay cache (parking_lot::Mutex).
//! * All other fields are immutable after construction.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as STANDARD_BASE64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use parking_lot::{Mutex, RwLock};
use rand::{Rng, distr::Alphanumeric};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

use crate::mac::HmacKey;
use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

/// Minimum length (in bytes) required for the shared secret (baseline 256 bits).
const MIN_SECRET_LEN: usize = 32;
/// Raw nonce byte length before base64url encoding.
const NONCE_LEN: usize = 12;
/// Expected base64url-no-pad encoded length of the nonce.
const NONCE_B64_LEN: usize = base64url_nopad_encoded_len(NONCE_LEN);
/// HMAC-SHA256 tag length in bytes.
const HMAC_TAG_LEN: usize = 32;
/// Expected base64url-no-pad encoded length of the HMAC tag.
const HMAC_TAG_B64_LEN: usize = base64url_nopad_encoded_len(HMAC_TAG_LEN);
/// Maximum digits in a u64 timestamp.
const MAX_TIMESTAMP_DIGITS: usize = 20;
/// Number of colon separators in the token format.
const TOKEN_SEPARATOR_COUNT: usize = 4;
/// Default validity window (seconds).
const DEFAULT_VALIDITY_WINDOW: u64 = 3600;
/// Default tolerated forward clock skew (seconds).
const DEFAULT_CLOCK_SKEW: u64 = 5;
/// Default maximum replay cache entries (used if enabled without override).
const DEFAULT_REPLAY_CACHE_MAX: usize = 4096;

/// Computes the encoded length of base64url-no-pad for a given byte length.
const fn base64url_nopad_encoded_len(byte_len: usize) -> usize {
    (byte_len * 4).div_ceil(3)
}

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
    hmac_key: HmacKey,
    validity_window: std::time::Duration,
    clock_skew: std::time::Duration,
    replay_cache_enabled: bool,
    replay_cache: Mutex<ReplayCache>,
    /// MLS signing material (see [`SigningMaterial`]). Lives in the shared
    /// `inner`, so an app and every session cloned from it present one identity.
    signing: RwLock<SigningMaterial>,
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

/// MLS signing material, stored in the shared [`SharedSecretInternal`].
///
/// An app and all the sessions cloned from it therefore present a single
/// identity: one Ed25519 signature key pair and the matching claims. Held
/// behind a `RwLock` (inside the shared `Arc`) so a key rotation or a restore
/// is visible to every clone at once.
struct SigningMaterial {
    /// MLS signature key pair: (secret_key_bytes, public_key_bytes).
    keys: (Vec<u8>, Vec<u8>),
    /// Precomputed URL_SAFE_NO_PAD(base64) of the claims JSON embedding the MLS
    /// public key. Only changes when the signature keys are rotated.
    claims_b64: String,
    /// True once the MLS layer has installed ciphersuite-correct keys via
    /// `set_signature_keys`. The keys generated at construction are placeholder
    /// Ed25519 keys (used for header signing) and are not necessarily valid for
    /// the MLS ciphersuite in use, so the MLS layer must not reuse them until
    /// this flips true.
    mls_installed: bool,
}

/// Public wrapper holding an Arc to internal implementation.
/// Cloning shares the replay cache, config, and MLS signing material, so all
/// clones present the same identity.
#[derive(Clone)]
pub struct SharedSecret {
    inner: Arc<SharedSecretInternal>,
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

/// Builder for [`SharedSecret`]. Configure the validity window, clock skew, and
/// (optionally) the replay cache, then call [`SharedSecretBuilder::build`].
///
/// Replaces the previous post-construction `with_*` transformers: configuration
/// is applied once, up front, so there is no need to rebuild an existing
/// instance (which would otherwise have to carefully carry over the shared MLS
/// identity and live replay-cache state).
pub struct SharedSecretBuilder {
    base_id: String,
    shared_secret: String,
    validity_window: std::time::Duration,
    clock_skew: std::time::Duration,
    /// `Some(max)` enables the replay cache with that capacity; `None` disables it.
    replay_cache: Option<usize>,
}

impl SharedSecretBuilder {
    fn new(id: &str, shared_secret: &str) -> Self {
        Self {
            base_id: id.to_owned(),
            shared_secret: shared_secret.to_owned(),
            validity_window: std::time::Duration::from_secs(DEFAULT_VALIDITY_WINDOW),
            clock_skew: std::time::Duration::from_secs(DEFAULT_CLOCK_SKEW),
            replay_cache: None,
        }
    }

    /// Token validity window.
    pub fn validity_window(mut self, window: std::time::Duration) -> Self {
        self.validity_window = window;
        self
    }

    /// Tolerated forward clock skew.
    pub fn clock_skew(mut self, skew: std::time::Duration) -> Self {
        self.clock_skew = skew;
        self
    }

    /// Enable replay protection with the given maximum cache capacity.
    pub fn replay_cache(mut self, max_size: usize) -> Self {
        self.replay_cache = Some(max_size);
        self
    }

    /// Validate the inputs and construct the [`SharedSecret`], generating a
    /// random `id` suffix and a fresh placeholder MLS signature key pair.
    pub fn build(self) -> Result<SharedSecret, AuthError> {
        SharedSecret::validate_id(&self.base_id)?;
        SharedSecret::validate_secret(&self.shared_secret)?;

        let random_suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let full_id = format!("{}_{}", self.base_id, random_suffix);

        let (secret_key, public_key) = crate::utils::generate_mls_signature_keys()?;
        let claims_b64 = SharedSecret::compute_claims_b64(&public_key);
        let hmac_key = HmacKey::new(self.shared_secret.as_bytes());

        let replay_cache_enabled = self.replay_cache.is_some();
        let replay_cache_max = self.replay_cache.unwrap_or(DEFAULT_REPLAY_CACHE_MAX);

        let internal = SharedSecretInternal {
            base_id: self.base_id,
            id: full_id,
            shared_secret: self.shared_secret,
            hmac_key,
            validity_window: self.validity_window,
            clock_skew: self.clock_skew,
            replay_cache_enabled,
            replay_cache: Mutex::new(ReplayCache::new(replay_cache_max)),
            signing: RwLock::new(SigningMaterial {
                keys: (secret_key, public_key),
                claims_b64,
                // The keys above are generated for the exact ciphersuite MLS
                // uses on this target (see `utils::generate_mls_signature_keys`),
                // so MLS adopts them directly and never has to rotate the signing
                // identity mid-handshake — which is what previously raced with
                // concurrent control-message signing.
                mls_installed: true,
            }),
        };
        Ok(SharedSecret {
            inner: Arc::new(internal),
        })
    }
}

impl SharedSecret {
    /// Build the URL_SAFE_NO_PAD(base64) of the claims JSON embedding the MLS pubkey.
    fn compute_claims_b64(pub_key: &[u8]) -> String {
        let pub_key_b64 = STANDARD_BASE64.encode(pub_key);
        let claims_json = serde_json::json!({ "pubkey": pub_key_b64 }).to_string();
        URL_SAFE_NO_PAD.encode(claims_json.as_bytes())
    }

    /// Start building a shared-secret provider. Replay protection is DISABLED
    /// unless [`SharedSecretBuilder::replay_cache`] is called.
    pub fn builder(id: &str, shared_secret: &str) -> SharedSecretBuilder {
        SharedSecretBuilder::new(id, shared_secret)
    }

    /// Construct a shared-secret provider with default settings and a randomized
    /// `id` suffix. Shorthand for `SharedSecret::builder(id, secret).build()`.
    pub fn new(id: &str, shared_secret: &str) -> Result<Self, AuthError> {
        Self::builder(id, shared_secret).build()
    }

    /// Returns the expected token byte length for this instance, suitable for
    /// preallocating a `String` buffer. `claims_len` is the byte length of the
    /// (already locked) claims field, passed in to avoid re-locking.
    fn token_capacity(&self, claims_len: usize) -> usize {
        self.inner.id.len()
            + TOKEN_SEPARATOR_COUNT
            + MAX_TIMESTAMP_DIGITS
            + NONCE_B64_LEN
            + claims_len
            + HMAC_TAG_B64_LEN
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
        let signing = self.inner.signing.read();
        let claims_b64 = signing.claims_b64.as_str();

        let cap = self.token_capacity(claims_b64.len());
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
        let tag = self.inner.hmac_key.sign(out.as_bytes());
        out.push(':');
        URL_SAFE_NO_PAD.encode_string(tag, out);

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
        web_time::SystemTime::now()
            .duration_since(web_time::SystemTime::UNIX_EPOCH)
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

impl TokenProvider for SharedSecret {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // SharedSecret has no async initialization steps.
        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let cap = self.token_capacity(self.inner.signing.read().claims_b64.len());
        let mut buf = String::with_capacity(cap);
        self.get_token_into(&mut buf)?;
        Ok(buf)
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok(self.id().to_string())
    }

    fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        // Read both keys under a single lock so a concurrent `set_signature_keys`
        // rotation can never hand back a mismatched (secret, public) pair.
        let signing = self.inner.signing.read();
        Ok((signing.keys.0.clone(), signing.keys.1.clone()))
    }

    fn mls_signature_keys_installed(&self) -> bool {
        self.inner.signing.read().mls_installed
    }

    async fn set_signature_keys(
        &mut self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        // Shared across clones: a rotation is visible to the app and every
        // session cloned from it.
        let mut signing = self.inner.signing.write();
        signing.claims_b64 = Self::compute_claims_b64(&public_key);
        signing.keys = (private_key, public_key);
        signing.mls_installed = true;
        Ok(())
    }
}

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

        // HMAC-SHA256 tag is always HMAC_TAG_LEN bytes, which encodes to exactly
        // HMAC_TAG_B64_LEN base64url-no-pad chars. Reject anything else immediately.
        let mac_bytes = mac_b64.as_bytes();
        if mac_bytes.len() != HMAC_TAG_B64_LEN {
            return Err(AuthError::TokenMalformed);
        }
        let mut mac_buf = [0u8; HMAC_TAG_LEN];
        URL_SAFE_NO_PAD
            .decode_slice(mac_bytes, &mut mac_buf)
            .map_err(|e| match e {
                base64::DecodeSliceError::DecodeError(de) => AuthError::Base64DecodeError(de),
                base64::DecodeSliceError::OutputSliceTooSmall => AuthError::TokenMalformed,
            })?;
        let expected: &[u8] = &mac_buf;
        if !self.inner.hmac_key.verify(message.as_bytes(), expected) {
            return Err(AuthError::TokenInvalid);
        }

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
        let tag = s.inner.hmac_key.sign(message.as_bytes());
        URL_SAFE_NO_PAD.encode(tag)
    }

    #[test]
    fn test_secret_too_short() {
        let result = SharedSecret::new("svc", "shortsecret");
        assert!(result.is_err_and(|e| matches!(e, AuthError::HmacKeyTooShort)));
    }

    #[tokio::test]
    async fn test_set_signature_keys_updates_keys_and_claims() {
        let mut s = SharedSecret::new("svc", &valid_secret()).unwrap();

        let initial_pub = s.get_signature_keys().unwrap().1;
        let initial_token = s.get_token().unwrap();

        let new_priv = vec![7u8; 32];
        let new_pub = vec![9u8; 32];
        s.set_signature_keys(new_priv.clone(), new_pub.clone())
            .await
            .unwrap();

        // The provider now reports the externally-supplied key pair.
        assert_eq!(
            s.get_signature_keys().unwrap(),
            (new_priv.clone(), new_pub.clone())
        );
        assert_ne!(s.get_signature_keys().unwrap().1, initial_pub);

        // The embedded claims carry the public key, so a token minted after the
        // swap differs from one minted before it.
        let new_token = s.get_token().unwrap();
        let claims_before = initial_token.split(':').nth(3).unwrap();
        let claims_after = new_token.split(':').nth(3).unwrap();
        assert_ne!(claims_before, claims_after);

        // The token must still self-verify after the key swap.
        assert!(s.try_verify(new_token).is_ok());
    }

    #[tokio::test]
    async fn test_set_signature_keys_changes_keys() {
        let mut s = SharedSecret::new("svc", &valid_secret()).unwrap();
        let (before_priv, before_pub) = s.get_signature_keys().unwrap();

        let new_priv = vec![42u8; 32];
        let new_pub = vec![43u8; 32];
        s.set_signature_keys(new_priv.clone(), new_pub.clone())
            .await
            .unwrap();

        let (after_priv, after_pub) = s.get_signature_keys().unwrap();
        assert_ne!(before_pub, after_pub);
        assert_ne!(before_priv, after_priv);
        assert_eq!(after_pub, new_pub);
        assert_eq!(after_priv, new_priv);

        // The updated identity still produces a verifiable token.
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token).is_ok());
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
        let s = SharedSecret::builder("svc", &valid_secret())
            .clock_skew(Duration::from_secs(2))
            .build()
            .unwrap();
        let future_ts = s.get_current_timestamp() + 10;
        let nonce = gen_nonce();
        let message = build_message(s.id(), future_ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_future_timestamp_within_skew() {
        let s = SharedSecret::builder("svc", &valid_secret())
            .clock_skew(Duration::from_secs(10))
            .build()
            .unwrap();
        let future_ts = s.get_current_timestamp() + 5;
        let nonce = gen_nonce();
        let message = build_message(s.id(), future_ts, &nonce, "");
        let mac = forge_mac(&s, &message);
        let token = format!("{}:{}:{}::{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_expired_token() {
        let s = SharedSecret::builder("svc", &valid_secret())
            .validity_window(Duration::from_secs(1))
            .build()
            .unwrap();
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
        let s = SharedSecret::builder("svc", &valid_secret())
            .replay_cache(128)
            .build()
            .unwrap();
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
        // Truncated MAC has wrong length → rejected as malformed before decoding.
        let truncated = &mac[..mac.len() / 2];
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, truncated);
        let res = s.try_verify(token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenMalformed)));
    }

    #[test]
    fn test_replay_after_expiration_enabled() {
        let s = SharedSecret::builder("svc", &valid_secret())
            .validity_window(Duration::from_secs(1))
            .replay_cache(128)
            .build()
            .unwrap();
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
        // Wrong-length MAC is rejected as malformed before base64 decoding.
        let bad_mac = "*invalid*mac*";
        let token = format!("{}:{}:{}::{}", s.id(), ts, nonce, bad_mac);
        let res = s.try_verify(token);
        assert!(res.is_err_and(|e| matches!(e, AuthError::TokenMalformed)));
    }

    #[test]
    fn test_replay_detection_multiple_enabled() {
        let s = SharedSecret::builder("svc", &valid_secret())
            .replay_cache(256)
            .build()
            .unwrap();
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
        let s = SharedSecret::builder("svc", &valid_secret())
            .replay_cache(64)
            .build()
            .unwrap();
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.sub.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window_secs());
    }

    #[test]
    fn test_replay_cache_capacity_enabled() {
        let s = SharedSecret::builder("svc", &valid_secret())
            .replay_cache(2)
            .build()
            .unwrap();
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
        let s = SharedSecret::builder("svc", &valid_secret())
            .replay_cache(128)
            .build()
            .unwrap();
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let cloned = s.clone();
        let res = cloned.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("replay"));
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
    fn test_replay_cache_purges_expired_entries() {
        // Short validity window so entries expire quickly.
        let s = SharedSecret::builder("svc", &valid_secret())
            .validity_window(Duration::from_secs(1))
            .replay_cache(128)
            .build()
            .unwrap();
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
        // get_signature_keys
        let (sk, pk_before) = s.get_signature_keys().unwrap();
        // set_signature_keys: public key must change after setting new keys.
        let new_priv = vec![11u8; 32];
        let new_pub = vec![22u8; 32];
        s.set_signature_keys(new_priv.clone(), new_pub.clone())
            .await
            .unwrap();
        let (sk_after, pk_after) = s.get_signature_keys().unwrap();
        assert_ne!(pk_before, pk_after);
        assert_ne!(sk, sk_after);
        // Token issued after key update must still verify with the same shared secret.
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

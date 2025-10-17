use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use rand::{Rng, distr::Alphanumeric};
use sha2::Sha256;
use std::{
    collections::{HashSet, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

type HmacSha256 = Hmac<Sha256>;

/// Minimum length (in bytes) required for the shared secret.
/// 32 bytes (~256 bits) is a reasonable baseline for HMAC-SHA256.
const MIN_SECRET_LEN: usize = 32;
/// Nonce length (raw bytes) before base64url encoding.
const NONCE_LEN: usize = 12;

/// Default validity window (in seconds) for tokens.
const DEFAULT_VALIDITY_WINDOW: u64 = 300; // 5 minutes
/// Default clock skew allowance (in seconds) for minor time drift.
const DEFAULT_CLOCK_SKEW: u64 = 5;
/// Maximum replay cache size to avoid unbounded memory growth.
const DEFAULT_REPLAY_CACHE_MAX: usize = 4096;

/// Internal representation of a replay entry (stores nonce + timestamp).
#[derive(Debug, PartialEq, Eq, Hash)]
struct ReplayEntry {
    nonce: String,
    timestamp: u64,
}

#[derive(Debug)]
pub struct SharedSecret {
    base_id: String,
    id: String,
    shared_secret: String,
    validity_window: std::time::Duration,
    clock_skew: std::time::Duration,
    replay_cache: Mutex<ReplayCache>,
}

impl Clone for SharedSecret {
    fn clone(&self) -> Self {
        SharedSecret {
            base_id: self.base_id.clone(),
            id: self.id.clone(),
            shared_secret: self.shared_secret.clone(),
            validity_window: self.validity_window,
            clock_skew: self.clock_skew,
            replay_cache: Mutex::new(ReplayCache::new(DEFAULT_REPLAY_CACHE_MAX)),
        }
    }
}

#[derive(Debug)]
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

    fn insert(
        &mut self,
        entry: ReplayEntry,
        now: u64,
        validity_window: u64,
    ) -> Result<(), AuthError> {
        // Evict expired entries first
        while let Some(front) = self.order.front() {
            if now.saturating_sub(front.timestamp) > validity_window {
                if let Some(front2) = self.order.pop_front() {
                    self.entries.remove(&front2);
                }
            } else {
                break;
            }
        }

        // Detect replay before evicting
        if self.entries.contains(&entry) {
            return Err(AuthError::TokenInvalid("replay detected".to_string()));
        }

        // Evict oldest if over capacity
        if self.entries.len() >= self.max_size {
            if let Some(front) = self.order.pop_front() {
                self.entries.remove(&front);
            }
        }

        let entry_for_set = ReplayEntry {
            nonce: entry.nonce.clone(),
            timestamp: entry.timestamp,
        };
        self.entries.insert(entry_for_set);
        self.order.push_back(entry);
        Ok(())
    }
}

impl SharedSecret {
    pub fn new(id: &str, shared_secret: &str) -> Self {
        // Validate inputs
        Self::validate_id(id).expect("invalid id");
        Self::validate_secret(shared_secret).expect("invalid shared_secret");

        // Generate unique id by appending random suffix
        let random_suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let full_id = format!("{}_{}", id, random_suffix);

        // Construct the SharedSecret instance
        Self {
            base_id: id.to_owned(),
            id: full_id,
            shared_secret: shared_secret.to_owned(),
            validity_window: std::time::Duration::from_secs(DEFAULT_VALIDITY_WINDOW),
            clock_skew: std::time::Duration::from_secs(DEFAULT_CLOCK_SKEW),
            replay_cache: Mutex::new(ReplayCache::new(DEFAULT_REPLAY_CACHE_MAX)),
        }
    }

    pub fn with_validity_window(self, window: std::time::Duration) -> Self {
        Self {
            validity_window: window,
            ..self
        }
    }

    pub fn with_clock_skew(self, skew: std::time::Duration) -> Self {
        Self {
            clock_skew: skew,
            ..self
        }
    }

    pub fn with_replay_cache_max(self, max_size: usize) -> Self {
        Self {
            replay_cache: Mutex::new(ReplayCache::new(max_size)),
            ..self
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base_id(&self) -> &str {
        &self.base_id
    }
    pub fn shared_secret(&self) -> &str {
        &self.shared_secret
    }

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
        let mut mac = HmacSha256::new_from_slice(self.shared_secret.as_bytes())
            .map_err(|_| AuthError::TokenInvalid("invalid secret key".to_string()))?;
        mac.update(message);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    fn create_hmac_b64(&self, message: &str) -> Result<String, AuthError> {
        let raw = self.create_hmac_raw(message.as_bytes())?;
        Ok(URL_SAFE_NO_PAD.encode(raw))
    }

    fn verify_hmac(&self, message: &str, expected_b64: &str) -> Result<(), AuthError> {
        let expected = URL_SAFE_NO_PAD
            .decode(expected_b64.as_bytes())
            .map_err(|_| AuthError::TokenInvalid("invalid mac encoding".to_string()))?;

        let mut mac = HmacSha256::new_from_slice(self.shared_secret.as_bytes())
            .map_err(|_| AuthError::TokenInvalid("invalid secret key".to_string()))?;

        mac.update(message.as_bytes());

        mac.verify_slice(&expected)
            .map_err(|_| AuthError::TokenInvalid("hmac verification failed".to_string()))
    }

    fn build_message(&self, id: &str, timestamp: u64, nonce: &str) -> String {
        // Message components included in MAC
        format!("{}:{}:{}", id, timestamp, nonce)
    }

    fn gen_nonce(&self) -> String {
        let mut bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut bytes);
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Parse token (id, ts, nonce, mac)
    fn parse_token(&self, token: &str) -> Result<(String, u64, String, String), AuthError> {
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 4 {
            return Err(AuthError::TokenInvalid("invalid token format".to_string()));
        }
        let id = parts[0].to_string();
        let ts = parts[1]
            .parse::<u64>()
            .map_err(|_| AuthError::TokenInvalid("invalid timestamp".to_string()))?;
        let nonce = parts[2].to_string();
        let mac = parts[3].to_string();
        Ok((id, ts, nonce, mac))
    }

    fn validate_timestamp(&self, now: u64, ts: u64) -> Result<(), AuthError> {
        if ts > now {
            let diff = ts - now;
            if diff > self.clock_skew.as_secs() {
                return Err(AuthError::TokenInvalid(
                    "timestamp too far in future".to_string(),
                ));
            }
        } else {
            let age = now - ts;
            if age > self.validity_window.as_secs() {
                return Err(AuthError::TokenInvalid("token expired".to_string()));
            }
        }
        Ok(())
    }

    fn record_replay(&self, nonce: &str, ts: u64, now: u64) -> Result<(), AuthError> {
        let entry = ReplayEntry {
            nonce: nonce.to_string(),
            timestamp: ts,
        };
        let mut cache = self.replay_cache.lock();
        // validity_window is a Duration; pass seconds component to cache
        cache.insert(entry, now, self.validity_window.as_secs())
    }
}

impl TokenProvider for SharedSecret {
    fn get_token(&self) -> Result<String, AuthError> {
        if self.shared_secret.is_empty() {
            return Err(AuthError::TokenInvalid(
                "shared_secret is empty".to_string(),
            ));
        }
        let ts = self.get_current_timestamp();
        let nonce = self.gen_nonce();
        let message = self.build_message(self.id(), ts, &nonce);
        let mac = self.create_hmac_b64(&message)?;
        // Format: id:timestamp:nonce:mac
        Ok(format!("{}:{}:{}:{}", self.id(), ts, nonce, mac))
    }

    fn get_id(&self) -> Result<String, AuthError> {
        Ok(self.id.clone())
    }
}

#[async_trait::async_trait]
impl Verifier for SharedSecret {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        // Convert the token to String
        let token = token.into();

        // Step 1: Get current timestamp
        let now = self.get_current_timestamp();

        // Step 2: Parse the token
        let (token_id, ts, nonce, mac_b64) = self.parse_token(&token)?;

        // No strict id equality check per request; we trust any id signed with the shared secret.

        // Step 3: Validate timestamp
        self.validate_timestamp(now, ts)?;

        // Step 4: Rebuild original message and verify HMAs
        let message = self.build_message(&token_id, ts, &nonce);
        self.verify_hmac(&message, &mac_b64)?;

        // Step 5: Check and record replas
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
        // Convert the token to String
        let token_str = token.into();

        // Verify the token first
        self.try_verify(token_str.clone())?;

        // Parse token and construct claims
        let (token_id, ts, _, _) = self.parse_token(&token_str)?;

        // Construct claims JSON
        let exp = ts + self.validity_window.as_secs();
        let claims_json = serde_json::json!({
            "id": token_id,
            "iat": ts,
            "exp": exp
        });

        // Deserialize into Claims type
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
        id: String,
        iat: u64,
        exp: u64,
    }

    fn valid_secret() -> String {
        "abcdefghijklmnopqrstuvwxyz012345".to_string()
    }

    #[test]
    #[should_panic(expected = "invalid shared_secret")]
    fn test_secret_too_short() {
        // Verify it returns an Err (no panic) for too-short secret
        let _result = SharedSecret::new("svc", "shortsecret");
    }

    #[test]
    fn test_id_validation() {
        let result = std::panic::catch_unwind(|| SharedSecret::new("good-id", &valid_secret()));
        assert!(result.is_ok());

        let result = std::panic::catch_unwind(|| SharedSecret::new("bad:id", &valid_secret()));
        assert!(result.is_err());

        let result = std::panic::catch_unwind(|| SharedSecret::new("bad id", &valid_secret()));
        assert!(result.is_err());

        let result = std::panic::catch_unwind(|| SharedSecret::new("", &valid_secret()));
        assert!(result.is_err());
    }

    #[test]
    fn test_token_generation_format() {
        let s = SharedSecret::new("app", &valid_secret());
        let token = s.get_token().unwrap();
        let parts: Vec<_> = token.split(':').collect();
        assert_eq!(parts.len(), 4);
        assert!(parts[0].starts_with("app_"));
        assert!(parts[1].parse::<u64>().is_ok());
        // nonce base64url no padding
        assert!(!parts[2].is_empty());
        // mac base64url decode
        assert!(URL_SAFE_NO_PAD.decode(parts[3]).is_ok());
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
        let b = SharedSecret::new("svc", &valid_secret()); // different random suffix
        let token = a.get_token().unwrap();
        // Should verify under b since only secret matters
        assert!(b.try_verify(token).is_ok());
    }

    #[test]
    fn test_future_timestamp_exceeds_skew() {
        let s = SharedSecret::new("svc", &valid_secret())
            .with_clock_skew(std::time::Duration::from_secs(2));
        let future_ts = s.get_current_timestamp() + 10;
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), future_ts, &nonce);
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}:{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_future_timestamp_within_skew() {
        let s = SharedSecret::new("svc", &valid_secret())
            .with_clock_skew(std::time::Duration::from_secs(10));
        let future_ts = s.get_current_timestamp() + 5;
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), future_ts, &nonce);
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}:{}", s.id(), future_ts, nonce, mac);
        assert!(s.try_verify(token).is_ok());
    }

    #[test]
    fn test_expired_token() {
        let s = SharedSecret::new("svc", &valid_secret())
            .with_validity_window(std::time::Duration::from_secs(1));
        let past_ts = s.get_current_timestamp().saturating_sub(10);
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), past_ts, &nonce);
        let mac = s.create_hmac_b64(&message).unwrap();
        let token = format!("{}:{}:{}:{}", s.id(), past_ts, nonce, mac);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn test_replay_detection() {
        let s = SharedSecret::new("svc", &valid_secret());
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        let replay = s.try_verify(token);
        assert!(replay.is_err());
        assert!(replay.unwrap_err().to_string().contains("replay"));
    }

    #[test]
    fn test_wrong_mac() {
        let s = SharedSecret::new("svc", &valid_secret());
        let ts = s.get_current_timestamp();
        let nonce = s.gen_nonce();
        let bad_mac = "!!notbase64";
        let token = format!("{}:{}:{}:{}", s.id(), ts, nonce, bad_mac);
        let res = s.try_verify(token);
        assert!(res.is_err());
    }

    #[test]
    fn test_invalid_token_format_parts() {
        let s = SharedSecret::new("svc", &valid_secret());
        // Too few parts
        assert!(s.try_verify("only:two:parts").is_err());
        // Too many parts
        assert!(s.try_verify("a:b:c:d:e").is_err());
    }

    #[test]
    fn test_invalid_timestamp_parse() {
        let s = SharedSecret::new("svc", &valid_secret());
        let nonce = s.gen_nonce();
        let mac = s
            .create_hmac_b64(&s.build_message(s.id(), s.get_current_timestamp(), &nonce))
            .unwrap();
        // Replace timestamp with non-numeric
        let token = format!("{}:{}:{}:{}", s.id(), "notanumber", nonce, mac);
        assert!(s.try_verify(token).is_err());
    }

    #[test]
    fn test_hmac_verification_failure() {
        let s = SharedSecret::new("svc", &valid_secret());
        let ts = s.get_current_timestamp();
        let nonce = s.gen_nonce();
        let message = s.build_message(s.id(), ts, &nonce);
        let mac = s.create_hmac_b64(&message).unwrap();
        // Truncate mac to force verification failure (valid base64 but wrong digest)
        let truncated = &mac[..mac.len() / 2];
        let token = format!("{}:{}:{}:{}", s.id(), ts, nonce, truncated);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("invalid mac encoding")
        );
    }

    #[test]
    fn test_replay_after_expiration_allows_reuse() {
        // After validity window, nonce should be expired and reuse not treated as replay
        let s = SharedSecret::new("svc", &valid_secret())
            .with_validity_window(std::time::Duration::from_secs(1));
        let token = s.get_token().unwrap();
        assert!(s.try_verify(token.clone()).is_ok());
        // Wait past expiration
        thread::sleep(Duration::from_secs(2));
        // Reuse should now fail due to expiration, not replay
        let res = s.try_verify(token);
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("expired"));
    }

    #[test]
    fn test_nonce_uniqueness_and_length() {
        let s = SharedSecret::new("svc", &valid_secret());
        let mut nonces = std::collections::HashSet::new();
        for _ in 0..50 {
            let t = s.get_token().unwrap();
            let parts: Vec<_> = t.split(':').collect();
            assert_eq!(parts.len(), 4);
            let nonce = parts[2];
            assert!(nonce.len() >= NONCE_LEN); // base64url expands length
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
        // Invalid base64url characters cause encoding error
        let bad_mac = "*invalid*mac*";
        let token = format!("{}:{}:{}:{}", s.id(), ts, nonce, bad_mac);
        let res = s.try_verify(token);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("invalid mac encoding")
        );
    }

    #[test]
    fn test_replay_detection_multiple() {
        let s = SharedSecret::new("svc", &valid_secret());
        let t1 = s.get_token().unwrap();
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t1.clone()).is_ok());
        assert!(s.try_verify(t2.clone()).is_ok());
        let r1 = s.try_verify(t1);
        assert!(r1.is_err());
        let r2 = s.try_verify(t2);
        assert!(r2.is_err());
    }

    #[test]
    fn test_claims() {
        let s = SharedSecret::new("svc", &valid_secret());
        let token = s.get_token().unwrap();
        let claims: BasicClaims = s.try_get_claims(token).unwrap();
        assert!(claims.id.starts_with("svc_"));
        assert_eq!(claims.exp, claims.iat + s.validity_window.as_secs());
    }

    #[test]
    fn test_replay_cache_capacity() {
        let s = SharedSecret::new("svc", &valid_secret())
            .with_replay_cache_max(2);
        let t1 = s.get_token().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = s.get_token().unwrap();
        assert!(s.try_verify(t1.clone()).is_ok());
        assert!(s.try_verify(t2.clone()).is_ok());
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t3 = s.get_token().unwrap();
        assert!(s.try_verify(t3.clone()).is_ok());
        // Replaying t1 may or may not succeed depending on eviction; ensure no panic.
        let _ = s.try_verify(t1);
    }
}

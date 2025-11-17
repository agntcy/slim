// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Identity claims abstraction for consistent handling between identity creation and verification.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value as JsonValue;
use slim_auth::metadata::MetadataMap;
use std::collections::HashMap;

use crate::errors::SlimIdentityError;

/// Claim key constants to ensure consistency across the codebase
pub mod claim_keys {
    /// Public key claim key
    pub const PUBKEY: &str = "pubkey";
    /// Subject claim key
    pub const SUBJECT: &str = "sub";
    /// Custom claims object key
    pub const CUSTOM_CLAIMS: &str = "custom_claims";
}

/// Represents the identity claims extracted from a token
#[derive(Debug, Clone)]
pub struct IdentityClaims {
    /// The subject (user/entity identifier)
    pub subject: String,
    /// The public key in base64 format
    pub public_key: String,
}

impl IdentityClaims {
    /// Creates a new IdentityClaims instance
    pub fn new(subject: impl Into<String>, public_key: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            public_key: public_key.into(),
        }
    }

    /// Extracts identity claims from a JSON value
    ///
    /// Tries to get the public key from the top-level claims first,
    /// then falls back to custom_claims if not found.
    pub fn from_json(claims: &JsonValue) -> Result<Self, SlimIdentityError> {
        // Try to get the public key from claims, falling back to custom_claims
        let public_key = claims
            .get(claim_keys::PUBKEY)
            .and_then(|pk| pk.as_str())
            .or_else(|| {
                claims
                    .get(claim_keys::CUSTOM_CLAIMS)
                    .and_then(|c| c.as_object())
                    .and_then(|cc| cc.get(claim_keys::PUBKEY))
                    .and_then(|pk| pk.as_str())
            })
            .ok_or(SlimIdentityError::PublicKeyNotFound)?;

        // Get the subject of this identity
        // Fall back to "id" field if "sub" is not present (for SharedSecret compatibility)
        let subject = claims
            .get(claim_keys::SUBJECT)
            .and_then(|s| s.as_str())
            .or_else(|| claims.get("id").and_then(|s| s.as_str()))
            .ok_or(SlimIdentityError::SubjectNotFound)?;

        Ok(Self {
            subject: subject.to_string(),
            public_key: public_key.to_string(),
        })
    }

    /// Converts identity claims to a HashMap suitable for token generation
    pub fn to_claims_map(&self) -> HashMap<String, JsonValue> {
        HashMap::from([
            (
                claim_keys::PUBKEY.to_string(),
                JsonValue::String(self.public_key.clone()),
            ),
            (
                claim_keys::SUBJECT.to_string(),
                JsonValue::String(self.subject.clone()),
            ),
        ])
    }

    /// Creates a claims map with just the public key (for backward compatibility)
    pub fn pubkey_only_claims_map(public_key: impl Into<String>) -> MetadataMap {
        let mut claims_map = MetadataMap::new();
        claims_map.insert(claim_keys::PUBKEY.to_string(), public_key.into());
        claims_map
    }

    /// Creates a claims map from raw public key bytes (base64 encoded)
    pub fn from_public_key_bytes(public_key_bytes: &[u8]) -> MetadataMap {
        let public_key_b64 = BASE64.encode(public_key_bytes);
        Self::pubkey_only_claims_map(public_key_b64)
    }

    /// Encodes public key bytes to base64 string
    pub fn encode_public_key(public_key_bytes: &[u8]) -> String {
        BASE64.encode(public_key_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_from_json_top_level_claims() {
        let claims = json!({
            "sub": "user123",
            "pubkey": "base64encodedkey"
        });

        let identity_claims = IdentityClaims::from_json(&claims).unwrap();
        assert_eq!(identity_claims.subject, "user123");
        assert_eq!(identity_claims.public_key, "base64encodedkey");
    }

    #[test]
    fn test_from_json_custom_claims() {
        let claims = json!({
            "sub": "user123",
            "custom_claims": {
                "pubkey": "base64encodedkey"
            }
        });

        let identity_claims = IdentityClaims::from_json(&claims).unwrap();
        assert_eq!(identity_claims.subject, "user123");
        assert_eq!(identity_claims.public_key, "base64encodedkey");
    }

    #[test]
    fn test_from_json_missing_pubkey() {
        let claims = json!({
            "sub": "user123"
        });

        let result = IdentityClaims::from_json(&claims);
        assert!(matches!(result, Err(SlimIdentityError::PublicKeyNotFound)));
    }

    #[test]
    fn test_from_json_missing_subject() {
        let claims = json!({
            "pubkey": "base64encodedkey"
        });

        let result = IdentityClaims::from_json(&claims);
        assert!(matches!(result, Err(SlimIdentityError::SubjectNotFound)));
    }

    #[test]
    fn test_to_claims_map() {
        let identity_claims = IdentityClaims::new("user123", "base64encodedkey");
        let claims_map = identity_claims.to_claims_map();

        assert_eq!(claims_map.len(), 2);
        assert_eq!(
            claims_map.get("pubkey").unwrap().as_str().unwrap(),
            "base64encodedkey"
        );
        assert_eq!(claims_map.get("sub").unwrap().as_str().unwrap(), "user123");
    }

    #[test]
    fn test_pubkey_only_claims_map() {
        let claims_map = IdentityClaims::pubkey_only_claims_map("base64encodedkey");

        assert_eq!(claims_map.len(), 1);
        let pkey: String = claims_map.get("pubkey").unwrap().try_into().unwrap();
        assert_eq!(&pkey, "base64encodedkey");
    }

    #[test]
    fn test_from_public_key_bytes() {
        let public_key_bytes = b"test_public_key";
        let claims_map = IdentityClaims::from_public_key_bytes(public_key_bytes);

        assert_eq!(claims_map.len(), 1);
        let encoded = claims_map.get("pubkey").unwrap().as_str().unwrap();
        // Verify it's base64 encoded
        assert!(!encoded.is_empty());
        assert_eq!(encoded, BASE64.encode(public_key_bytes));
    }

    #[test]
    fn test_encode_public_key() {
        let public_key_bytes = b"test_public_key";
        let encoded = IdentityClaims::encode_public_key(public_key_bytes);

        assert!(!encoded.is_empty());
        assert_eq!(encoded, BASE64.encode(public_key_bytes));
    }

    #[test]
    fn test_encode_public_key_roundtrip() {
        let original_bytes = b"some_key_data_here";
        let encoded = IdentityClaims::encode_public_key(original_bytes);

        // Verify we can decode it back
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap();
        assert_eq!(decoded, original_bytes);
    }
}

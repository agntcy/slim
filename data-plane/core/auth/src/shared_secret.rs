// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use rand::{Rng, distr::Alphanumeric};

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

#[derive(Debug, Clone)]
pub struct SharedSecret {
    /// Unique identifier for the entity
    id: String,

    /// Shared secret
    shared_secret: String,
}

impl SharedSecret {
    pub fn new(id: &str, shared_secret: &str) -> Self {
        // Generate a random 8-character suffix to append to the id
        let random_suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        let full_id = format!("{}_{}", id, random_suffix);

        Self {
            id: full_id,
            shared_secret: shared_secret.to_owned(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn shared_secret(&self) -> &str {
        &self.shared_secret
    }
}

impl TokenProvider for SharedSecret {
    fn get_token(&self) -> Result<String, AuthError> {
        if self.shared_secret.is_empty() {
            Err(AuthError::TokenInvalid(
                "shared_secret is empty".to_string(),
            ))
        } else {
            // Join the shared secret and id to create a token
            Ok(format!("{}:{}", self.shared_secret, self.id))
        }
    }

    fn get_id(&self) -> Result<String, AuthError> {
        // Return the id (which already includes the random suffix)
        Ok(self.id.clone())
    }
}

#[async_trait::async_trait]
impl Verifier for SharedSecret {
    async fn verify(&self, token: impl Into<String> + Send) -> Result<(), AuthError> {
        self.try_verify(token)
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), AuthError> {
        let token = token.into();

        // Split the token into shared_secret and id
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 2 {
            return Err(AuthError::TokenInvalid("invalid token format".to_string()));
        }

        if parts[0] == self.shared_secret {
            Ok(())
        } else {
            Err(AuthError::TokenInvalid(
                "shared secret mismatch".to_string(),
            ))
        }
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
        self.try_verify(token.into())?;
        Ok(serde_json::from_str(r#"{"exp":0}"#).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_secret_get_id() {
        let shared_secret = SharedSecret::new("test-id", "test-secret");

        let id = shared_secret.get_id().unwrap();

        // The ID should start with "test-id_" and have 8 random characters appended
        assert!(id.starts_with("test-id_"));
        assert_eq!(id.len(), "test-id_".len() + 8);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn test_shared_secret_get_id_deterministic() {
        let shared_secret = SharedSecret::new("test-id", "test-secret");

        let id1 = shared_secret.get_id().unwrap();
        let id2 = shared_secret.get_id().unwrap();

        // Since the random suffix is generated once in constructor, IDs should be the same
        assert_eq!(id1, id2);
        assert!(id1.starts_with("test-id_"));
    }

    #[test]
    fn test_shared_secret_get_token() {
        let shared_secret = SharedSecret::new("test-id", "test-secret");

        let token = shared_secret.get_token().unwrap();

        // Token should contain shared secret and the id with random suffix
        assert!(token.starts_with("test-secret:test-id_"));
        let parts: Vec<&str> = token.split(':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "test-secret");
        assert!(parts[1].starts_with("test-id_"));
    }

    #[test]
    fn test_shared_secret_get_token_empty_secret() {
        let shared_secret = SharedSecret::new("test-id", "");

        let result = shared_secret.get_token();

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("shared_secret is empty")
        );
    }

    #[test]
    fn test_shared_secret_different_instances_have_different_ids() {
        let shared_secret1 = SharedSecret::new("test-id", "test-secret");
        let shared_secret2 = SharedSecret::new("test-id", "test-secret");

        let id1 = shared_secret1.get_id().unwrap();
        let id2 = shared_secret2.get_id().unwrap();

        // Different instances should have different random suffixes
        assert_ne!(id1, id2);
        assert!(id1.starts_with("test-id_"));
        assert!(id2.starts_with("test-id_"));
    }

    #[test]
    fn test_shared_secret_verify() {
        let shared_secret = SharedSecret::new("test-id", "test-secret");

        // Get the actual token that would be generated
        let valid_token = shared_secret.get_token().unwrap();

        let result = shared_secret.try_verify(&valid_token);
        assert!(result.is_ok());

        let result = shared_secret.try_verify("wrong-secret:test-id_12345678");
        assert!(result.is_err());

        let result = shared_secret.try_verify("invalid-format");
        assert!(result.is_err());
    }

    #[test]
    fn test_shared_secret_id_format() {
        let shared_secret = SharedSecret::new("user123", "secret456");
        let id = shared_secret.get_id().unwrap();

        // Verify the ID format: original_id + "_" + 8 random chars
        assert!(id.starts_with("user123_"));
        assert_eq!(id.len(), "user123_".len() + 8);

        // Verify the suffix is alphanumeric
        let suffix = &id["user123_".len()..];
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_shared_secret_id_accessor() {
        let shared_secret = SharedSecret::new("test", "secret");

        // The id() accessor should return the full ID with random suffix
        let accessor_id = shared_secret.id();
        let get_id_result = shared_secret.get_id().unwrap();

        assert_eq!(accessor_id, get_id_result);
        assert!(accessor_id.starts_with("test_"));
    }
}

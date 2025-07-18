// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

#[derive(serde::Deserialize)]
struct EmptyClaims;

#[derive(Debug, Clone)]
pub struct SharedSecret {
    /// Unique identifier for the entity
    id: String,

    /// Shared secret
    shared_secret: String,
}

impl SharedSecret {
    pub fn new(id: &str, shared_secret: &str) -> Self {
        Self {
            id: id.to_owned(),
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
}

#[async_trait::async_trait]
impl Verifier for SharedSecret {
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.try_verify(token)
    }

    fn try_verify<Claims>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, crate::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let token = token.into();

        // Split the token into shared_secret and id
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 2 {
            return Err(AuthError::TokenInvalid("invalid token format".to_string()));
        }

        if parts[0] == self.shared_secret {
            Ok(serde_json::from_str(r#"{"exp":0}"#).unwrap())
        } else {
            Err(AuthError::TokenInvalid(
                "shared secret mismatch".to_string(),
            ))
        }
    }
}

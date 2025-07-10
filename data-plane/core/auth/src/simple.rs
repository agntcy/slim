// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

#[derive(serde::Deserialize)]
struct EmptyClaims;

#[derive(Debug, Clone)]
pub struct SimpleGroup {
    /// Unique identifier for the entity
    id: String,

    /// The group this identity belongs to
    group: String,
}

impl SimpleGroup {
    pub fn new(id: &str, group: &str) -> Self {
        Self {
            id: id.to_owned(),
            group: group.to_owned(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn group(&self) -> &str {
        &self.group
    }
}

impl TokenProvider for SimpleGroup {
    fn get_token(&self) -> Result<String, AuthError> {
        if self.group.is_empty() {
            Err(AuthError::TokenInvalid("group is empty".to_string()))
        } else {
            // Join the group and id to create a token
            Ok(format!("{}:{}", self.group, self.id))
        }
    }
}

#[async_trait::async_trait]
impl Verifier for SimpleGroup {
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

        // Split the token into group and id
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 2 {
            return Err(AuthError::TokenInvalid("invalid token format".to_string()));
        }

        if parts[0] == self.group {
            Ok(serde_json::from_str(r#"{"exp":0}"#).unwrap())
        } else {
            Err(AuthError::TokenInvalid("invalid token".to_string()))
        }
    }
}

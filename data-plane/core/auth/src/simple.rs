// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::{
    errors::AuthError,
    traits::{TokenProvider, Verifier},
};

#[derive(Debug, Clone)]
pub struct Simple {
    token: String,
}

impl Simple {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_owned(),
        }
    }

    pub fn get_token(&self) -> &str {
        &self.token
    }
}

#[async_trait::async_trait]
impl TokenProvider for Simple {
    async fn get_token(&self) -> Result<String, AuthError> {
        Ok(async { self.token.clone() }.await)
    }

    fn try_get_token(&self) -> Result<String, crate::errors::AuthError> {
        Ok(self.token.clone())
    }
}

#[async_trait::async_trait]
impl Verifier for Simple {
    async fn verify<Claims>(&self, token: impl Into<String> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let token = token.into();

        if token == self.token {
            // Here you would typically decode the token and return the claims
            Ok(serde_json::from_str("{}").unwrap()) // Placeholder for actual claims
        } else {
            Err(AuthError::TokenInvalid("invalid token".to_string()))
        }
    }

    fn try_verify<Claims>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, crate::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let token = token.into();

        if token == self.token {
            Ok(serde_json::from_str("{}").unwrap()) // Placeholder for actual claims
        } else {
            Err(AuthError::TokenInvalid("invalid token".to_string()))
        }
    }
}

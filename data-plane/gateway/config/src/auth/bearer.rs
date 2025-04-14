// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use tower_http::auth::{AddAuthorizationLayer, require_authorization::Bearer};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use crate::opaque::OpaqueString;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Config {
    token: OpaqueString,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            token: OpaqueString::new("token"),
        }
    }
}

impl Config {
    /// Create a new Config
    pub fn new(token: &str) -> Self {
        Config {
            token: OpaqueString::new(token),
        }
    }

    /// Get the token
    pub fn token(&self) -> &OpaqueString {
        &self.token
    }
}

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = AddAuthorizationLayer;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        match self.token().as_ref() {
            "" => Err(AuthError::ConfigError("token is empty".to_string())),
            _ => Ok(AddAuthorizationLayer::bearer(self.token())),
        }
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default,
{
    // Associated types
    type ServerLayer = ValidateRequestHeaderLayer<Bearer<Response>>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        Ok(ValidateRequestHeaderLayer::bearer(self.token()))
    }
}

// tests
#[cfg(test)]
mod tests {
    use tower::ServiceBuilder;
    use tower_reqwest::HttpClientLayer;

    use super::*;

    #[test]
    fn test_config() {
        let token = OpaqueString::new("token");
        let config = Config::new(&token);

        assert_eq!(config.token(), &token);
    }

    #[tokio::test]
    async fn test_authenticator() {
        let token = OpaqueString::new("token");
        let config = Config::new(&token);

        let client_layer = config.get_client_layer().unwrap();
        let server_layer: ValidateRequestHeaderLayer<Bearer<String>> =
            config.get_server_layer().unwrap();

        // Check that we can use the layers when building a service
        let _ = ServiceBuilder::new().layer(server_layer);

        let _ = ServiceBuilder::new()
            .layer(HttpClientLayer)
            .layer(client_layer);
    }
}

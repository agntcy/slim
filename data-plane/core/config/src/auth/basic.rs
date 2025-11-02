// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower_http::auth::{AddAuthorizationLayer, require_authorization::Basic};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use crate::opaque::OpaqueString;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    /// The username the client will use to authenticate.
    username: String,

    /// The password for the username.
    password: OpaqueString,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            username: "admin".to_string(),
            password: OpaqueString::new("password"),
        }
    }
}

impl Config {
    /// Create a new Config
    pub fn new(username: &str, password: &str) -> Self {
        Config {
            username: username.to_string(),
            password: OpaqueString::new(password),
        }
    }

    /// Get the username
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Get the password
    pub fn password(&self) -> &OpaqueString {
        &self.password
    }
}

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = AddAuthorizationLayer;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        match (self.username(), self.password().as_ref()) {
            ("", _) => Err(AuthError::ConfigError("username is empty".to_string())),
            (_, "") => Err(AuthError::ConfigError("password is empty".to_string())),
            _ => Ok(AddAuthorizationLayer::basic(
                self.username(),
                self.password(),
            )),
        }
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default,
{
    // Associated types
    type ServerLayer = ValidateRequestHeaderLayer<Basic<Response>>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        Ok(ValidateRequestHeaderLayer::basic(
            self.username(),
            self.password(),
        ))
    }
}

// tests
#[cfg(test)]
mod tests {
    use crate::testutils::tower_service::HeaderCheckService;
    use tower::ServiceBuilder;

    use super::*;

    #[test]
    fn test_config() {
        let username = "admin".to_string();
        let password = OpaqueString::new("password");
        let config = Config::new(&username, &password);

        assert_eq!(config.username(), username);
        assert_eq!(config.password(), &password);
    }

    #[tokio::test]
    async fn test_authenticator() {
        let username = "admin".to_string();
        let password = OpaqueString::new("password");
        let config = Config::new(&username, &password);

        let client_layer = config.get_client_layer().unwrap();
        let server_layer: ValidateRequestHeaderLayer<Basic<String>> =
            config.get_server_layer().unwrap();

        // Check that we can use the layers when building a service
        let _ = ServiceBuilder::new().layer(server_layer);

        let _ = ServiceBuilder::new()
            .layer(HeaderCheckService)
            .layer(client_layer);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use duration_str::deserialize_duration;
use http::HeaderValue;
use serde::Deserialize;
use slim_auth::builder::JwtBuilder;
use tower_http::auth::{AddAuthorizationLayer, require_authorization::Basic};
use tower_http::validate_request::ValidateRequestHeaderLayer;
use tower_layer::Layer;

use crate::auth::jwt_middleware::{SignerLayer, VerifierLayer};

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use crate::opaque::OpaqueString;
use slim_auth::jwt::{Algorithm, Jwt, Key, SignerJwt, VerifierJwt};
use slim_auth::traits::{Claimer, Signer, Verifier};

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Claims {
    /// JWT audience
    audience: Option<String>,

    /// JWT Issuer
    issuer: Option<String>,

    /// JWT Subject
    subject: Option<String>,

    /// JWT Duration (will becomde exp: now() + duration)
    #[serde(
        default = "default_duration",
        deserialize_with = "deserialize_duration"
    )]
    duration: Duration,

    // Other claims
    custom_claims: Option<std::collections::HashMap<String, serde_yaml::Value>>,
}

impl Default for Claims {
    fn default() -> Self {
        Claims {
            audience: None,
            issuer: None,
            subject: None,
            duration: default_duration(),
            custom_claims: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
enum JwtKey {
    Encoding(Key),
    Decoding(Key),
    Autoresolve(bool),
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Config {
    /// Claims
    #[serde(default)]
    claims: Claims,

    /// JWT Duration in seconds
    #[serde(
        default = "default_duration",
        deserialize_with = "deserialize_duration"
    )]
    duration: Duration,

    /// One of: `encoding`, `decoding`, or `autoresolve`
    #[serde(with = "serde_yaml::with::singleton_map")]
    key: JwtKey,
}

fn default_duration() -> Duration {
    Duration::from_secs(3600)
}

impl Config {
    /// Create a new Config
    pub fn new(claims: Claims, duration: Duration, key: JwtKey) -> Self {
        Config {
            claims,
            duration,
            key,
        }
    }

    /// Get the claims
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Get the duration
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Get the key
    pub fn key(&self) -> &JwtKey {
        &self.key
    }
}

// Using the JWT middleware from jwt_middleware.rs

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = SignerLayer<SignerJwt>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        let signer = match self.key() {
            JwtKey::Encoding(key) => {
                // Use the builder pattern to construct the JWT
                let mut builder = JwtBuilder::new();

                // Set optional fields
                if let Some(issuer) = &self.claims().issuer {
                    builder = builder.issuer(issuer);
                }
                if let Some(audience) = &self.claims().audience {
                    builder = builder.audience(audience);
                }
                if let Some(subject) = &self.claims().subject {
                    builder = builder.subject(subject);
                }

                // Set the key and build
                builder
                    .private_key(key.algorithm, &key.key)
                    .build()
                    .map_err(|e| AuthError::ConfigError(e.to_string()))?
            }
            _ => {
                return Err(AuthError::ConfigError(
                    "Encoding key is required for client authentication".to_string(),
                ));
            }
        };

        // Add custom claims if any
        let mut claims = std::collections::HashMap::<String, serde_json::Value>::new();
        if let Some(custom_claims) = &self.claims().custom_claims {
            // Convert yaml values to json values
            claims = custom_claims
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap()))
                .collect();
        }

        // Create token duration in seconds
        let duration = self.duration.as_secs();

        Ok(SignerLayer::new(signer, claims, duration))
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    // Associated types
    type ServerLayer = VerifierLayer<VerifierJwt>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        let jwt = match self.key() {
            JwtKey::Decoding(key) => {
                // Use the builder pattern to construct the JWT
                let mut builder = JwtBuilder::new();

                // Set optional fields
                if let Some(issuer) = &self.claims().issuer {
                    builder = builder.issuer(issuer);
                }
                if let Some(audience) = &self.claims().audience {
                    builder = builder.audience(audience);
                }
                if let Some(subject) = &self.claims().subject {
                    builder = builder.subject(subject);
                }

                // Set the key and build
                builder
                    .public_key(key.algorithm, &key.key)
                    .build()
                    .map_err(|e| AuthError::ConfigError(e.to_string()))?
            }
            JwtKey::Autoresolve(true) => {
                return Err(AuthError::ConfigError(
                    "Autoresolve not implemented yet".to_string(),
                ));
            }
            _ => {
                return Err(AuthError::ConfigError(
                    "Decoding key is required for server authentication".to_string(),
                ));
            }
        };

        // Create standard claims for verification
        let claims = jwt.create_standard_claims(None);

        Ok(VerifierLayer::new(jwt, claims))
    }
}

// tests
#[cfg(test)]
mod tests {
    // use slim_auth::jwt::Algorithm;
    // use tower::ServiceBuilder;
    // use tower_reqwest::HttpClientLayer;

    // use super::*;

    // #[test]
    // fn test_config() {
    //     let claims = Claims {
    //         audience: Some("audience".to_string()),
    //         issuer: Some("issuer".to_string()),
    //         subject: Some("subject".to_string()),
    //         duration: Duration::from_secs(3600),
    //         custom_claims: None,
    //     };

    //     let key = JwtKey::Encoding(Key {
    //         algorithm: Algorithm::HS256,
    //         key: "test-key".to_string(),
    //     });

    //     let config = Config::new(claims.clone(), Duration::from_secs(3600), key);

    //     assert_eq!(config.claims(), &claims);
    //     assert_eq!(config.duration(), Duration::from_secs(3600));
    // }

    // #[tokio::test]
    // async fn test_authenticator() {
    //     let claims = Claims {
    //         audience: Some("audience".to_string()),
    //         issuer: Some("issuer".to_string()),
    //         subject: Some("subject".to_string()),
    //         duration: Duration::from_secs(3600),
    //         custom_claims: None,
    //     };

    //     let encoding_key = JwtKey::Encoding(Key {
    //         algorithm: Algorithm::HS256,
    //         key: "test-key".to_string(),
    //     });

    //     let decoding_key = JwtKey::Decoding(Key {
    //         algorithm: Algorithm::HS256,
    //         key: "test-key".to_string(),
    //     });

    //     let client_config = Config::new(claims.clone(), Duration::from_secs(3600), encoding_key);
    //     let server_config = Config::new(claims.clone(), Duration::from_secs(3600), decoding_key);

    //     let client_layer = client_config.get_client_layer().unwrap();
    //     let server_layer = server_config.get_server_layer().unwrap();

    //     // Check that we can use the layers when building a service
    //     let _ = ServiceBuilder::new().layer(server_layer);

    //     let _ = ServiceBuilder::new()
    //         .layer(HttpClientLayer)
    //         .layer(client_layer);
    // }
}

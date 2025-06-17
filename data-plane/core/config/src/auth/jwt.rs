// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::Duration;

use duration_str::deserialize_duration;
use serde::Deserialize;
use slim_auth::builder::JwtBuilder;

use slim_auth::jwt_middleware::{SignJwtLayer, ValidateJwtLayer};

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use slim_auth::jwt::{Key, SignerJwt, VerifierJwt};

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

impl Claims {
    /// Create a new Claims
    pub fn new(
        audience: Option<String>,
        issuer: Option<String>,
        subject: Option<String>,
        duration: Duration,
        custom_claims: Option<std::collections::HashMap<String, serde_yaml::Value>>,
    ) -> Self {
        Claims {
            audience,
            issuer,
            subject,
            duration,
            custom_claims,
        }
    }

    pub fn with_audience(self, audience: impl Into<String>) -> Self {
        Claims {
            audience: Some(audience.into()),
            ..self
        }
    }

    pub fn with_issuer(self, issuer: impl Into<String>) -> Self {
        Claims {
            issuer: Some(issuer.into()),
            ..self
        }
    }

    pub fn with_subject(self, subject: impl Into<String>) -> Self {
        Claims {
            subject: Some(subject.into()),
            ..self
        }
    }

    pub fn with_duration(self, duration: Duration) -> Self {
        Claims { duration, ..self }
    }

    pub fn with_custom_claims(
        self,
        custom_claims: std::collections::HashMap<String, serde_yaml::Value>,
    ) -> Self {
        Claims {
            custom_claims: Some(custom_claims),
            ..self
        }
    }

    /// Get the audience
    pub fn audience(&self) -> &Option<String> {
        &self.audience
    }

    /// Get the issuer
    pub fn issuer(&self) -> &Option<String> {
        &self.issuer
    }

    /// Get the subject
    pub fn subject(&self) -> &Option<String> {
        &self.subject
    }

    /// Get the duration
    pub fn duration(&self) -> Duration {
        self.duration
    }
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
pub enum JwtKey {
    Encoding(Key),
    Decoding(Key),
    Autoresolve(bool),
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Config {
    /// Claims
    #[serde(default)]
    claims: Claims,

    /// One of: `encoding`, `decoding`, or `autoresolve`
    /// Encoding key is used for signing JWTs (client-side).
    /// Decoding key is used for verifying JWTs (server-side).
    /// Autoresolve is used to automatically resolve the key based on the claims.
    #[serde(with = "serde_yaml::with::singleton_map")]
    key: JwtKey,
}

fn default_duration() -> Duration {
    Duration::from_secs(3600)
}

impl Config {
    /// Create a new Config
    pub fn new(claims: Claims, key: JwtKey) -> Self {
        Config { claims, key }
    }

    /// Set claims
    pub fn with_claims(self, claims: Claims) -> Self {
        Config { claims, ..self }
    }

    /// Set key
    pub fn with_key(self, key: JwtKey) -> Self {
        Config { key, ..self }
    }

    /// Get the claims
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Get the key
    pub fn key(&self) -> &JwtKey {
        &self.key
    }

    fn custom_claims(&self) -> HashMap<String, serde_json::Value> {
        let mut claims = std::collections::HashMap::<String, serde_json::Value>::new();
        if let Some(custom_claims) = &self.claims().custom_claims {
            // Convert yaml values to json values
            claims = custom_claims
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap()))
                .collect();
        }

        claims
    }
}

// Using the JWT middleware from jwt_middleware.rs

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = SignJwtLayer<SignerJwt>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
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

        let signer = match self.key() {
            JwtKey::Encoding(key) => builder
                .private_key(key)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string()))?,
            _ => {
                return Err(AuthError::ConfigError(
                    "Encoding key is required for client authentication".to_string(),
                ));
            }
        };

        // Add custom claims if any
        let custom_claims = self.custom_claims();

        // Create token duration in seconds
        let duration = self.claims.duration.as_secs();

        Ok(SignJwtLayer::new(signer, custom_claims, duration))
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    // Associated types
    type ServerLayer = ValidateJwtLayer<HashMap<String, serde_json::Value>, VerifierJwt>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
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

        let verifier = match self.key() {
            JwtKey::Decoding(key) => builder
                .public_key(key)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string()))?,
            JwtKey::Autoresolve(true) => builder
                .auto_resolve_keys(true)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string()))?,
            _ => {
                return Err(AuthError::ConfigError(
                    "Decoding key or autoresolve = true is required for server authentication"
                        .to_string(),
                ));
            }
        };

        // Create standard claims for verification
        let custom_claims = self.custom_claims();

        Ok(ValidateJwtLayer::new(verifier, custom_claims))
    }
}

// tests
#[cfg(test)]
mod tests {
    use futures::future::{self, Ready};
    use futures::task::Poll;
    use http::{Request, Response, StatusCode};
    use slim_auth::jwt::Algorithm;
    use slim_auth::jwt::KeyData;
    use std::task::Context;
    use tower::Service;
    use tower::ServiceBuilder;

    use super::*;

    // Define a Body type for testing
    type Body = Vec<u8>;

    // A simple test service that returns a 200 OK response
    #[derive(Clone)]
    struct HeaderCheckService;
    impl Service<Request<Body>> for HeaderCheckService {
        type Response = Response<Body>;
        type Error = std::convert::Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<Body>) -> Self::Future {
            // Check if the Authorization header exists and starts with "Bearer "
            let auth_header = req.headers().get(http::header::AUTHORIZATION);
            let has_bearer = auth_header
                .and_then(|h| h.to_str().ok())
                .map(|s| s.starts_with("Bearer "))
                .unwrap_or(false);

            if has_bearer {
                future::ready(Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from("Authorization header is present and correct"))
                    .unwrap()))
            } else {
                future::ready(Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("Missing or invalid Authorization header"))
                    .unwrap()))
            }
        }
    }

    #[test]
    fn test_config() {
        let claims = Claims {
            audience: Some("audience".to_string()),
            issuer: Some("issuer".to_string()),
            subject: Some("subject".to_string()),
            duration: Duration::from_secs(3600),
            custom_claims: None,
        };

        let key = JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            key: KeyData::Pem("test-key".to_string()),
        });

        let config = Config::new(claims.clone(), key);

        assert_eq!(config.claims(), &claims);
        assert_eq!(config.claims().duration, Duration::from_secs(3600));
    }

    #[tokio::test]
    async fn test_authenticator() {
        let claims = Claims {
            audience: Some("audience".to_string()),
            issuer: Some("issuer".to_string()),
            subject: Some("subject".to_string()),
            duration: Duration::from_secs(3600),
            custom_claims: None,
        };

        let encoding_key = JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            key: KeyData::Pem("test-key".to_string()),
        });

        let decoding_key = JwtKey::Decoding(Key {
            algorithm: Algorithm::HS256,
            key: KeyData::Pem("test-key".to_string()),
        });

        let client_config = Config::new(claims.clone(), encoding_key);
        let server_config = Config::new(claims.clone(), decoding_key);

        // Construct a client service that adds a JWT token
        let _client = ServiceBuilder::new()
            .layer(client_config.get_client_layer().unwrap())
            .service(HeaderCheckService);

        // Construct a server service that verifies the JWT token
        let _server = ServiceBuilder::new()
            .layer(
                <Config as ServerAuthenticator<Response<Body>>>::get_server_layer(&server_config)
                    .unwrap(),
            )
            .service(HeaderCheckService);
    }
}

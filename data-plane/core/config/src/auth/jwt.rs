// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::builder::JwtBuilder;

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use slim_auth::jwt::{Key, SignerJwt, VerifierJwt};
use slim_auth::jwt_middleware::{AddJwtLayer, ValidateJwtLayer};
use slim_auth::metadata::MetadataMap;

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Claims {
    /// JWT audience
    #[serde(alias = "aud", alias = "audiences")]
    audience: Option<Vec<String>>,

    /// JWT Issuer
    #[serde(alias = "iss")]
    issuer: Option<String>,

    /// JWT Subject
    #[serde(alias = "sub")]
    subject: Option<String>,

    // Other claims
    #[serde(default)]
    custom_claims: Option<MetadataMap>,
}

impl Claims {
    /// Create a new Claims
    pub fn new(
        audience: Option<Vec<String>>,
        issuer: Option<String>,
        subject: Option<String>,
        custom_claims: Option<MetadataMap>,
    ) -> Self {
        Claims {
            audience,
            issuer,
            subject,
            custom_claims,
        }
    }

    pub fn with_audience(self, audience: &[impl Into<String> + Clone]) -> Self {
        Claims {
            audience: Some(audience.iter().map(|a| a.clone().into()).collect()),
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

    pub fn with_custom_claims(self, custom_claims: MetadataMap) -> Self {
        Claims {
            custom_claims: Some(custom_claims),
            ..self
        }
    }

    /// Get the audience
    pub fn audience(&self) -> &Option<Vec<String>> {
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
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum JwtKey {
    Encoding(Key),
    Decoding(Key),
    Autoresolve,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    /// Claims
    #[serde(default)]
    claims: Claims,

    /// JWT Duration (will become exp: now() + duration)
    #[serde(default = "default_duration")]
    #[schemars(with = "String")]
    duration: DurationString,

    /// One of: `encoding`, `decoding`, or `autoresolve`
    /// Encoding key is used for signing JWTs (client-side).
    /// Decoding key is used for verifying JWTs (server-side).
    /// Autoresolve is used to automatically resolve the key based on the claims.
    key: JwtKey,
}

fn default_duration() -> DurationString {
    Duration::from_secs(3600).into()
}

impl Config {
    /// Create a new Config
    pub fn new(claims: Claims, duration: Duration, key: JwtKey) -> Self {
        Config {
            claims,
            duration: duration.into(),
            key,
        }
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

    fn custom_claims(&self) -> MetadataMap {
        self.claims.custom_claims.clone().unwrap_or_default()
    }

    /// Internal helper to build a base JwtBuilder with configured standard claims.
    fn jwt_builder(&self) -> JwtBuilder {
        let mut builder = JwtBuilder::new();
        if let Some(issuer) = &self.claims().issuer {
            builder = builder.issuer(issuer);
        }
        if let Some(audience) = &self.claims().audience {
            builder = builder.audience(audience);
        }
        if let Some(subject) = &self.claims().subject {
            builder = builder.subject(subject);
        }
        builder
    }

    /// Build and return a SignerJwt from this configuration.
    /// Returns an error if the configuration does not contain an encoding key.
    pub fn get_provider(&self) -> Result<SignerJwt, AuthError> {
        match self.key() {
            JwtKey::Encoding(key) => {
                let custom_claims = self.custom_claims();
                self.jwt_builder()
                    .private_key(key)
                    .custom_claims(custom_claims)
                    .build()
                    .map_err(|e| AuthError::ConfigError(e.to_string()))
            }
            _ => Err(AuthError::ConfigError(
                "Encoding key is required for client authentication".to_string(),
            )),
        }
    }

    /// Build and return a VerifierJwt from this configuration.
    /// Returns an error if neither a decoding key nor autoresolve=true are configured.
    pub fn get_verifier(&self) -> Result<VerifierJwt, AuthError> {
        match self.key() {
            JwtKey::Decoding(key) => self
                .jwt_builder()
                .public_key(key)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string())),
            JwtKey::Autoresolve => self
                .jwt_builder()
                .auto_resolve_keys(true)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string())),
            _ => Err(AuthError::ConfigError(
                "Decoding key or autoresolve = true is required for server authentication"
                    .to_string(),
            )),
        }
    }
}

// Using the JWT middleware from jwt_middleware.rs

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = AddJwtLayer<SignerJwt>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, AuthError> {
        let signer = self.get_provider()?;
        let duration = self.duration.as_secs();
        Ok(AddJwtLayer::new(signer, duration))
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    // Associated types
    type ServerLayer = ValidateJwtLayer<MetadataMap, VerifierJwt>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, AuthError> {
        let verifier = self.get_verifier()?;
        let custom_claims = self.custom_claims();
        Ok(ValidateJwtLayer::new(verifier, custom_claims))
    }
}

// tests
#[cfg(test)]
mod tests {
    use crate::testutils::tower_service::{Body, HeaderCheckService};
    use crate::tls::provider::initialize_crypto_provider;
    use http::Response;
    use serde_json;
    use slim_auth::jwt::Algorithm;
    use slim_auth::jwt::KeyData;
    use slim_auth::jwt::KeyFormat;
    use tower::ServiceBuilder;

    use super::*;

    #[test]
    fn test_config() {
        let claims = Claims {
            audience: Some(vec!["audience".to_string()]),
            issuer: Some("issuer".to_string()),
            subject: Some("subject".to_string()),
            custom_claims: None,
        };

        let key = JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("test-key".to_string()),
        });

        let config = Config::new(claims.clone(), Duration::from_secs(3600), key);

        assert_eq!(config.claims(), &claims);
        assert_eq!(config.duration, Duration::from_secs(3600));
    }

    #[tokio::test]
    async fn test_authenticator() {
        let claims = Claims {
            audience: Some(vec!["audience".to_string()]),
            issuer: Some("issuer".to_string()),
            subject: Some("subject".to_string()),
            custom_claims: None,
        };

        let encoding_key = JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("test-key".to_string()),
        });

        let decoding_key = JwtKey::Decoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("test-key".to_string()),
        });

        let client_config = Config::new(claims.clone(), Duration::from_secs(3600), encoding_key);
        let server_config = Config::new(claims.clone(), Duration::from_secs(3600), decoding_key);

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

    #[test]
    fn test_jwt_config_valid_duration_deserialize() {
        // Use autoresolve to avoid specifying key details
        let json = r#"{
            "duration": "1h2m3s",
            "key": { "type": "autoresolve" }
        }"#;
        let cfg: Config = serde_json::from_str(json).expect("valid duration should deserialize");
        assert_eq!(cfg.duration, Duration::from_secs(3600 + 120 + 3));

        let json = r#"{
            "duration": "750ms",
            "key": { "type": "autoresolve" }
        }"#;
        let cfg: Config = serde_json::from_str(json).expect("millis duration should deserialize");
        assert_eq!(cfg.duration, Duration::from_millis(750));
    }

    #[test]
    fn test_jwt_config_invalid_duration_deserialize() {
        let cases = [
            r#"{ "duration": "abc", "key": { "type": "autoresolve" } }"#,
            r#"{ "duration": "10x", "key": { "type": "autoresolve" } }"#,
            r#"{ "duration": "-5s", "key": { "type": "autoresolve" } }"#,
        ];
        for js in cases {
            let res: Result<Config, _> = serde_json::from_str(js);
            assert!(res.is_err(), "expected error for json: {}", js);
        }
    }

    #[test]
    fn test_jwt_config_duration_roundtrip() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(125),
            JwtKey::Autoresolve,
        );
        let ser = serde_json::to_string(&cfg).expect("serialize");
        let de: Config = serde_json::from_str(&ser).expect("deserialize");
        assert_eq!(de.duration, Duration::from_secs(125));
    }

    #[test]
    fn test_get_signer_ok() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Encoding(Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-signing-key".to_string()),
            }),
        );
        let signer = cfg.get_provider();
        assert!(
            signer.is_ok(),
            "expected signer to be created: {:?}",
            signer.err()
        );
    }

    #[test]
    fn test_get_signer_err_with_decoding_key() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Decoding(Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("verification-key".to_string()),
            }),
        );
        let signer = cfg.get_provider();
        assert!(
            signer.is_err(),
            "expected error when using decoding key for signer"
        );
    }

    #[test]
    fn test_get_verifier_ok_with_decoding_key() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Decoding(Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("verification-key".to_string()),
            }),
        );
        let verifier = cfg.get_verifier();
        assert!(
            verifier.is_ok(),
            "expected verifier to be created: {:?}",
            verifier.err()
        );
    }

    #[test]
    fn test_get_verifier_ok_with_autoresolve() {
        // init crypto provider first
        initialize_crypto_provider();
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Autoresolve,
        );
        let verifier = cfg.get_verifier();
        assert!(
            verifier.is_ok(),
            "expected verifier with autoresolve: {:?}",
            verifier.err()
        );
    }

    #[test]
    fn test_get_verifier_err_with_encoding_key() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Encoding(Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("secret-signing-key".to_string()),
            }),
        );
        let verifier = cfg.get_verifier();
        assert!(
            verifier.is_err(),
            "expected error when using encoding key for verifier"
        );
    }
}

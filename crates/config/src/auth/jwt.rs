// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::builder::JwtBuilder;

use std::path::PathBuf;

use crate::auth::ConfigAuthError;
use slim_auth::errors::AuthError;
use slim_auth::identity_binding::IdentityBinding;

use super::{ClientAuthenticator, PolicyConfig, ServerAuthenticator};
use slim_auth::jwt::{Key, SignerJwt, VerifierJwt};
use slim_auth::jwt_middleware::{AddJwtLayer, PolicyCheckLayer, ValidateJwtLayer};
use slim_auth::metadata::MetadataMap;
use tower_layer::Stack;

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

    /// Optional policy evaluated against JWT claims on every authenticated request.
    /// Absent means all authenticated requests are allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyConfig>,

    /// For a decoding key that is a list of peer keys (a JWKS where each key is
    /// one peer's): whether a token's `sub` must be the `kid`, or the `did:key`,
    /// of the key that signed it. `off`, `warn` or `enforce`; absent means not
    /// checked, as for an identity provider's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject_binding: Option<IdentityBinding>,
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
            policy: None,
            subject_binding: None,
        }
    }

    /// Set an inline Rego policy evaluated against JWT claims on every request
    pub fn with_rego_policy(mut self, text: impl Into<String>) -> Self {
        self.policy = Some(PolicyConfig::Rego(text.into()));
        self
    }

    /// Set a path to a `.rego` file read at server startup
    pub fn with_rego_policy_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.policy = Some(PolicyConfig::RegoFile(path.into()));
        self
    }

    /// Set a CEL expression evaluated against JWT claims on every request
    pub fn with_cel_policy(mut self, expression: impl Into<String>) -> Self {
        self.policy = Some(PolicyConfig::Cel(expression.into()));
        self
    }

    /// Hold each token's `sub` to the peer key that signed it; see the field.
    pub fn with_subject_binding(mut self, mode: IdentityBinding) -> Self {
        self.subject_binding = Some(mode);
        self
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

    /// How a token's `sub` is held to the peer key that signed it, if at all.
    pub fn subject_binding(&self) -> Option<IdentityBinding> {
        self.subject_binding
    }

    /// Get the duration
    pub fn duration(&self) -> Duration {
        *self.duration
    }

    fn custom_claims(&self) -> MetadataMap {
        self.claims.custom_claims.clone().unwrap_or_default()
    }

    /// Internal helper to build a base JwtBuilder with configured standard claims.
    fn jwt_builder(&self) -> JwtBuilder {
        self.jwt_builder_impl(true)
    }

    fn jwt_builder_impl(&self, include_issuer: bool) -> JwtBuilder {
        let mut builder = JwtBuilder::new();
        if include_issuer && let Some(issuer) = &self.claims().issuer {
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
            }
            _ => Err(AuthError::JwtMissingPrivateKey),
        }
    }

    /// Build and return a VerifierJwt from this configuration.
    /// Returns an error if neither a decoding key nor autoresolve=true are configured.
    pub fn get_verifier(&self) -> Result<VerifierJwt, AuthError> {
        match self.key() {
            JwtKey::Decoding(key) => {
                let verifier = self.jwt_builder().public_key(key).build()?;
                Ok(match self.subject_binding {
                    Some(mode) => verifier.with_subject_binding(mode),
                    None => verifier,
                })
            }
            JwtKey::Autoresolve => self.jwt_builder().auto_resolve_keys(true).build(),
            _ => Err(AuthError::JwtMissingDecodingKeyOrKeyResolver),
        }
    }

    /// Build a verifier that derives its decoding key from each token's own
    /// `iss` claim (a `did:key:z...` Ed25519 identifier) instead of from
    /// `key()` -- `key()` is not consulted for this mode. `claims.issuer` is
    /// also ignored: each peer's `iss` is its own did:key, so there is no
    /// single expected issuer to validate against (a configured value is
    /// logged and dropped rather than silently rejecting every token).
    /// Reachable only via
    /// [`crate::auth::identity::IdentityVerifierConfig::InsecureSelfCertifiedDid`].
    #[cfg(feature = "insecure-self-certified-did")]
    pub fn get_self_certified_did_verifier(&self) -> Result<VerifierJwt, AuthError> {
        if let Some(issuer) = &self.claims().issuer {
            tracing::warn!(
                issuer,
                "ignoring configured issuer: self-certified-did mode verifies each token's own did:key as its issuer"
            );
        }

        // `auto_resolve_keys(true)` is just how a `VerifierJwt` is reached
        // with no decoding key of its own yet (the only other route,
        // `.public_key(key)`, needs a `Key` we don't have here); the
        // `KeyResolver::new()` it attaches is immediately replaced below, so
        // it never does any network resolution.
        let verifier = self.jwt_builder_impl(false).auto_resolve_keys(true).build()?;
        Ok(verifier.with_key_resolver(slim_auth::resolver::KeyResolver::with_self_certified_did()))
    }
}

// Using the JWT middleware from jwt_middleware.rs

impl ClientAuthenticator for Config {
    // Associated types
    type ClientLayer = AddJwtLayer<SignerJwt>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, ConfigAuthError> {
        let signer = self.get_provider()?;
        Ok(AddJwtLayer::new(signer))
    }
}

impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    // Associated types
    type ServerLayer = Stack<PolicyCheckLayer, ValidateJwtLayer<MetadataMap, VerifierJwt>>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, ConfigAuthError> {
        let verifier = self.get_verifier()?;
        let jwt_layer = ValidateJwtLayer::new(verifier, self.custom_claims());
        let policy_layer = match &self.policy {
            None => PolicyCheckLayer::none(),
            Some(PolicyConfig::Rego(text)) => PolicyCheckLayer::rego(text)?,
            Some(PolicyConfig::RegoFile(path)) => PolicyCheckLayer::rego_file(path)?,
            Some(PolicyConfig::Cel(expr)) => PolicyCheckLayer::cel(expr)?,
        };
        Ok(Stack::new(policy_layer, jwt_layer))
    }
}

// tests
#[cfg(test)]
mod tests {
    use crate::testutils::tower_service::{Body, HeaderCheckService};
    use crate::tls::provider::initialize_crypto_provider;
    use http::{Response, StatusCode};
    use serde_json;
    use slim_auth::jwt::Algorithm;
    use slim_auth::jwt::KeyData;
    use slim_auth::jwt::KeyFormat;
    use slim_auth::metadata::MetadataMap;
    use slim_auth::traits::Signer;
    use tower::{Service, ServiceBuilder};

    use super::*;

    fn hs256_encoding_key() -> JwtKey {
        JwtKey::Encoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("policy-test-secret".to_string()),
        })
    }

    fn hs256_decoding_key() -> JwtKey {
        JwtKey::Decoding(Key {
            algorithm: Algorithm::HS256,
            format: KeyFormat::Pem,
            key: KeyData::Data("policy-test-secret".to_string()),
        })
    }

    fn token_with_groups(groups: Vec<&str>) -> String {
        let mut custom = MetadataMap::new();
        custom.insert("groups", groups);
        let cfg = Config::new(
            Claims::new(None, None, None, Some(custom)),
            Duration::from_secs(3600),
            hs256_encoding_key(),
        );
        cfg.get_provider().unwrap().sign_standard_claims().unwrap()
    }

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
    fn subject_binding_is_read_from_config_and_absent_by_default() {
        let json = r#"{ "key": { "type": "autoresolve" }, "subject_binding": "enforce" }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.subject_binding(), Some(IdentityBinding::Enforce));

        let json = r#"{ "key": { "type": "autoresolve" } }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.subject_binding(), None);
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
    fn test_server_layer_with_cel_policy() {
        let cfg = Config::new(
            Claims::default(),
            Duration::from_secs(60),
            JwtKey::Decoding(Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("verification-key".to_string()),
            }),
        )
        .with_cel_policy("\"admin\" in claims.groups");
        assert!(
            <Config as super::ServerAuthenticator<http::Response<Body>>>::get_server_layer(&cfg)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_cel_policy_allows_member_of_group() {
        let token = token_with_groups(vec!["slim-node", "ops"]);
        let server_cfg = Config::new(
            Claims::default(),
            Duration::from_secs(3600),
            hs256_decoding_key(),
        )
        .with_cel_policy("\"slim-node\" in claims.groups");
        let layer =
            <Config as ServerAuthenticator<Response<Body>>>::get_server_layer(&server_cfg).unwrap();
        let mut svc = ServiceBuilder::new()
            .layer(layer)
            .service(HeaderCheckService);

        let req = http::Request::builder()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .body(vec![])
            .unwrap();
        assert_eq!(svc.call(req).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cel_policy_rejects_non_member() {
        let token = token_with_groups(vec!["other-group"]);
        let server_cfg = Config::new(
            Claims::default(),
            Duration::from_secs(3600),
            hs256_decoding_key(),
        )
        .with_cel_policy("\"slim-node\" in claims.groups");
        let layer =
            <Config as ServerAuthenticator<Response<Body>>>::get_server_layer(&server_cfg).unwrap();
        let mut svc = ServiceBuilder::new()
            .layer(layer)
            .service(HeaderCheckService);

        let req = http::Request::builder()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .body(vec![])
            .unwrap();
        assert_eq!(svc.call(req).await.unwrap().status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_rego_policy_allows_member_of_group() {
        let token = token_with_groups(vec!["slim-node", "ops"]);
        let server_cfg =
            Config::new(Claims::default(), Duration::from_secs(3600), hs256_decoding_key())
                .with_rego_policy(
                    "package slim.auth\ndefault allow = false\nallow if \"slim-node\" in input.claims.groups",
                );
        let layer =
            <Config as ServerAuthenticator<Response<Body>>>::get_server_layer(&server_cfg).unwrap();
        let mut svc = ServiceBuilder::new()
            .layer(layer)
            .service(HeaderCheckService);

        let req = http::Request::builder()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .body(vec![])
            .unwrap();
        assert_eq!(svc.call(req).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_rego_policy_rejects_non_member() {
        let token = token_with_groups(vec!["other-group"]);
        let server_cfg =
            Config::new(Claims::default(), Duration::from_secs(3600), hs256_decoding_key())
                .with_rego_policy(
                    "package slim.auth\ndefault allow = false\nallow if \"slim-node\" in input.claims.groups",
                );
        let layer =
            <Config as ServerAuthenticator<Response<Body>>>::get_server_layer(&server_cfg).unwrap();
        let mut svc = ServiceBuilder::new()
            .layer(layer)
            .service(HeaderCheckService);

        let req = http::Request::builder()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .body(vec![])
            .unwrap();
        assert_eq!(svc.call(req).await.unwrap().status(), StatusCode::FORBIDDEN);
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

    #[cfg(feature = "insecure-self-certified-did")]
    #[test]
    fn test_get_self_certified_did_verifier_ignores_key() {
        initialize_crypto_provider();
        // `key` is mandatory on `Config::new` but irrelevant here -- any
        // placeholder works, including one that would fail `get_verifier()`.
        let cfg = Config::new(Claims::default(), Duration::from_secs(60), hs256_encoding_key());
        let verifier = cfg.get_self_certified_did_verifier();
        assert!(
            verifier.is_ok(),
            "expected a self-certified-did verifier regardless of key: {:?}",
            verifier.err()
        );
    }

    #[cfg(feature = "insecure-self-certified-did")]
    #[tokio::test]
    async fn test_get_self_certified_did_verifier_ignores_configured_issuer() {
        use slim_auth::traits::{Signer, Verifier};

        initialize_crypto_provider();

        // RFC 8032 section 7.1, test 1 -- a fixed Ed25519 key pair, PKCS#8-wrapped.
        let seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        let seed_bytes: Vec<u8> = (0..seed.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&seed[i..i + 2], 16).unwrap())
            .collect();
        let mut der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        der.extend_from_slice(&seed_bytes);
        let pem = slim_auth::utils::bytes_to_pem(
            &der,
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----\n",
        );
        let did = slim_auth::did_key::ed25519_did_key_from_pkcs8_pem(&pem).unwrap();
        let public_key = slim_auth::did_key::ed25519_public_key_from_pkcs8_pem(&pem).unwrap();
        let pubkey_claim = slim_auth::identity_claims::IdentityClaims::from_public_key_bytes(&public_key);

        let signer_cfg = Config::new(
            Claims::new(
                None,
                Some(did.clone()),
                Some(did.clone()),
                Some(pubkey_claim),
            ),
            Duration::from_secs(60),
            JwtKey::Encoding(Key {
                algorithm: Algorithm::EdDSA,
                format: KeyFormat::Pem,
                key: KeyData::Data(pem),
            }),
        );
        let token = signer_cfg
            .get_provider()
            .unwrap()
            .sign_standard_claims()
            .unwrap();

        // `issuer` is set to a value that is NOT the signer's did:key --
        // self-certified-did mode must not reject the token on that basis.
        let verifier_cfg = Config::new(
            Claims::default().with_issuer("some-other-issuer"),
            Duration::from_secs(60),
            JwtKey::Autoresolve,
        );
        let verifier = verifier_cfg.get_self_certified_did_verifier().unwrap();
        let claims: serde_json::Value = verifier.get_claims(token).await.unwrap();
        assert_eq!(claims["sub"], did);
    }
}

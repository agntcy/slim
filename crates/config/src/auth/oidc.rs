// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::errors::AuthError;
use slim_auth::jwt_middleware::{AddJwtLayer, PolicyCheckLayer, ValidateJwtLayer};
use slim_auth::metadata::MetadataMap;
use slim_auth::refresh_token::{RefreshTokenProvider, RefreshTokenProviderConfig};
use slim_auth::traits::TokenProvider;
use tower_layer::Stack;

use super::{ClientAuthenticator, ConfigAuthError, ServerAuthenticator};
use slim_auth::oidc::{OidcProviderConfig, OidcTokenProvider, OidcVerifier};

pub use super::PolicyConfig;

/// Unified OIDC Configuration that can act as both provider and verifier
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Config {
    /// OIDC issuer URL (e.g., https://auth.example.com)
    pub issuer_url: String,

    /// OAuth2 client ID (required for provider functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// OAuth2 client secret (required for provider functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,

    /// Expected audience for JWT tokens (required for verifier functionality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,

    /// Refresh token obtained from `slimctl login` (provider only).
    /// When set, uses the OAuth2 refresh-token grant instead of client credentials.
    /// Only one of `client_secret` or `refresh_token` may be set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Optional scope parameter for the token request (provider only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// HTTP timeout for token requests (default: 30s, provider only)
    #[serde(default = "default_timeout")]
    #[schemars(with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<DurationString>,

    /// JWKS cache TTL (default: 1 hour, verifier only)
    #[serde(default = "default_jwks_ttl")]
    #[schemars(with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwks_ttl: Option<DurationString>,

    /// Cache TTL for merged JWT+userinfo claims (verifier only).
    /// When set, claims are cached per token for this duration (e.g. "5m", "300s").
    /// Absent means no caching — userinfo is fetched on every request.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub claim_cache_ttl: Option<DurationString>,

    /// Rego policy evaluated against JWT claims on every request.
    /// Input shape: `{ "claims": { <all JWT payload fields> } }`.
    /// Must define `package slim.auth` with `default allow = false`.
    /// Absent means all authenticated requests are allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyConfig>,
}

fn default_timeout() -> Option<DurationString> {
    Some(Duration::from_secs(30).into())
}

fn default_jwks_ttl() -> Option<DurationString> {
    Some(Duration::from_secs(3600).into()) // 1 hour
}

impl Config {
    /// Create a new OIDC configuration with the issuer URL
    pub fn new(issuer_url: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: None,
            client_secret: None,
            refresh_token: None,
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Create a provider-only configuration (client credentials flow)
    pub fn provider(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        issuer_url: impl Into<String>,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            refresh_token: None,
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Create a provider configuration using a refresh token (authorization-code flow).
    pub fn with_refresh_token(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: Some(client_id.into()),
            client_secret: None,
            refresh_token: Some(refresh_token.into()),
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Create a verifier-only configuration
    pub fn verifier(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: None,
            client_secret: None,
            refresh_token: None,
            audience: Some(audience.into()),
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Create a combined configuration that can work as both provider and verifier
    pub fn combined(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            refresh_token: None,
            audience: Some(audience.into()),
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Set the client credentials for provider functionality
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Set the audience for verifier functionality
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Set the scope for the OIDC token request (provider functionality)
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Set the HTTP timeout for token requests (provider functionality)
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout.into());
        self
    }

    /// Set the JWKS cache TTL (verifier functionality)
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_ttl = Some(ttl.into());
        self
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

    /// Enable claim caching with the given TTL (verifier only)
    pub fn with_claim_cache_ttl(mut self, ttl: DurationString) -> Self {
        self.claim_cache_ttl = Some(ttl);
        self
    }

    /// Check if this configuration can act as a provider
    pub fn can_provide(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }

    /// Check if this configuration can act as a verifier
    pub fn can_verify(&self) -> bool {
        self.audience.is_some()
    }

    /// Convert to the auth crate's OidcProviderConfig
    fn to_auth_config(&self) -> Result<OidcProviderConfig, ConfigAuthError> {
        let client_id = self
            .client_id
            .as_ref()
            .ok_or(ConfigAuthError::AuthOidcEmptyClientId)?;
        let client_secret = self
            .client_secret
            .as_ref()
            .ok_or(ConfigAuthError::AuthOidcEmptyClientSecret)?;

        Ok(OidcProviderConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            issuer_url: self.issuer_url.clone(),
            scope: self.scope.clone(),
            timeout: self.timeout.map(|d| d.into()),
        })
    }

    /// Create an OIDC token provider from this configuration
    fn create_provider(&self) -> Result<OidcTokenProvider, ConfigAuthError> {
        let config = self.to_auth_config()?;
        let provider = OidcTokenProvider::new(config)?;
        Ok(provider)
    }

    /// Create an OIDC verifier from this configuration
    fn create_verifier(&self) -> Result<OidcVerifier, ConfigAuthError> {
        let audience = self
            .audience
            .as_ref()
            .ok_or(ConfigAuthError::AuthJwtAudienceRequired)?;

        let mut verifier = OidcVerifier::new(&self.issuer_url, audience);
        if let Some(ttl) = self.jwks_ttl {
            verifier = verifier.with_jwks_ttl(ttl.into());
        }
        if let Some(ttl) = &self.claim_cache_ttl {
            verifier = verifier.with_claim_cache(Duration::from(*ttl));
        }
        Ok(verifier)
    }
}

/// Wraps either OIDC flow behind a single `TokenProvider + Clone` so
/// `ClientAuthenticator` can return a concrete `AddJwtLayer` regardless of
/// which grant type is configured.
#[derive(Clone)]
pub enum OidcClientProvider {
    ClientCredentials(OidcTokenProvider),
    RefreshToken(RefreshTokenProvider),
}

impl TokenProvider for OidcClientProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        match self {
            OidcClientProvider::ClientCredentials(p) => p.initialize().await,
            OidcClientProvider::RefreshToken(p) => p.initialize().await,
        }
    }

    fn get_token(&self) -> Result<String, AuthError> {
        match self {
            OidcClientProvider::ClientCredentials(p) => p.get_token(),
            OidcClientProvider::RefreshToken(p) => p.get_token(),
        }
    }

    fn get_id(&self) -> Result<String, AuthError> {
        match self {
            OidcClientProvider::ClientCredentials(p) => p.get_id(),
            OidcClientProvider::RefreshToken(p) => p.get_id(),
        }
    }

    async fn set_signature_keys(
        &mut self,
        _private_key: Vec<u8>,
        _public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        Err(AuthError::MlsNotSupported)
    }
}

/// Round-trippable view of `~/.slimctl/credentials.yaml`.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct StoredCredentials {
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    client_id: String,
    issuer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    token_endpoint: String,
}

fn credentials_file_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| {
        std::path::PathBuf::from(h)
            .join(".slimctl")
            .join("credentials.yaml")
    })
}

fn load_stored_credentials() -> Option<StoredCredentials> {
    let path = credentials_file_path()?;
    let data = std::fs::read_to_string(&path).ok()?;
    serde_yaml::from_str(&data).ok()
}

fn persist_stored_credentials(path: std::path::PathBuf, creds: StoredCredentials) {
    match serde_yaml::to_string(&creds) {
        Ok(yaml) => {
            if let Err(e) = std::fs::write(&path, &yaml) {
                tracing::error!("failed to write rotated credentials to {path:?}: {e}");
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            tracing::debug!("persisted rotated refresh token to {path:?}");
        }
        Err(e) => tracing::error!("failed to serialize credentials: {e}"),
    }
}

// Implement ClientAuthenticator for Config
impl ClientAuthenticator for Config {
    type ClientLayer = AddJwtLayer<OidcClientProvider>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, ConfigAuthError> {
        let provider = if let Some(rt) = &self.refresh_token {
            // Explicit refresh token (programmatic use, e.g. slimctl).
            let client_id = self
                .client_id
                .as_ref()
                .ok_or(ConfigAuthError::AuthOidcEmptyClientId)?;
            OidcClientProvider::RefreshToken(RefreshTokenProvider::new(
                RefreshTokenProviderConfig {
                    refresh_token: rt.clone(),
                    issuer_url: self.issuer_url.clone(),
                    client_id: client_id.clone(),
                    timeout: self.timeout,
                    initial_access_token: None,
                    persist_credentials: None,
                },
            )?)
        } else if self.client_secret.is_some() {
            // Client-credentials flow.
            OidcClientProvider::ClientCredentials(self.create_provider()?)
        } else {
            // No explicit tokens — load from ~/.slimctl/credentials.yaml.
            let creds =
                load_stored_credentials().ok_or(ConfigAuthError::IdentityProviderNotConfigured)?;
            if !self.issuer_url.is_empty() && creds.issuer != self.issuer_url {
                return Err(ConfigAuthError::IdentityProviderNotConfigured);
            }
            let refresh_token = creds
                .refresh_token
                .clone()
                .ok_or(ConfigAuthError::IdentityProviderNotConfigured)?;
            let persist = credentials_file_path().map(|path| {
                let creds = creds.clone();
                let cb: Arc<dyn Fn(String, String) + Send + Sync> =
                    Arc::new(move |access_token, new_refresh_token| {
                        let mut updated = creds.clone();
                        updated.access_token = Some(access_token);
                        updated.refresh_token = Some(new_refresh_token);
                        persist_stored_credentials(path.clone(), updated);
                    });
                cb
            });
            OidcClientProvider::RefreshToken(RefreshTokenProvider::new(
                RefreshTokenProviderConfig {
                    refresh_token,
                    issuer_url: creds.issuer,
                    client_id: self.client_id.clone().unwrap_or(creds.client_id),
                    timeout: self.timeout,
                    initial_access_token: creds.access_token,
                    persist_credentials: persist,
                },
            )?)
        };

        Ok(Self::ClientLayer::new(provider))
    }
}

// Implement ServerAuthenticator for Config
impl<Response> ServerAuthenticator<Response> for Config
where
    Response: Default + Send + 'static,
{
    type ServerLayer = Stack<PolicyCheckLayer, ValidateJwtLayer<MetadataMap, OidcVerifier>>;

    fn get_server_layer(&self) -> Result<Self::ServerLayer, ConfigAuthError> {
        if !self.can_verify() {
            return Err(ConfigAuthError::AuthJwtAudienceRequired);
        }

        let verifier = self.create_verifier()?;
        let jwt_layer = ValidateJwtLayer::new(verifier, MetadataMap::default());

        let policy_layer = match &self.policy {
            None => PolicyCheckLayer::none(),
            Some(PolicyConfig::Rego(text)) => PolicyCheckLayer::rego(text)?,
            Some(PolicyConfig::RegoFile(path)) => PolicyCheckLayer::rego_file(path)?,
            Some(PolicyConfig::Cel(expr)) => PolicyCheckLayer::cel(expr)?,
        };

        Ok(Stack::new(policy_layer, jwt_layer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::{self, Ready};
    use http::{Request, Response, StatusCode};
    use serde_json;
    use slim_auth::jwt_middleware::PolicyCheckLayer;
    use slim_auth::metadata::MetadataMap;
    use std::task::{Context, Poll};
    use tower::{Service, ServiceBuilder};

    type Body = Vec<u8>;

    #[derive(Clone)]
    struct OkService;
    impl Service<Request<Body>> for OkService {
        type Response = Response<Body>;
        type Error = std::convert::Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: Request<Body>) -> Self::Future {
            future::ready(Ok(Response::builder()
                .status(StatusCode::OK)
                .body(vec![])
                .unwrap()))
        }
    }

    fn policy_layer_from(config: &PolicyConfig) -> PolicyCheckLayer {
        match config {
            PolicyConfig::Rego(text) => PolicyCheckLayer::rego(text).unwrap(),
            PolicyConfig::RegoFile(path) => PolicyCheckLayer::rego_file(path).unwrap(),
            PolicyConfig::Cel(expr) => PolicyCheckLayer::cel(expr).unwrap(),
        }
    }

    fn request_with_groups(groups: Vec<&str>) -> Request<Body> {
        let mut claims = MetadataMap::new();
        claims.insert("groups", groups);
        let mut req = Request::builder().body(vec![]).unwrap();
        req.extensions_mut().insert(claims);
        req
    }

    #[test]
    fn test_provider_config_creation() {
        let config = Config::provider(
            "test-client-id",
            "test-client-secret",
            "https://auth.example.com",
        )
        .with_scope("api:read")
        .with_timeout(Duration::from_secs(45));

        assert_eq!(config.client_id, Some("test-client-id".to_string()));
        assert_eq!(config.client_secret, Some("test-client-secret".to_string()));
        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.timeout, Some(Duration::from_secs(45).into()));
        assert!(config.can_provide());
        assert!(!config.can_verify());
    }

    #[test]
    fn test_verifier_config_creation() {
        let config = Config::verifier("https://auth.example.com", "test-audience")
            .with_jwks_ttl(Duration::from_secs(1800));

        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.audience, Some("test-audience".to_string()));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800).into()));
        assert!(!config.can_provide());
        assert!(config.can_verify());
    }

    #[test]
    fn test_combined_config_creation() {
        let config = Config::combined(
            "https://auth.example.com",
            "client-id",
            "client-secret",
            "audience",
        )
        .with_scope("api:read")
        .with_jwks_ttl(Duration::from_secs(1800));

        assert_eq!(config.issuer_url, "https://auth.example.com");
        assert_eq!(config.client_id, Some("client-id".to_string()));
        assert_eq!(config.client_secret, Some("client-secret".to_string()));
        assert_eq!(config.audience, Some("audience".to_string()));
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800).into()));
        assert!(config.can_provide());
        assert!(config.can_verify());
    }

    #[test]
    fn test_config_builder_pattern() {
        let config = Config::new("https://auth.example.com")
            .with_client_credentials("client-id", "client-secret")
            .with_audience("test-audience")
            .with_scope("api:read")
            .with_timeout(Duration::from_secs(45))
            .with_jwks_ttl(Duration::from_secs(1800));

        assert!(config.can_provide());
        assert!(config.can_verify());
        assert_eq!(config.scope, Some("api:read".to_string()));
        assert_eq!(config.timeout, Some(Duration::from_secs(45).into()));
        assert_eq!(config.jwks_ttl, Some(Duration::from_secs(1800).into()));
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::combined(
            "https://auth.example.com",
            "client-id",
            "client-secret",
            "audience",
        )
        .with_scope("api:read");

        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: Config = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_policy_config_serde_rego() {
        let policy = PolicyConfig::Rego("package slim.auth\ndefault allow = false".to_string());
        let json = serde_json::to_string(&policy).expect("serialize");
        assert!(json.contains("\"rego\""));
        let back: PolicyConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(policy, back);
    }

    #[test]
    fn test_policy_config_serde_rego_file() {
        let policy = PolicyConfig::RegoFile(PathBuf::from("/etc/slim/auth.rego"));
        let json = serde_json::to_string(&policy).expect("serialize");
        assert!(json.contains("\"rego_file\""));
        let back: PolicyConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(policy, back);
    }

    #[test]
    fn test_policy_config_serde_cel() {
        let policy = PolicyConfig::Cel("\"admin\" in claims.groups".to_string());
        let json = serde_json::to_string(&policy).expect("serialize");
        assert!(json.contains("\"cel\""));
        let back: PolicyConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(policy, back);
    }

    #[test]
    fn test_client_layer_creation_fails_without_credentials() {
        let config = Config::verifier("https://auth.example.com", "test-audience");

        let layer = config.get_client_layer();
        assert!(
            layer.is_err(),
            "Should fail to create client layer without credentials"
        );
    }

    #[test]
    fn test_can_provide_and_verify_methods() {
        let provider_only = Config::provider("id", "secret", "https://auth.example.com");
        assert!(provider_only.can_provide());
        assert!(!provider_only.can_verify());

        let verifier_only = Config::verifier("https://auth.example.com", "audience");
        assert!(!verifier_only.can_provide());
        assert!(verifier_only.can_verify());

        let combined = Config::combined("https://auth.example.com", "id", "secret", "audience");
        assert!(combined.can_provide());
        assert!(combined.can_verify());
    }

    #[test]
    fn test_get_server_layer_builds_with_cel_policy() {
        let config = Config::verifier("https://auth.example.com", "audience")
            .with_cel_policy("\"slim-node\" in claims.groups");
        assert!(
            <Config as super::ServerAuthenticator<Response<Body>>>::get_server_layer(&config)
                .is_ok()
        );
    }

    #[test]
    fn test_get_server_layer_builds_with_rego_policy() {
        let config = Config::verifier("https://auth.example.com", "audience").with_rego_policy(
            "package slim.auth\ndefault allow = false\nallow if \"slim-node\" in input.claims.groups",
        );
        assert!(
            <Config as super::ServerAuthenticator<Response<Body>>>::get_server_layer(&config)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_cel_policy_allows_member_of_group() {
        let policy = PolicyConfig::Cel("\"slim-node\" in claims.groups".to_string());
        let mut svc = ServiceBuilder::new()
            .layer(policy_layer_from(&policy))
            .service(OkService);

        let resp = svc
            .call(request_with_groups(vec!["slim-node", "ops"]))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cel_policy_rejects_non_member() {
        let policy = PolicyConfig::Cel("\"slim-node\" in claims.groups".to_string());
        let mut svc = ServiceBuilder::new()
            .layer(policy_layer_from(&policy))
            .service(OkService);

        let resp = svc
            .call(request_with_groups(vec!["other-group"]))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_rego_policy_allows_member_of_group() {
        let policy = PolicyConfig::Rego(
            "package slim.auth\ndefault allow = false\nallow if \"slim-node\" in input.claims.groups"
                .to_string(),
        );
        let mut svc = ServiceBuilder::new()
            .layer(policy_layer_from(&policy))
            .service(OkService);

        let resp = svc
            .call(request_with_groups(vec!["slim-node", "ops"]))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_rego_policy_rejects_non_member() {
        let policy = PolicyConfig::Rego(
            "package slim.auth\ndefault allow = false\nallow if \"slim-node\" in input.claims.groups"
                .to_string(),
        );
        let mut svc = ServiceBuilder::new()
            .layer(policy_layer_from(&policy))
            .service(OkService);

        let resp = svc
            .call(request_with_groups(vec!["other-group"]))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}

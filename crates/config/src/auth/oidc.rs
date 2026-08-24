// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slim_auth::errors::AuthError;
use slim_auth::jwt_middleware::{AddJwtLayer, PolicyCheckLayer, ValidateJwtLayer};
use slim_auth::metadata::MetadataMap;
use slim_auth::refresh_token::{LockAndReloadFn, RefreshTokenProvider, RefreshTokenProviderConfig};
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
    ///
    /// Prefer `refresh_token_file` for anything long-lived: an inline token (or one
    /// substituted in via `${file:...}`) cannot be written back, so a rotating IdP
    /// invalidates it after the first exchange.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Path to a file holding the refresh token (provider only).
    ///
    /// Unlike `refresh_token`, this is a *read-write* binding: the token is read
    /// from the file, and when the IdP rotates it the new value is written back
    /// to the same path (mode 0600), so the next start resumes the chain instead
    /// of replaying a token the IdP already invalidated.  Required for IdPs that
    /// issue single-use refresh tokens.
    ///
    /// A refresh token serves **one process at a time**: whoever exchanges it
    /// invalidates the copy everyone else holds.  Nothing here enforces that, so
    /// point each long-lived consumer at its own file.
    ///
    /// Takes precedence over `refresh_token` when both are set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token_file: Option<String>,

    /// Path to a file caching the current access token (provider only).
    ///
    /// Optional companion to `refresh_token_file`.  A still-valid token found here
    /// seeds the provider's cache, sparing short-lived processes a token-endpoint
    /// round-trip — and a refresh-token rotation — on every invocation.  Refreshed
    /// tokens are written back (mode 0600).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token_file: Option<String>,

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
            refresh_token_file: None,
            access_token_file: None,
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
            refresh_token_file: None,
            access_token_file: None,
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
            refresh_token_file: None,
            access_token_file: None,
            audience: None,
            scope: None,
            timeout: default_timeout(),
            jwks_ttl: default_jwks_ttl(),
            claim_cache_ttl: None,
            policy: None,
        }
    }

    /// Create a provider configuration backed by a refresh token *file*, so that
    /// rotated tokens survive process restarts.  `access_token_file` is an
    /// optional cache that lets short-lived processes skip a token exchange.
    pub fn with_token_files(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        refresh_token_file: impl Into<String>,
        access_token_file: Option<String>,
    ) -> Self {
        Self {
            client_id: Some(client_id.into()),
            refresh_token_file: Some(refresh_token_file.into()),
            access_token_file,
            ..Self::new(issuer_url)
        }
    }

    /// Create a verifier-only configuration
    pub fn verifier(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: None,
            client_secret: None,
            refresh_token: None,
            refresh_token_file: None,
            access_token_file: None,
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
            refresh_token_file: None,
            access_token_file: None,
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
    ///
    /// `client_secret` is optional: authorization code + PKCE clients are public.
    /// The transport path gates on it in `get_client_layer`; the identity path is
    /// checked in [`create_identity_provider`](Self::create_identity_provider).
    fn to_auth_config(&self) -> Result<OidcProviderConfig, ConfigAuthError> {
        let client_id = self
            .client_id
            .as_ref()
            .ok_or(ConfigAuthError::AuthOidcEmptyClientId)?;

        Ok(OidcProviderConfig {
            client_id: client_id.clone(),
            client_secret: self.client_secret.clone().unwrap_or_default(),
            issuer_url: self.issuer_url.clone(),
            scope: self.scope.clone(),
            timeout: self.timeout.map(|d| d.into()),
        })
    }

    /// Create an OIDC token provider.
    ///
    /// Does *not* adopt a stored login — this is also the transport-auth path,
    /// where a daemon would otherwise authenticate as whoever last ran
    /// `slimctl login` and load their MLS private key. Use
    /// [`create_identity_provider`](Self::create_identity_provider) for identity.
    pub fn create_provider(&self) -> Result<OidcTokenProvider, ConfigAuthError> {
        let config = self.to_auth_config()?;
        let provider = OidcTokenProvider::new(config)?;
        Ok(provider)
    }

    /// Provider for the application-identity path, adopting the MLS key and
    /// refresh token `slimctl login --dpop` bound for this issuer.
    pub fn create_identity_provider(&self) -> Result<OidcTokenProvider, ConfigAuthError> {
        let provider = self.create_provider()?;

        if self.should_adopt_stored_login() {
            self.seed_from_login(&provider);
        }

        // A working renewal path is either a client_secret (service identity) or
        // an adopted refresh-token delegate (user identity). An installed MLS
        // key alone isn't enough — a login whose IdP granted no `offline_access`
        // stores a key but no refresh token, which would otherwise build fine
        // and only fail at the first token fetch, behind a swallowed warning.
        if self.client_secret.as_deref().unwrap_or_default().is_empty()
            && !provider.has_refresh_delegate()
        {
            tracing::error!(
                issuer = %self.issuer_url,
                "OIDC identity has no client secret and no usable stored login for this \
                 issuer (its refresh token may be missing — check the IdP granted \
                 `offline_access`); run `slimctl login --dpop`, or set client_secret for a \
                 service identity"
            );
            return Err(ConfigAuthError::IdentityProviderNotConfigured);
        }

        Ok(provider)
    }

    /// A configured `client_secret` means this is a service identity: never let a
    /// stray personal `slimctl login` for the same issuer on the same host
    /// silently override it with someone's own MLS key and refresh token.
    fn should_adopt_stored_login(&self) -> bool {
        self.client_secret.as_deref().unwrap_or_default().is_empty()
    }

    /// Adopt what `slimctl login` left for this issuer: the MLS key the token was
    /// bound to, and the refresh token that renews it. Without the key the app
    /// signs as an identity the IdP never attested; without the refresh token it
    /// renews as the service. Silent when absent.
    fn seed_from_login(&self, provider: &OidcTokenProvider) {
        let Some(creds) = load_stored_credentials() else {
            return;
        };
        self.apply_stored_credentials(&creds, provider);
    }

    /// Split from reading the file so it is testable without `$HOME`.
    fn apply_stored_credentials(&self, creds: &StoredCredentials, provider: &OidcTokenProvider) {
        // Credentials for another issuer say nothing about this one. Normalize the
        // trailing slash, or the same issuer written two ways fails to match.
        let configured = self.issuer_url.trim_end_matches('/');
        if !configured.is_empty() && creds.issuer.trim_end_matches('/') != configured {
            // `warn`: a login exists but is being discarded.
            tracing::warn!(
                stored = %creds.issuer, configured = %self.issuer_url,
                "ignoring stored credentials issued by a different issuer"
            );
            return;
        }

        if let (Some(private_key), Some(public_key)) =
            (&creds.mls_private_key, &creds.mls_public_key)
        {
            match (BASE64.decode(private_key), BASE64.decode(public_key)) {
                (Ok(private_key), Ok(public_key)) => {
                    if let Err(e) = provider.install_signature_keys(private_key, public_key) {
                        tracing::error!(error = %e, "stored MLS key is unusable for DPoP");
                    }
                }
                _ => tracing::error!("stored MLS key is not valid base64; ignoring"),
            }
        }

        if let Some(refresh_token) = &creds.refresh_token {
            // Write rotations back, or a restart replays an invalidated token.
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

            if let Err(e) = provider.adopt_refresh_token(
                refresh_token.clone(),
                creds.access_token.clone(),
                persist,
            ) {
                tracing::error!(error = %e, "could not adopt the stored refresh token");
            }
        }
    }

    /// Create an OIDC verifier from this configuration
    pub fn create_verifier(&self) -> Result<OidcVerifier, ConfigAuthError> {
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
    /// MLS signature key pair (standard base64) written by `slimctl login --dpop`.
    /// Present only for DPoP-bound logins; absent for a plain bearer login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mls_private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mls_public_key: Option<String>,
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

/// Write a bare secret to `path`, owner-readable only.
///
/// Truncates in place rather than writing-then-renaming: these files are read by
/// the same user on the next invocation, and a rename would drop the 0600 mode
/// if the destination were pre-created differently.
fn write_secret_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(contents.as_bytes())
}

/// Read a secret written by [`write_secret_file`], trimming the trailing newline
/// a user's editor may have added.
fn read_secret_file(path: &str) -> Result<String, ConfigAuthError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        tracing::error!("failed to read token file {path}: {e}");
        ConfigAuthError::IdentityProviderNotConfigured
    })?;
    let token = raw.trim();
    if token.is_empty() {
        tracing::error!("token file {path} is empty");
        return Err(ConfigAuthError::IdentityProviderNotConfigured);
    }
    Ok(token.to_owned())
}

// Implement ClientAuthenticator for Config
impl ClientAuthenticator for Config {
    type ClientLayer = AddJwtLayer<OidcClientProvider>;

    fn get_client_layer(&self) -> Result<Self::ClientLayer, ConfigAuthError> {
        let provider = if let Some(rt_file) = &self.refresh_token_file {
            // File-backed refresh token: read it, and write rotations straight
            // back so the next start resumes from the live token.
            let client_id = self
                .client_id
                .as_ref()
                .ok_or(ConfigAuthError::AuthOidcEmptyClientId)?;
            let refresh_token = read_secret_file(rt_file)?;

            // Only seed from a cached access token, never fail on one — a missing
            // or stale cache just costs one token exchange.
            let initial_access_token = self
                .access_token_file
                .as_deref()
                .and_then(|p| read_secret_file(p).ok());

            let rt_path = PathBuf::from(rt_file);
            let at_path = self.access_token_file.as_deref().map(PathBuf::from);

            // Wrapped in a block so rt_path and at_path can be cloned before they
            // are moved into the persist closure below.
            let persist: Arc<dyn Fn(String, String) + Send + Sync> = {
                let rt_path = rt_path.clone();
                let at_path = at_path.clone();
                Arc::new(move |access_token, new_refresh_token| {
                    if let Err(e) = write_secret_file(&rt_path, &new_refresh_token) {
                        tracing::error!(
                            "failed to write rotated refresh token to {rt_path:?}: {e}"
                        );
                    } else {
                        tracing::debug!("persisted rotated refresh token to {rt_path:?}");
                    }
                    if let Some(at_path) = &at_path
                        && let Err(e) = write_secret_file(at_path, &access_token)
                    {
                        tracing::error!("failed to write access token to {at_path:?}: {e}");
                    }
                })
            };

            let lock_and_reload: LockAndReloadFn = {
                // Companion lock file: a sibling path used only as a locking target.
                // Keeping it separate from the token file means write_secret_file can
                // open and truncate the token file freely without hitting a re-entrant
                // flock from the same process.
                let lock_path = {
                    let mut s = rt_path.clone().into_os_string();
                    s.push(".lock");
                    PathBuf::from(s)
                };
                let rt_path = rt_path.clone();
                Arc::new(move || {
                    use fs2::FileExt;
                    // Create the lock file if absent, then block until we hold an
                    // exclusive lock. The fd is the guard: closing it releases the lock.
                    let lock_file = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(&lock_path)
                        .ok()?;
                    lock_file.lock_exclusive().ok()?;
                    // Re-read both token files now that we hold the lock. Another
                    // process may have rotated them while we were waiting. The access
                    // token lets fetch_new_token skip the IdP call if it is still fresh.
                    // Note: the freshness optimization only activates when access_token_file
                    // is configured alongside refresh_token_file. Without it, access_token
                    // is None here and the full exchange always runs.
                    let token = read_secret_file(rt_path.to_str()?).ok()?;
                    let access_token = at_path
                        .as_ref()
                        .and_then(|p| read_secret_file(p.to_str()?).ok());
                    Some((token, access_token, Box::new(lock_file) as Box<dyn Send>))
                })
            };

            OidcClientProvider::RefreshToken(RefreshTokenProvider::new(
                RefreshTokenProviderConfig {
                    refresh_token,
                    issuer_url: self.issuer_url.clone(),
                    client_id: client_id.clone(),
                    timeout: self.timeout.map(Into::into),
                    initial_access_token,
                    persist_credentials: Some(persist),
                    lock_and_reload: Some(lock_and_reload),
                },
            )?)
        } else if let Some(rt) = &self.refresh_token {
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
                    timeout: self.timeout.map(Into::into),
                    initial_access_token: None,
                    persist_credentials: None,
                    lock_and_reload: None,
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
                    timeout: self.timeout.map(Into::into),
                    initial_access_token: creds.access_token,
                    persist_credentials: persist,
                    lock_and_reload: None,
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

    fn stored(issuer: &str, keys: Option<(&[u8], &[u8])>) -> StoredCredentials {
        StoredCredentials {
            id_token: "id".into(),
            access_token: None,
            refresh_token: Some("rt".into()),
            client_id: "slim-app".into(),
            issuer: issuer.into(),
            token_endpoint: String::new(),
            mls_private_key: keys.map(|(sk, _)| BASE64.encode(sk)),
            mls_public_key: keys.map(|(_, pk)| BASE64.encode(pk)),
        }
    }

    fn identity_provider(issuer: &str) -> OidcTokenProvider {
        OidcTokenProvider::new(OidcProviderConfig {
            client_id: "slim-app".into(),
            client_secret: String::new(),
            issuer_url: issuer.into(),
            scope: None,
            timeout: None,
        })
        .unwrap()
    }

    /// Otherwise the app signs as a key the IdP never attested.
    #[test]
    fn stored_mls_key_becomes_the_provider_identity() {
        let issuer = "https://idp.example.com";
        let (sk, pk) = slim_auth::utils::generate_mls_signature_keys().unwrap();
        let provider = identity_provider(issuer);
        assert!(!provider.mls_signature_keys_installed());

        Config::new(issuer).apply_stored_credentials(&stored(issuer, Some((&sk, &pk))), &provider);

        assert!(provider.mls_signature_keys_installed());
        assert_eq!(provider.get_signature_keys().unwrap(), (sk, pk));
    }

    /// Adopting these would sign with a key an unrelated IdP bound.
    #[test]
    fn stored_credentials_for_another_issuer_are_ignored() {
        let (sk, pk) = slim_auth::utils::generate_mls_signature_keys().unwrap();
        let provider = identity_provider("https://idp.example.com");

        Config::new("https://idp.example.com").apply_stored_credentials(
            &stored("https://other-idp.example.com", Some((&sk, &pk))),
            &provider,
        );

        assert!(!provider.mls_signature_keys_installed());
    }

    /// Treating these as different issuers silently discards a valid login.
    #[test]
    fn issuer_match_ignores_trailing_slash() {
        let (sk, pk) = slim_auth::utils::generate_mls_signature_keys().unwrap();
        let provider = identity_provider("https://idp.example.com");

        Config::new("https://idp.example.com/").apply_stored_credentials(
            &stored("https://idp.example.com", Some((&sk, &pk))),
            &provider,
        );

        assert!(provider.mls_signature_keys_installed());
    }

    /// A plain login carries no keys; seeding must be a no-op, not an error.
    #[test]
    fn stored_credentials_without_mls_keys_seed_nothing() {
        let issuer = "https://idp.example.com";
        let provider = identity_provider(issuer);
        Config::new(issuer).apply_stored_credentials(&stored(issuer, None), &provider);
        assert!(!provider.mls_signature_keys_installed());
    }

    /// The transport path must not adopt a human login, or a daemon would
    /// authenticate as whoever last ran `slimctl login`.
    #[test]
    fn only_the_identity_builder_seeds_a_stored_login() {
        let issuer = "https://idp.example.com";
        let service = Config::provider("svc", "secret", issuer)
            .create_provider()
            .unwrap();
        // Nothing was seeded regardless of what is on disk.
        assert!(!service.mls_signature_keys_installed());
    }

    /// A service identity must never be overridden by a stray personal login
    /// for the same issuer sitting in the same host's credentials file.
    #[test]
    fn service_identity_with_client_secret_does_not_adopt_stored_login() {
        let cfg = Config::provider("svc", "secret", "https://idp.example.com");
        assert!(!cfg.should_adopt_stored_login());
    }

    #[test]
    fn public_client_without_secret_adopts_stored_login() {
        let cfg = Config::new("https://idp.example.com").with_client_credentials("slim-app", "");
        assert!(cfg.should_adopt_stored_login());
    }

    /// Corrupt key material must not install a half-usable identity.
    #[test]
    fn corrupt_stored_mls_key_is_rejected() {
        let issuer = "https://idp.example.com";
        let provider = identity_provider(issuer);
        let mut creds = stored(issuer, None);
        creds.mls_private_key = Some("!!!not base64!!!".into());
        creds.mls_public_key = Some("!!!not base64!!!".into());

        Config::new(issuer).apply_stored_credentials(&creds, &provider);
        assert!(!provider.mls_signature_keys_installed());
    }

    /// A login whose IdP granted no `offline_access` stores an MLS key but no
    /// refresh token — signature keys install fine, but there is no renewal
    /// path. `create_identity_provider`'s construction-time guard must catch
    /// this via `has_refresh_delegate`, not treat installed keys alone as proof
    /// of a working identity.
    #[test]
    fn mls_key_without_a_refresh_token_leaves_no_renewal_path() {
        let issuer = "https://idp.example.com";
        let provider = identity_provider(issuer);
        let (sk, pk) = slim_auth::utils::generate_mls_signature_keys().unwrap();
        let mut creds = stored(issuer, Some((&sk, &pk)));
        creds.refresh_token = None;

        Config::new(issuer).apply_stored_credentials(&creds, &provider);

        assert!(provider.mls_signature_keys_installed());
        assert!(
            !provider.has_refresh_delegate(),
            "no refresh token means no renewal path, regardless of installed keys"
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

    // ── file-backed refresh token ───────────────────────────────────────────

    fn write_tmp(dir: &std::path::Path, name: &str, contents: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn read_secret_file_trims_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_tmp(dir.path(), "rt", "the-token\n");
        assert_eq!(read_secret_file(&path).unwrap(), "the-token");
    }

    #[test]
    fn read_secret_file_rejects_empty_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let empty = write_tmp(dir.path(), "empty", "   \n");
        assert!(read_secret_file(&empty).is_err());
        assert!(read_secret_file("/nonexistent/token").is_err());
    }

    #[test]
    fn write_secret_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_secret_file(&path, "s3cret").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "s3cret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
        }
    }

    /// Overwriting a longer secret must not leave a tail of the old one behind.
    #[test]
    fn write_secret_file_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_secret_file(&path, "a-very-long-refresh-token").unwrap();
        write_secret_file(&path, "short").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short");
    }

    #[test]
    fn with_token_files_sets_paths_and_no_inline_token() {
        let cfg = Config::with_token_files(
            "https://issuer.example.com",
            "myclient",
            "/tmp/rt",
            Some("/tmp/at".to_string()),
        );
        assert_eq!(cfg.refresh_token_file.as_deref(), Some("/tmp/rt"));
        assert_eq!(cfg.access_token_file.as_deref(), Some("/tmp/at"));
        assert!(cfg.refresh_token.is_none());
        assert!(cfg.client_secret.is_none());
        assert_eq!(cfg.client_id.as_deref(), Some("myclient"));
    }

    #[test]
    fn token_file_fields_round_trip_through_yaml() {
        let cfg = Config::with_token_files(
            "https://issuer.example.com",
            "myclient",
            "/tmp/rt",
            Some("/tmp/at".to_string()),
        );
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(yaml.contains("refresh_token_file: /tmp/rt"), "{yaml}");
        assert!(yaml.contains("access_token_file: /tmp/at"), "{yaml}");
        let back: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back, cfg);
    }

    /// Absent fields must stay absent, so existing configs are unaffected.
    #[test]
    fn token_file_fields_are_omitted_when_unset() {
        let yaml = serde_yaml::to_string(&Config::new("https://issuer.example.com")).unwrap();
        assert!(!yaml.contains("refresh_token_file"), "{yaml}");
        assert!(!yaml.contains("access_token_file"), "{yaml}");
    }

    #[test]
    fn refresh_token_file_requires_client_id() {
        crate::tls::provider::initialize_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        let rt = write_tmp(dir.path(), "rt", "the-refresh-token");
        let mut cfg = Config::new("https://issuer.example.com");
        cfg.refresh_token_file = Some(rt);
        assert!(matches!(
            cfg.get_client_layer(),
            Err(ConfigAuthError::AuthOidcEmptyClientId)
        ));
    }

    #[test]
    fn refresh_token_file_missing_is_an_error() {
        crate::tls::provider::initialize_crypto_provider();
        let cfg = Config::with_token_files(
            "https://issuer.example.com",
            "myclient",
            "/nonexistent/refresh_token",
            None,
        );
        assert!(cfg.get_client_layer().is_err());
    }

    /// A file-backed refresh token builds a provider; an unreadable access-token
    /// cache is only a cache, so it must not fail the build.
    #[test]
    fn refresh_token_file_builds_provider_despite_missing_access_token_cache() {
        crate::tls::provider::initialize_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        let rt = write_tmp(dir.path(), "rt", "the-refresh-token\n");
        let cfg = Config::with_token_files(
            "https://issuer.example.com",
            "myclient",
            rt,
            Some("/nonexistent/access_token".to_string()),
        );
        assert!(cfg.get_client_layer().is_ok());
    }

    /// `refresh_token_file` wins over an inline `refresh_token`, so a stale
    /// inline value can never shadow the live file.
    #[test]
    fn refresh_token_file_takes_precedence_over_inline() {
        crate::tls::provider::initialize_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        let rt = write_tmp(dir.path(), "rt", "from-file");
        let mut cfg =
            Config::with_token_files("https://issuer.example.com", "myclient", rt.clone(), None);
        cfg.refresh_token = Some("inline-and-stale".to_string());
        // Both are set; the file branch is the one that must run. It succeeds
        // because the file is readable — the inline branch would too, so assert
        // on the observable difference: deleting the file now fails the build.
        assert!(cfg.get_client_layer().is_ok());
        std::fs::remove_file(&rt).unwrap();
        assert!(
            cfg.get_client_layer().is_err(),
            "inline token shadowed the file"
        );
    }

    /// The same file is both source and sink, so a rotation persisted on one run
    /// is what the next run reads back.
    #[test]
    fn refresh_token_file_round_trips_a_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt");
        write_secret_file(&path, "seed-token").unwrap();
        assert_eq!(
            read_secret_file(path.to_str().unwrap()).unwrap(),
            "seed-token"
        );

        // What the persist callback does on rotation.
        write_secret_file(&path, "rotated-token").unwrap();
        assert_eq!(
            read_secret_file(path.to_str().unwrap()).unwrap(),
            "rotated-token"
        );
    }

    /// Two concurrent callers of the lock_and_reload closure must never read the
    /// same refresh token. The second caller blocks at flock until the first has
    /// written the rotated token to disk, then reads that new value.
    #[test]
    fn concurrent_lock_and_reload_closures_serialize_token_reads() {
        use fs2::FileExt;

        let dir = tempfile::tempdir().unwrap();
        let rt_path = dir.path().join("rt");
        let lock_path = {
            let mut s = rt_path.clone().into_os_string();
            s.push(".lock");
            PathBuf::from(s)
        };

        write_secret_file(&rt_path, "token-1").unwrap();

        // Replicates the lock_and_reload closure built in get_client_layer.
        let make_closure = |rt: PathBuf, lk: PathBuf| -> LockAndReloadFn {
            Arc::new(move || {
                let lock_file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&lk)
                    .ok()?;
                lock_file.lock_exclusive().ok()?;
                let token = read_secret_file(rt.to_str()?).ok()?;
                Some((token, None, Box::new(lock_file) as Box<dyn Send>))
            })
        };

        let cb_a = make_closure(rt_path.clone(), lock_path.clone());
        let cb_b = make_closure(rt_path.clone(), lock_path.clone());

        // Thread A: acquires the lock, writes the rotated token (simulating persist),
        // signals B that the new token is on disk, then holds the lock a moment longer
        // to ensure B is already blocking on flock before the lock releases.
        let rt_for_write = rt_path.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let thread_a = std::thread::spawn(move || {
            let (token, _at, guard) = cb_a().expect("cb_a");
            write_secret_file(&rt_for_write, "token-2").unwrap();
            ready_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(guard);
            token
        });

        // Wait until A has written token-2, then contend on the lock.
        // B will block at flock until A's sleep expires and the guard drops.
        ready_rx.recv().unwrap();
        let (token_b, _at_b, guard_b) = cb_b().expect("cb_b");
        drop(guard_b);

        let token_a = thread_a.join().unwrap();

        assert_eq!(token_a, "token-1");
        assert_eq!(
            token_b, "token-2",
            "B must read the token A wrote, not the original"
        );
    }
}

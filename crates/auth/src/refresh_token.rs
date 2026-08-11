// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use display_error_chain::ErrorChainExt;
use parking_lot::RwLock;
use reqwest::Client as ReqwestClient;

use crate::errors::AuthError;
use crate::jwt::{extract_exp_claim_unsafe, extract_sub_claim_unsafe};
use crate::resolver::same_origin;
use crate::traits::TokenProvider;

const REFRESH_BUFFER_SECS: u64 = 60;

#[derive(Clone)]
pub struct RefreshTokenProviderConfig {
    /// Initial refresh token. Copied into the provider's internal `current_refresh_token`
    /// on construction; the internal field is the live value after that.
    pub refresh_token: String,
    /// OIDC issuer URL; the token endpoint is discovered from
    /// `{issuer_url}/.well-known/openid-configuration` on first use.
    pub issuer_url: String,
    pub client_id: String,
    pub timeout: Option<Duration>,
    /// If provided, used as the current access token on startup instead of
    /// immediately fetching a new one. Seeded into the cache if still valid;
    /// otherwise a fresh token is fetched immediately.
    pub initial_access_token: Option<String>,
    /// Called after a successful token exchange when the IdP issues a new
    /// refresh token (rotation). Arguments: (new_access_token, new_refresh_token).
    /// Use this to persist the rotated tokens so process restarts don't lose them.
    pub persist_credentials: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
}

struct CachedToken {
    token: String,
    refresh_at: u64,
}

#[derive(Clone)]
pub struct RefreshTokenProvider {
    config: RefreshTokenProviderConfig,
    // Updated in-memory when the IdP rotates the refresh token.
    current_refresh_token: Arc<RwLock<String>>,
    cached: Arc<RwLock<Option<CachedToken>>>,
    // Discovered once on first fetch; avoids repeated discovery round-trips.
    cached_token_endpoint: Arc<RwLock<Option<String>>>,
    // Prevents concurrent refresh spawns; only the first caller that sets this
    // to true proceeds, avoiding spurious invalid_grant errors from rotation.
    refreshing: Arc<AtomicBool>,
    client: ReqwestClient,
}

impl RefreshTokenProvider {
    pub fn new(mut config: RefreshTokenProviderConfig) -> Result<Self, AuthError> {
        config.issuer_url = config.issuer_url.trim_end_matches('/').to_owned();
        let parsed = url::Url::parse(&config.issuer_url)?;
        let is_loopback = matches!(
            parsed.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("::1")
        );
        if parsed.scheme() != "https" && !is_loopback {
            return Err(AuthError::OidcInsecureIssuerUrl(config.issuer_url.clone()));
        }

        let mut builder = ReqwestClient::builder();
        if let Some(t) = config.timeout {
            builder = builder.timeout(t);
        }
        let client = builder.build()?;
        let current_refresh_token = Arc::new(RwLock::new(config.refresh_token.clone()));

        Ok(Self {
            config,
            current_refresh_token,
            cached: Arc::new(RwLock::new(None)),
            cached_token_endpoint: Arc::new(RwLock::new(None)),
            refreshing: Arc::new(AtomicBool::new(false)),
            client,
        })
    }

    /// Discover the token endpoint from the issuer's OIDC discovery document.
    /// The result is cached so discovery only happens once.
    async fn get_token_endpoint(&self) -> Result<String, AuthError> {
        if let Some(ep) = self.cached_token_endpoint.read().clone() {
            return Ok(ep);
        }
        let issuer_parsed = url::Url::parse(&self.config.issuer_url)?;
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer_url
        );
        let doc: serde_json::Value = self.client.get(&discovery_url).send().await?.json().await?;
        let token_endpoint = doc
            .get("token_endpoint")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::OidcDiscoveryMissingTokenEndpoint)?
            .to_owned();
        let token_url = url::Url::parse(&token_endpoint)?;
        if !same_origin(&issuer_parsed, &token_url) {
            return Err(AuthError::OidcDiscoveryUrlOriginMismatch {
                field: "token_endpoint",
                url: token_endpoint,
            });
        }
        *self.cached_token_endpoint.write() = Some(token_endpoint.clone());
        Ok(token_endpoint)
    }

    async fn fetch_new_token(&self) -> Result<(), AuthError> {
        let token_endpoint = self.get_token_endpoint().await?;
        let refresh_token = self.current_refresh_token.read().clone();

        let http_resp = self
            .client
            .post(&token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", self.config.client_id.as_str()),
            ])
            .send()
            .await?;

        let status = http_resp.status();
        let body = http_resp.text().await?;
        let resp: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

        if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
            if err == "invalid_grant" {
                return Err(AuthError::RefreshTokenRevoked);
            }
            let desc = resp
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("no description");
            return Err(AuthError::TokenEndpointError {
                status: status.as_u16(),
                body: format!("{err}: {desc}"),
            });
        }

        if !status.is_success() {
            return Err(AuthError::TokenEndpointError {
                status: status.as_u16(),
                body,
            });
        }

        let access_token = resp["access_token"]
            .as_str()
            .ok_or(AuthError::GetTokenError)?
            .to_owned();

        let expires_in = resp["expires_in"].as_u64().unwrap_or(3600);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let refresh_at = now + (expires_in * 2 / 3).max(REFRESH_BUFFER_SECS + 1);

        if let Some(new_rt) = resp["refresh_token"].as_str() {
            *self.current_refresh_token.write() = new_rt.to_owned();
            if let Some(cb) = self.config.persist_credentials.clone() {
                let at = access_token.clone();
                let rt = new_rt.to_owned();
                tokio::task::spawn_blocking(move || cb(at, rt));
            }
        }

        *self.cached.write() = Some(CachedToken {
            token: access_token,
            refresh_at,
        });

        Ok(())
    }
}

impl TokenProvider for RefreshTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        if let Some(token) = &self.config.initial_access_token {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Ok(exp) = extract_exp_claim_unsafe(token)
                && exp > now
            {
                let remaining = exp - now;
                let refresh_at = now + (remaining * 2 / 3).max(REFRESH_BUFFER_SECS + 1);
                *self.cached.write() = Some(CachedToken {
                    token: token.clone(),
                    refresh_at,
                });
                self.config.initial_access_token = None;
                tracing::debug!(exp, "seeded token cache from initial access token");
                return Ok(());
            }
            self.config.initial_access_token = None;
            tracing::debug!("initial access token is expired; fetching a new one");
        }

        self.fetch_new_token().await.inspect_err(|e| {
            tracing::error!(
                error = %e.chain(),
                "failed to obtain initial OIDC token; re-run `slimctl login`"
            );
        })
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let cached = self.cached.read();
        let c = cached.as_ref().ok_or(AuthError::GetTokenError)?;
        let token = c.token.clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let needs_refresh = now >= c.refresh_at;
        drop(cached);

        if needs_refresh && !self.refreshing.swap(true, Ordering::Relaxed) {
            let provider = self.clone();
            tokio::spawn(async move {
                match provider.fetch_new_token().await {
                    Ok(()) => {}
                    Err(AuthError::RefreshTokenRevoked) => {
                        tracing::error!("refresh token revoked or expired; re-run `slimctl login`");
                    }
                    Err(e) => {
                        tracing::error!(error = %e.chain(), "token refresh failed");
                    }
                }
                provider.refreshing.store(false, Ordering::Relaxed);
            });
        }

        Ok(token)
    }

    fn get_id(&self) -> Result<String, AuthError> {
        extract_sub_claim_unsafe(&self.get_token()?)
    }

    async fn set_signature_keys(
        &mut self,
        _private_key: Vec<u8>,
        _public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        Err(AuthError::MlsNotSupported)
    }
}

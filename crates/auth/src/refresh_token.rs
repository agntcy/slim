// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
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
    ///
    /// Runs on the blocking pool and is awaited before the exchange completes, so
    /// the rotated token is on disk before the new access token is handed out.
    /// Implementations may block (file I/O) but should not be slow: they sit in
    /// the path of every renewal.
    pub persist_credentials: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
}

struct CachedToken {
    token: String,
    exp: u64,
    refresh_at: tokio::time::Instant,
}

#[derive(Clone)]
pub struct RefreshTokenProvider {
    config: RefreshTokenProviderConfig,
    // Updated in-memory when the IdP rotates the refresh token.
    current_refresh_token: Arc<RwLock<String>>,
    cached: Arc<RwLock<Option<CachedToken>>>,
    // Discovered once on first fetch; avoids repeated discovery round-trips.
    cached_token_endpoint: Arc<RwLock<Option<String>>>,
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
        let refresh_at = tokio::time::Instant::now()
            + Duration::from_secs((expires_in * 2 / 3).max(REFRESH_BUFFER_SECS + 1));

        if let Some(new_rt) = resp["refresh_token"].as_str() {
            *self.current_refresh_token.write() = new_rt.to_owned();
            if let Some(cb) = self.config.persist_credentials.clone() {
                let at = access_token.clone();
                let rt = new_rt.to_owned();
                // Awaited, not detached: the in-memory refresh token has just
                // moved on, so until this lands the copy on disk is one the IdP
                // may already have invalidated. Losing the write means the next
                // process start replays a spent token and fails with
                // invalid_grant — exactly what persisting is meant to prevent.
                //
                // spawn_blocking because the callback does synchronous file I/O;
                // no lock is held across the await (the guard above is a
                // statement-level temporary).
                if let Err(e) = tokio::task::spawn_blocking(move || cb(at, rt)).await {
                    tracing::error!(
                        error = %e,
                        "failed to persist rotated refresh token; the copy on disk is now stale"
                    );
                }
            }
        }

        *self.cached.write() = Some(CachedToken {
            token: access_token,
            exp: now + expires_in,
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
                let refresh_at = tokio::time::Instant::now()
                    + Duration::from_secs((remaining * 2 / 3).max(REFRESH_BUFFER_SECS + 1));
                *self.cached.write() = Some(CachedToken {
                    token: token.clone(),
                    exp,
                    refresh_at,
                });
                self.config.initial_access_token = None;
                tracing::debug!(exp, "seeded token cache from initial access token");
            } else {
                self.config.initial_access_token = None;
                tracing::debug!("initial access token is expired; fetching a new one");
                self.fetch_new_token().await.inspect_err(|e| {
                    tracing::error!(
                        error = %e.chain(),
                        "failed to obtain initial OIDC token; re-run `slimctl login`"
                    );
                })?;
            }
        } else {
            self.fetch_new_token().await.inspect_err(|e| {
                tracing::error!(
                    error = %e.chain(),
                    "failed to obtain initial OIDC token; re-run `slimctl login`"
                );
            })?;
        }

        // Proactive background renewal: wakes at refresh_at and fetches a new
        // token before the current one expires, so get_token() always returns
        // a valid token under normal conditions.
        let provider = self.clone();
        tokio::spawn(async move {
            loop {
                let refresh_at = provider.cached.read().as_ref().map(|c| c.refresh_at);
                let Some(refresh_at) = refresh_at else { break };

                tokio::time::sleep_until(refresh_at).await;

                match provider.fetch_new_token().await {
                    Ok(()) => {}
                    Err(AuthError::RefreshTokenRevoked) => {
                        tracing::error!("refresh token revoked; re-run `slimctl login`");
                        break;
                    }
                    Err(e) => {
                        let err = e;
                        tracing::warn!(error = %err.chain(), "background token refresh failed; retrying in 30s");
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
                }
            }
        });

        Ok(())
    }

    fn get_token(&self) -> Result<String, AuthError> {
        let cached = self.cached.read();
        let c = cached.as_ref().ok_or(AuthError::GetTokenError)?;
        let token = c.token.clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let is_expired = now >= c.exp;
        drop(cached);

        if is_expired {
            return Err(AuthError::GetTokenError);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Mock IdP: discovery pointing at its own token endpoint, which rotates the
    /// refresh token on every exchange.
    async fn mock_idp(new_refresh_token: &str) -> MockServer {
        let server = MockServer::start().await;
        let uri = server.uri();

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": uri,
                "token_endpoint": format!("{uri}/token"),
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "new-access-token",
                "refresh_token": new_refresh_token,
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        server
    }

    fn provider(
        server: &MockServer,
        persist: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
    ) -> RefreshTokenProvider {
        // reqwest builds its TLS backend eagerly, so a crypto provider must be
        // installed even though the mock server speaks plaintext.
        slim_config::tls::provider::initialize_crypto_provider();
        RefreshTokenProvider::new(RefreshTokenProviderConfig {
            refresh_token: "seed-token".to_string(),
            issuer_url: server.uri(),
            client_id: "test-client".to_string(),
            timeout: Some(Duration::from_secs(5)),
            initial_access_token: None,
            persist_credentials: persist,
        })
        .expect("provider")
    }

    /// The persist callback must have completed by the time the exchange returns.
    ///
    /// This is the regression guard for detaching it: the callback sleeps, so with
    /// a fire-and-forget `spawn_blocking` the flag is still unset when
    /// `fetch_new_token` resolves.
    #[tokio::test(flavor = "multi_thread")]
    async fn persist_completes_before_fetch_returns() {
        let server = mock_idp("rotated-token").await;

        let done = Arc::new(AtomicBool::new(false));
        let seen: Arc<parking_lot::Mutex<Option<(String, String)>>> =
            Arc::new(parking_lot::Mutex::new(None));

        let cb_done = done.clone();
        let cb_seen = seen.clone();
        let p = provider(
            &server,
            Some(Arc::new(move |access, refresh| {
                // Blocking work, as a real file write would be.
                std::thread::sleep(Duration::from_millis(150));
                *cb_seen.lock() = Some((access, refresh));
                cb_done.store(true, Ordering::SeqCst);
            })),
        );

        p.fetch_new_token().await.expect("exchange");

        assert!(
            done.load(Ordering::SeqCst),
            "fetch_new_token returned before the rotated token was persisted"
        );
        assert_eq!(
            *seen.lock(),
            Some(("new-access-token".to_string(), "rotated-token".to_string()))
        );
    }

    /// The in-memory token and the persisted one must agree — the whole point of
    /// ordering the write.
    #[tokio::test(flavor = "multi_thread")]
    async fn persisted_token_matches_in_memory_token() {
        let server = mock_idp("rotated-token").await;
        let persisted: Arc<parking_lot::Mutex<Option<String>>> =
            Arc::new(parking_lot::Mutex::new(None));
        let sink = persisted.clone();

        let p = provider(
            &server,
            Some(Arc::new(move |_access, refresh| {
                *sink.lock() = Some(refresh);
            })),
        );

        p.fetch_new_token().await.expect("exchange");

        assert_eq!(persisted.lock().as_deref(), Some("rotated-token"));
        assert_eq!(*p.current_refresh_token.read(), "rotated-token");
    }

    /// No callback configured is not an error; rotation still updates memory.
    #[tokio::test(flavor = "multi_thread")]
    async fn rotation_without_persist_callback_is_fine() {
        let server = mock_idp("rotated-token").await;
        let p = provider(&server, None);

        p.fetch_new_token().await.expect("exchange");

        assert_eq!(*p.current_refresh_token.read(), "rotated-token");
        assert_eq!(p.get_token().expect("token"), "new-access-token");
    }

    /// A response with no `refresh_token` must not invoke the callback: there is
    /// nothing new to persist, and overwriting with the old value is pointless.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_rotation_means_no_persist() {
        let server = MockServer::start().await;
        let uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": uri, "token_endpoint": format!("{uri}/token"),
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "new-access-token",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let p = provider(
            &server,
            Some(Arc::new(move |_a, _r| {
                counter.fetch_add(1, Ordering::SeqCst);
            })),
        );

        p.fetch_new_token().await.expect("exchange");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(*p.current_refresh_token.read(), "seed-token");
    }
}

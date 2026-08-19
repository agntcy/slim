// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::AuthError;
use crate::jwt::extract_sub_claim_unsafe;
use crate::refresh_token::{RefreshTokenProvider, RefreshTokenProviderConfig};
use crate::resolver::JwksCache;
use crate::traits::{TokenProvider, Verifier};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD as BASE64_STD, URL_SAFE_NO_PAD as BASE64_URL};
use display_error_chain::ErrorChainExt;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

use crate::jwt::key_alg_to_algorithm;
use crate::resolver::same_origin;
use oauth2::{AuthUrl, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl, basic::BasicClient};
use parking_lot::RwLock;
use reqwest::Client as ReqwestClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use url::Url;

/// Returns an error if `url` does not use `https`, unless the host is localhost.
fn require_https(url: &str) -> Result<Url, AuthError> {
    let parsed = Url::parse(url)?;
    let is_loopback = matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    );
    if parsed.scheme() != "https" && !is_loopback {
        return Err(AuthError::OidcInsecureIssuerUrl(url.to_string()));
    }
    Ok(parsed)
}

// Default token refresh buffer (60 seconds before expiry)
const REFRESH_BUFFER_SECONDS: u64 = 60;

/// Separates the access token from the base64url MLS public key in the
/// credential providers hand out.
///
/// `cnf.jkt` is a one-way hash but the MLS layer needs the key itself, so the
/// holder presents it and the verifier re-hashes it against `cnf.jkt` — the
/// check an RFC 9449 resource server does on a proof's `jwk` header. Riding in
/// `SLIMHeader.identity`, already an opaque provider string, avoids a new proto
/// field that older relays would strip when re-encoding.
///
/// `~` is outside the JWT alphabet, so it cannot occur in either half.
const DPOP_KEY_SEPARATOR: char = '~';

/// Split a credential into `(access_token, presented_mls_public_key)`. Without
/// the separator it is a plain bearer token and passes through untouched.
pub(crate) fn split_credential(credential: &str) -> (&str, Option<&str>) {
    match credential.split_once(DPOP_KEY_SEPARATOR) {
        Some((token, key)) => (token, Some(key)),
        None => (credential, None),
    }
}

/// Inverse of [`split_credential`]. Both DPoP-capable providers go through here
/// so the format cannot drift between the grant that mints the binding and the
/// one that renews it.
pub(crate) fn present_credential(access_token: &str, public_key: Option<&[u8]>) -> String {
    // Only present a key the token commits to. MLS installs its own pair
    // whenever none is present, and pairing that with an unbound token would be
    // rejected by every peer on every message.
    let public_key = public_key.filter(|_| crate::dpop::token_confirmation(access_token).is_some());

    match public_key {
        Some(public_key) => format!(
            "{access_token}{DPOP_KEY_SEPARATOR}{}",
            BASE64_URL.encode(public_key)
        ),
        None => access_token.to_string(),
    }
}

/// Confirm a presented MLS public key is the one its token was bound to, then
/// surface it as a `pubkey` claim so everything downstream stays DPoP-unaware.
/// Written only after the thumbprint matches.
fn bind_presented_key(
    claims: &mut serde_json::Value,
    presented_key_b64url: &str,
) -> Result<(), AuthError> {
    let key = BASE64_URL.decode(presented_key_b64url)?;

    let expected = claims
        .get("cnf")
        .and_then(|cnf| cnf.get("jkt"))
        .and_then(|jkt| jkt.as_str())
        .ok_or(AuthError::DpopMissingConfirmation)?;

    if crate::dpop::jwk_thumbprint(&key)? != expected {
        return Err(AuthError::DpopThumbprintMismatch);
    }

    if let Some(obj) = claims.as_object_mut() {
        obj.insert(
            crate::identity_claims::claim_keys::PUBKEY.to_string(),
            serde_json::Value::String(BASE64_STD.encode(&key)),
        );
    }
    Ok(())
}

/// Remove unverified `pubkey` claims from both places
/// [`crate::identity_claims::IdentityClaims::from_json`] looks: top level and
/// `custom_claims`. Clearing only the former leaves the nested one adoptable.
fn strip_unverified_pubkey(claims: &mut serde_json::Value) {
    use crate::identity_claims::claim_keys::{CUSTOM_CLAIMS, PUBKEY};

    let Some(obj) = claims.as_object_mut() else {
        return;
    };
    obj.remove(PUBKEY);
    if let Some(custom) = obj.get_mut(CUSTOM_CLAIMS).and_then(|c| c.as_object_mut()) {
        custom.remove(PUBKEY);
    }
}

/// POST to a token endpoint with a DPoP proof when `signature_keys` are given.
///
/// Shared with `slimctl login` so the proof cannot differ between the grant that
/// mints the binding and the one that renews it.
///
/// Retries once on a `use_dpop_nonce` challenge (RFC 9449 §8). Only once: a
/// second challenge means a misbehaving endpoint, not a race.
pub async fn post_token_request_with_dpop(
    client: &ReqwestClient,
    token_endpoint: &str,
    form: &[(&str, &str)],
    signature_keys: Option<&(Vec<u8>, Vec<u8>)>,
) -> Result<serde_json::Value, AuthError> {
    let mut nonce: Option<String> = None;

    for attempt in 0..2 {
        let mut request = client.post(token_endpoint).form(form);
        if let Some((secret, public)) = signature_keys {
            let proof =
                crate::dpop::build_proof(secret, public, "POST", token_endpoint, nonce.as_deref())?;
            request = request.header("DPoP", proof);
        }

        let response = request.send().await?;
        let status = response.status();
        let server_nonce = response
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await?;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

        if let Some(error) = parsed.get("error").and_then(|v| v.as_str()) {
            // Retry once with the challenge nonce; a proof without it is
            // rejected by design, not because anything is wrong.
            if error == "use_dpop_nonce" && attempt == 0 && server_nonce.is_some() {
                nonce = server_nonce;
                continue;
            }
            // `invalid_grant` means "the refresh token is spent" only on the
            // refresh grant; on the authorization-code grant it means the code
            // was stale or replayed, and reporting that as a revoked refresh
            // token sends the caller down entirely the wrong path.
            let is_refresh_grant = form
                .iter()
                .any(|(k, v)| *k == "grant_type" && *v == "refresh_token");
            if error == "invalid_grant" && is_refresh_grant {
                return Err(AuthError::RefreshTokenRevoked);
            }
            let description = parsed
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("no description");
            return Err(AuthError::TokenEndpointError {
                status: status.as_u16(),
                body: format!("{error}: {description}"),
            });
        }

        if !status.is_success() {
            return Err(AuthError::TokenEndpointError {
                status: status.as_u16(),
                body,
            });
        }

        return Ok(parsed);
    }

    Err(AuthError::TokenEndpointError {
        status: 400,
        body: "authorization server kept demanding a new DPoP nonce".to_string(),
    })
}

/// Cache entry for OIDC access tokens
#[derive(Debug, Clone)]
struct TokenCacheEntry {
    /// The cached access token
    token: String,
    /// Expiration time in seconds since UNIX epoch
    expiry: u64,
    /// Time when the token should be refreshed (2/3 of lifetime)
    refresh_at: u64,
}

/// Cache for OIDC tokens to avoid repeated token requests
#[derive(Debug)]
struct OidcTokenCache {
    /// Map from cache key (issuer_url + client_id + scope) to token entry
    entries: RwLock<HashMap<String, TokenCacheEntry>>,
}

impl OidcTokenCache {
    /// Create a new OIDC token cache
    fn new() -> Self {
        OidcTokenCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store a token in the cache
    fn store(
        &self,
        key: impl Into<String>,
        token: impl Into<String>,
        expiry: u64,
        refresh_at: u64,
    ) {
        let entry = TokenCacheEntry {
            token: token.into(),
            expiry,
            refresh_at,
        };
        self.entries.write().insert(key.into(), entry);
    }

    /// Retrieve a token from the cache if it exists and is still valid
    fn get(&self, key: impl Into<String>) -> Option<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let key = key.into();
        if let Some(entry) = self.entries.read().get(&key)
            && entry.expiry > now + REFRESH_BUFFER_SECONDS
        {
            return Some(entry.token.clone());
        }
        None
    }

    /// Get tokens that need to be refreshed
    fn get_tokens_needing_refresh(&self) -> Vec<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        self.entries
            .read()
            .iter()
            .filter_map(|(key, entry)| {
                if now >= entry.refresh_at && entry.expiry > now + REFRESH_BUFFER_SECONDS {
                    Some(key.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Cache for JWKS to avoid repeated JWKS requests
#[derive(Debug)]
struct OidcJwksCache {
    /// Map from issuer URL to JWKS entry
    entries: RwLock<HashMap<String, JwksCache>>,
}

impl OidcJwksCache {
    /// Create a new JWKS cache
    fn new() -> Self {
        OidcJwksCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store JWKS in the cache with custom TTL
    fn store_with_ttl(&self, issuer_url: impl Into<String>, jwks: JwkSet, ttl: Duration) {
        let entry = JwksCache::new(jwks, Instant::now(), ttl);
        self.entries.write().insert(issuer_url.into(), entry);
    }

    /// Retrieve JWKS from the cache if it exists and is still valid
    fn get(&self, issuer_url: impl Into<String>) -> Option<JwkSet> {
        let key = issuer_url.into();
        if let Some(entry) = self.entries.read().get(&key) {
            // Use the per-entry TTL instead of hardcoded value
            if entry.fetched_at.elapsed() <= entry.ttl {
                return Some(entry.jwks.clone());
            }
        }
        None
    }
}

#[derive(Clone)]
pub struct OidcProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    pub issuer_url: String,
    pub scope: Option<String>,
    /// HTTP timeout for token requests (default: 30s)
    pub timeout: Option<Duration>,
}

/// OIDC Token Provider that implements the Client Credentials flow
#[derive(Clone)]
pub struct OidcTokenProvider {
    config: OidcProviderConfig,
    token_cache: Arc<OidcTokenCache>,
    client: ReqwestClient,
    /// Shutdown signal sender for the background refresh task
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Handle to the background refresh task
    refresh_task: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
    /// MLS signature key pair `(secret, public)`. Shared across clones so a
    /// rotation is visible to every session cloned from the same app. `None`
    /// keeps this a plain bearer token source.
    signature_keys: Arc<RwLock<Option<(Vec<u8>, Vec<u8>)>>>,
    /// Renewal delegate for a *user* identity; `None` means client credentials.
    ///
    /// Renewal, its schedule, persistence and the cross-process rotation lock
    /// all live in [`RefreshTokenProvider`]. A second copy here is what once let
    /// a user identity renew as the service account.
    refresh: Arc<RwLock<Option<RefreshTokenProvider>>>,
    /// Guards the delegate's renewal loop against a second `initialize`.
    delegate_started: Arc<AtomicBool>,
}

impl OidcTokenProvider {
    /// Create a new OIDC Token Provider synchronously
    /// Note: Call `initialize()` after creation to start background tasks and fetch initial token
    pub fn new(config: OidcProviderConfig) -> Result<Self, AuthError> {
        require_https(&config.issuer_url)?;

        // Create HTTP client with timeout
        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth OAuth2")
            .timeout(config.timeout.unwrap_or(Duration::from_secs(30)))
            .build()?;

        // Create shutdown channel for background task
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());

        Ok(Self {
            config,
            token_cache,
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            signature_keys: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Initialize the provider asynchronously - starts background tasks and fetches initial token
    async fn initialize(&mut self) -> Result<(), AuthError> {
        // Check if already initialized
        if self.refresh_task.lock().is_some() {
            return Ok(());
        }

        // A refresh token means a user identity: renewal, including its schedule,
        // belongs to the delegate. Clones share its state, so initializing this
        // handle populates what every clone reads.
        let delegate = self.refresh.read().clone();
        if let Some(mut delegate) = delegate {
            // A failed first call must stay retryable, or the provider reports
            // success while `get_token` fails forever.
            if self.delegate_started.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            return delegate.initialize().await.inspect_err(|_| {
                self.delegate_started.store(false, Ordering::SeqCst);
            });
        }

        // Create new shutdown receiver using the existing sender
        let shutdown_rx = self.shutdown_tx.subscribe();

        // Start background refresh task
        let refresh_task = self.start_refresh_task(shutdown_rx);
        *self.refresh_task.lock() = Some(refresh_task);

        // Fetch initial token to populate cache
        if let Err(e) = self.fetch_new_token().await {
            tracing::warn!(error = %e.chain(), "Warning: Failed to fetch initial token");
            // Don't fail initialization, let background task handle it
        }
        Ok(())
    }

    /// Generate cache key for token caching
    fn get_cache_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.config.issuer_url,
            self.config.client_id,
            self.config.scope.as_deref().unwrap_or("")
        )
    }

    /// Check if cached token is still valid
    #[cfg(test)]
    fn is_token_valid(&self, now: u64, expiry: u64) -> bool {
        expiry > now + REFRESH_BUFFER_SECONDS
    }

    /// Fetch the issuer's discovery document, with the parsed issuer URL, so a
    /// caller needing two endpoints from it pays for one round trip.
    pub(crate) async fn discovery_doc(&self) -> Result<(serde_json::Value, Url), AuthError> {
        let issuer_parsed = require_https(&self.config.issuer_url)?;
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer_url
        );
        let doc: serde_json::Value = self.client.get(&discovery_url).send().await?.json().await?;
        Ok((doc, issuer_parsed))
    }

    /// Token endpoint from a discovery document, rejected if off-origin.
    fn token_endpoint_from(doc: &serde_json::Value, issuer: &Url) -> Result<String, AuthError> {
        let token_endpoint = doc
            .get("token_endpoint")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::OidcDiscoveryMissingTokenEndpoint)?;

        let token_url = Url::parse(token_endpoint)?;
        if !same_origin(issuer, &token_url) {
            return Err(AuthError::OidcDiscoveryUrlOriginMismatch {
                field: "token_endpoint",
                url: token_endpoint.to_string(),
            });
        }
        Ok(token_endpoint.to_string())
    }

    /// Fetch a new token using client credentials flow
    async fn fetch_new_token(&self) -> Result<String, AuthError> {
        let (discovery_response, issuer_parsed) = self.discovery_doc().await?;
        let token_endpoint = Self::token_endpoint_from(&discovery_response, &issuer_parsed)?;
        let token_endpoint = token_endpoint.as_str();

        let auth_url_str = discovery_response
            .get("authorization_endpoint")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}/authorize", self.config.issuer_url));

        // Create OAuth2 client (updated for new oauth2 builder API)
        let client = BasicClient::new(ClientId::new(self.config.client_id.clone()))
            .set_client_secret(ClientSecret::new(self.config.client_secret.clone()))
            .set_auth_uri(AuthUrl::new(auth_url_str)?)
            .set_token_uri(TokenUrl::new(token_endpoint.to_string())?);

        let mut token_request = client.exchange_client_credentials();

        if let Some(ref scope) = self.config.scope {
            token_request = token_request.add_scope(Scope::new(scope.clone()));
        }

        let token_response = token_request
            .request_async(&self.client)
            .await
            .map_err(|e| AuthError::OAuth2Request(Box::new(e)))?;

        let access_token = token_response.access_token().secret();
        let expires_in = token_response
            .expires_in()
            .map(|duration| duration.as_secs())
            .unwrap_or(3600); // Default to 1 hour

        // Calculate expiry timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expiry = now + expires_in;

        // Calculate refresh time (2/3 of token lifetime) using integer math to avoid float casting
        let refresh_at = now + (expires_in * 2 / 3);

        // Cache the token using the structured cache
        let cache_key = self.get_cache_key();
        self.token_cache
            .store(cache_key, access_token, expiry, refresh_at);

        Ok(access_token.to_string())
    }

    /// POST a form to the token endpoint, carrying a DPoP proof when MLS keys
    /// are installed.
    async fn post_token_request(
        &self,
        token_endpoint: &str,
        form: &[(&str, &str)],
    ) -> Result<serde_json::Value, AuthError> {
        let keys = self.signature_keys.read().clone();
        post_token_request_with_dpop(&self.client, token_endpoint, form, keys.as_ref()).await
    }

    /// Cache the access token from a token-endpoint response, and adopt any
    /// refresh token — which is what marks this a user identity.
    fn store_token_response(&self, response: &serde_json::Value) -> Result<String, AuthError> {
        let access_token = response["access_token"]
            .as_str()
            .ok_or(AuthError::GetTokenError)?
            .to_owned();
        let expires_in = response["expires_in"].as_u64().unwrap_or(3600);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.token_cache.store(
            self.get_cache_key(),
            access_token.clone(),
            now + expires_in,
            now + (expires_in * 2 / 3),
        );

        // Seed the delegate with the token just issued, so `get_token` serves it
        // straight away rather than only after `initialize`.
        if let Some(refresh_token) = response["refresh_token"].as_str() {
            // Update in place: rebuilding would drop the `persist_credentials`
            // callback, so rotations would stop reaching disk.
            let existing = self.refresh.read().clone();
            match existing {
                Some(delegate) => delegate.replace_refresh_token(refresh_token),
                None => self.adopt_refresh_token(refresh_token, None, None)?,
            }
            if let Some(delegate) = self.refresh.read().as_ref() {
                delegate.seed_access_token(&access_token, expires_in);
            }
        }

        Ok(access_token)
    }

    /// Exchange an authorization code for a DPoP-bound access token.
    ///
    /// The grant that yields a *user* identity — a client-credentials token's
    /// `sub` is the client, not the human. With MLS keys installed the request
    /// carries a proof, so the token returns with a matching `cnf.jkt`.
    pub async fn exchange_authorization_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, AuthError> {
        let (doc, issuer) = self.discovery_doc().await?;
        let token_endpoint = Self::token_endpoint_from(&doc, &issuer)?;

        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", self.config.client_id.as_str()),
        ];
        // Public clients (the interactive login case) have no secret; sending an
        // empty one makes Keycloak reject the request as malformed.
        if !self.config.client_secret.is_empty() {
            form.push(("client_secret", self.config.client_secret.as_str()));
        }

        let response = self.post_token_request(&token_endpoint, &form).await?;
        self.store_token_response(&response)
    }

    /// Start the background refresh task
    fn start_refresh_task(&self, mut shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
        let provider_clone = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check for tokens that need refreshing
                        let tokens_to_refresh = provider_clone.token_cache.get_tokens_needing_refresh();

                        for cache_key in tokens_to_refresh {
                            // Extract the parts from the cache key to determine which token to refresh
                            // For now, we'll just refresh the current provider's token if it matches
                            let current_cache_key = provider_clone.get_cache_key();
                            if cache_key == current_cache_key
                                && let Err(e) = provider_clone.refresh_token_background().await
                            {
                                tracing::error!(error = %e.chain(), "failed to refresh token in background");
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        })
    }

    /// Refresh the client-credentials token in the background. Never reached in
    /// delegated mode, where this grant would swap the user for the service.
    async fn refresh_token_background(&self) -> Result<(), AuthError> {
        let result = self.fetch_new_token().await.map(|_| ());
        if let Err(ref e) = result {
            tracing::error!(error = %e.chain(), "failed to refresh token in background");
        }
        result
    }

    /// Install the MLS signature key pair the credential is bound to.
    ///
    /// Sync counterpart to
    /// [`TokenProvider::set_signature_keys`](crate::traits::TokenProvider::set_signature_keys),
    /// so config can seed keys minted by `slimctl login` outside an async context.
    pub fn install_signature_keys(
        &self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        // Reject a key type DPoP cannot express now, rather than at every later
        // verification once the credential is already in flight.
        let thumbprint = crate::dpop::jwk_thumbprint(&public_key)?;

        // Refuse a key the live token was not issued for. `cnf.jkt` cannot be
        // re-bound locally, so swapping the key under it (an MLS rotation, say)
        // would break the identity for every peer, silently. Rotating an OIDC
        // identity's key means signing in again.
        if let Some(bound) = self.bound_thumbprint()
            && bound != thumbprint
        {
            return Err(AuthError::DpopThumbprintMismatch);
        }

        // The delegate renews, so a key installed only here stops being proved.
        if let Some(delegate) = self.refresh.read().as_ref() {
            delegate.install_signature_keys(private_key.clone(), public_key.clone())?;
        }

        *self.signature_keys.write() = Some((private_key, public_key));
        Ok(())
    }

    /// The `cnf.jkt` of the currently served token, if it is DPoP-bound.
    fn bound_thumbprint(&self) -> Option<String> {
        // `get_token` picks the live cache for the current mode.
        let credential = self.get_token().ok()?;
        crate::dpop::token_confirmation(split_credential(&credential).0)
    }

    /// Hand renewal to [`RefreshTokenProvider`], using a refresh token from
    /// `slimctl login` or an authorization-code exchange.
    ///
    /// `persist_credentials` runs on rotation — supply it so a restart resumes
    /// the chain instead of replaying an invalidated token. Sync, because config
    /// builds providers outside async; the delegate fetches in `initialize`.
    pub fn adopt_refresh_token(
        &self,
        refresh_token: impl Into<String>,
        initial_access_token: Option<String>,
        persist_credentials: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
    ) -> Result<(), AuthError> {
        let delegate = RefreshTokenProvider::new(RefreshTokenProviderConfig {
            refresh_token: refresh_token.into(),
            issuer_url: self.config.issuer_url.clone(),
            client_id: self.config.client_id.clone(),
            timeout: self.config.timeout,
            initial_access_token,
            persist_credentials,
            lock_and_reload: None,
        })?;

        // Carry over any installed key so renewals prove the same one.
        if let Some((secret, public)) = self.signature_keys.read().clone() {
            delegate.install_signature_keys(secret, public)?;
        }

        *self.refresh.write() = Some(delegate);
        Ok(())
    }

    /// Shutdown the background refresh task
    pub fn shutdown(&self) {
        if let Err(e) = self.shutdown_tx.send(true) {
            // Print the error message during drop
            tracing::debug!(error = %e.chain(), "Failed to send shutdown signal");
        }
    }
}

impl TokenProvider for OidcTokenProvider {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        OidcTokenProvider::initialize(self).await
    }

    fn get_token(&self) -> Result<String, AuthError> {
        // Exactly one cache per mode: reading the wrong one serves a service
        // token under a user's MLS key.
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.get_token();
        }

        let cache_key = self.get_cache_key();
        let token = self
            .token_cache
            .get(&cache_key)
            .ok_or(AuthError::GetTokenError)?;

        Ok(present_credential(
            &token,
            self.signature_keys
                .read()
                .as_ref()
                .map(|(_, public)| &**public),
        ))
    }

    fn get_id(&self) -> Result<String, AuthError> {
        let credential = self.get_token()?;
        // Parse the access token, not the presented-key suffix.
        extract_sub_claim_unsafe(split_credential(&credential).0)
    }

    fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        self.signature_keys
            .read()
            .clone()
            .ok_or(AuthError::MlsNotSupported)
    }

    fn mls_signature_keys_installed(&self) -> bool {
        self.signature_keys.read().is_some()
    }

    async fn set_signature_keys(
        &mut self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        self.install_signature_keys(private_key, public_key)
    }
}

impl Drop for OidcTokenProvider {
    fn drop(&mut self) {
        // Signal shutdown when the provider is dropped
        if let Err(e) = self.shutdown_tx.send(true) {
            // Print the error message during drop
            tracing::debug!(error = %e.chain(), "Failed to send shutdown signal");
        }
    }
}

/// Derive the signing algorithm from a JWK — never from the untrusted token header.
/// Prefers the explicit `alg` field; falls back to inferring from key type.
/// Symmetric algorithms (HS*) are rejected: they have no place in a public JWKS.
fn alg_from_jwk(jwk: &Jwk) -> Result<Algorithm, AuthError> {
    if let Some(key_alg) = &jwk.common.key_algorithm {
        match key_alg {
            KeyAlgorithm::HS256 | KeyAlgorithm::HS384 | KeyAlgorithm::HS512 => {
                return Err(AuthError::JwtUnsupportedKeyAlgorithm(*key_alg));
            }
            _ => return key_alg_to_algorithm(key_alg),
        }
    }
    match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => Ok(Algorithm::RS256),
        AlgorithmParameters::EllipticCurve(_) => Ok(Algorithm::ES256),
        AlgorithmParameters::OctetKeyPair(_) => Ok(Algorithm::EdDSA),
        AlgorithmParameters::OctetKey(_) => Err(AuthError::JwtMissingKeyAlgorithm),
    }
}

/// OIDC Token Verifier that validates JWTs using JWKS
#[derive(Clone)]
pub struct OidcVerifier {
    issuer_url: String,
    audience: String,
    jwks_cache: Arc<OidcJwksCache>,
    http_client: ReqwestClient,
    jwks_ttl: Duration,
    userinfo_endpoint: Arc<std::sync::OnceLock<String>>,
    // When Some, merged claims are cached for claim_cache_ttl per token.
    claim_cache: Option<Arc<RwLock<HashMap<String, (serde_json::Value, Instant)>>>>,
    claim_cache_ttl: Duration,
}

impl OidcVerifier {
    /// Create a new OIDC Token Verifier
    pub fn new(issuer_url: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            audience: audience.into(),
            jwks_cache: Arc::new(OidcJwksCache::new()),
            http_client: reqwest::Client::new(),
            jwks_ttl: Duration::from_secs(3600), // Default 1 hour
            userinfo_endpoint: Arc::new(std::sync::OnceLock::new()),
            claim_cache: None,
            claim_cache_ttl: Duration::ZERO,
        }
    }

    /// Create a new OIDC Token Verifier with custom JWKS TTL
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_ttl = ttl;
        self
    }

    /// Enable claim caching with the given TTL.
    /// When enabled, merged JWT+userinfo claims are cached per token for `ttl`.
    pub fn with_claim_cache(mut self, ttl: Duration) -> Self {
        self.claim_cache_ttl = ttl;
        self.claim_cache = Some(Arc::new(RwLock::new(HashMap::new())));
        self
    }

    /// Fetch JWKS from the issuer
    async fn fetch_jwks(&self) -> Result<JwkSet, AuthError> {
        let issuer_parsed = require_https(&self.issuer_url)?;
        let discovery_url = format!("{}/.well-known/openid-configuration", self.issuer_url);
        let discovery_response: serde_json::Value = self
            .http_client
            .get(&discovery_url)
            .send()
            .await?
            .json()
            .await?;

        // OIDC spec: 'issuer' in discovery doc must match the URL used to discover it.
        let doc_issuer = discovery_response
            .get("issuer")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::OidcDiscoveryMissingIssuer)?;
        if doc_issuer.trim_end_matches('/') != self.issuer_url.trim_end_matches('/') {
            return Err(AuthError::OidcDiscoveryIssuerMismatch {
                expected: self.issuer_url.clone(),
                got: doc_issuer.to_string(),
            });
        }

        let jwks_uri = discovery_response
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::OidcDiscoveryMissingJwksUri)?;

        // jwks_uri must be on the same origin as the issuer.
        let jwks_url = Url::parse(jwks_uri)?;
        if !same_origin(&issuer_parsed, &jwks_url) {
            return Err(AuthError::OidcDiscoveryUrlOriginMismatch {
                field: "jwks_uri",
                url: jwks_uri.to_string(),
            });
        }

        if let Some(ep) = discovery_response
            .get("userinfo_endpoint")
            .and_then(|v| v.as_str())
        {
            let _ = self.userinfo_endpoint.set(ep.to_string());
        }

        let jwks: JwkSet = self
            .http_client
            .get(jwks_url.as_str())
            .send()
            .await?
            .json()
            .await?;

        Ok(jwks)
    }

    /// Get JWKS (from cache or fetch new)
    async fn get_jwks(&self) -> Result<JwkSet, AuthError> {
        // Check cache first
        if let Some(cached_jwks) = self.jwks_cache.get(&self.issuer_url) {
            return Ok(cached_jwks);
        }

        // Fetch new JWKS and cache it with the configured TTL
        let jwks = self.fetch_jwks().await?;
        self.jwks_cache
            .store_with_ttl(&self.issuer_url, jwks.clone(), self.jwks_ttl);
        Ok(jwks)
    }

    /// Verify a full credential (see [`DPOP_KEY_SEPARATOR`]) against JWKS. Any
    /// presented key is checked against `cnf.jkt` and surfaced as `pubkey`.
    fn verify_token_util(
        &self,
        credential: &str,
        jwks: &JwkSet,
    ) -> Result<serde_json::Value, AuthError> {
        let (token, presented_key) = split_credential(credential);
        let header = decode_header(token)?;

        let jwk = match header.kid {
            Some(kid) => jwks
                .keys
                .iter()
                .find(|k| k.common.key_id.as_deref() == Some(&kid))
                .ok_or(AuthError::OidcKeyNotFound(kid))?,
            None => match jwks.keys.as_slice() {
                [single] => single,
                _ => return Err(AuthError::OidcMissingKidWithMultipleKeys),
            },
        };

        let decoding_key = DecodingKey::from_jwk(jwk)?;
        let mut validation = Validation::new(alg_from_jwk(jwk)?);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[&self.issuer_url]);

        let token_data = decode::<serde_json::Value>(token, &decoding_key, &validation)?;
        let mut claims = token_data.claims;

        match presented_key {
            Some(presented_key) => bind_presented_key(&mut claims, presented_key)?,
            // Nothing proved possession, so a `pubkey` claim in the token is
            // unverified — and `from_json` would hand it to MLS as a binding.
            // Here it may only ever come from a verified `cnf.jkt`.
            None => strip_unverified_pubkey(&mut claims),
        }

        Ok(claims)
    }

    /// Fetch userinfo claims; returns empty object on any error (best effort).
    async fn userinfo_claims(&self, token: &str) -> serde_json::Value {
        let Some(endpoint) = self.userinfo_endpoint.get() else {
            tracing::debug!("userinfo_endpoint not discovered yet");
            return serde_json::Value::Object(Default::default());
        };
        match self
            .http_client
            .get(endpoint)
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => resp.json::<serde_json::Value>().await.unwrap_or_default(),
            Err(e) => {
                tracing::debug!(error=%e, "userinfo fetch failed, proceeding without");
                serde_json::Value::Object(Default::default())
            }
        }
    }

    /// Verify a JWT token and enrich claims from userinfo.
    async fn verify_token<Claims>(&self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned,
    {
        if let Some(cache) = &self.claim_cache
            && let Some((cached_claims, expiry)) = cache.read().get(token)
            && Instant::now() < *expiry
        {
            return Ok(serde_json::from_value(cached_claims.clone())?);
        }

        let jwks = self.get_jwks().await?;
        let mut claims = self.verify_token_util(token, &jwks)?;
        // Bearer-auth call: the access token alone, never the key suffix.
        let extra = self.userinfo_claims(split_credential(token).0).await;
        if let (Some(obj), Some(extra_obj)) = (claims.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.entry(k).or_insert_with(|| v.clone());
            }
        }

        // The merge can reintroduce a `pubkey` that `verify_token_util` stripped.
        if split_credential(token).1.is_none() {
            strip_unverified_pubkey(&mut claims);
        }

        if let Some(cache) = &self.claim_cache {
            cache.write().insert(
                token.to_owned(),
                (claims.clone(), Instant::now() + self.claim_cache_ttl),
            );
        }

        Ok(serde_json::from_value(claims)?)
    }
}

impl Verifier for OidcVerifier {
    async fn initialize(&mut self) -> Result<(), AuthError> {
        Ok(()) // no-op
    }

    async fn verify(&self, token: impl AsRef<str> + Send) -> Result<(), AuthError> {
        // Verify the token structure is valid - this will fetch JWKS if needed
        let _: serde_json::Value = self.verify_token(token.as_ref()).await?;
        Ok(())
    }

    fn try_verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
        if let Some(cached_jwks) = self.jwks_cache.get(&self.issuer_url) {
            self.verify_token_util(token.as_ref(), &cached_jwks)?;
            Ok(())
        } else {
            Err(AuthError::WouldBlockOn)
        }
    }

    async fn get_claims<Claims>(&self, token: impl AsRef<str> + Send) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        self.verify_token(token.as_ref()).await
    }

    fn try_get_claims<Claims>(&self, token: impl AsRef<str>) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        if let Some(cache) = &self.claim_cache
            && let Some((cached_claims, expiry)) = cache.read().get(token.as_ref())
            && Instant::now() < *expiry
        {
            return Ok(serde_json::from_value(cached_claims.clone())?);
        }

        // Verify against the cached JWKS, as `try_verify` does. This is the only
        // path MLS has — `validate_member` is sync — so returning `WouldBlockOn`
        // fails every member of an OIDC-backed group. Userinfo needs async and is
        // skipped; `sub` and `cnf` come from the token anyway.
        let jwks = self
            .jwks_cache
            .get(&self.issuer_url)
            .ok_or(AuthError::WouldBlockOn)?;
        let claims = self.verify_token_util(token.as_ref(), &jwks)?;
        Ok(serde_json::from_value(claims)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Use the test utilities from the testutils module
    use slim_testing::utils::{TestClaims, setup_oidc_mock_server, setup_test_jwt_resolver};

    #[tokio::test]
    async fn test_oidc_token_provider_client_credentials_flow() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url,
            scope: Some("api:read".to_string()),
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

        // Test token retrieval
        let token = provider.get_token().unwrap();
        assert_eq!(token, expected_token);
    }

    #[tokio::test]
    async fn test_oidc_token_provider_caching() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

        // First call - should fetch token
        let token1 = provider.get_token().unwrap();
        assert_eq!(token1, expected_token);

        // Second call - should use cached token
        let token2 = provider.get_token().unwrap();
        assert_eq!(token2, expected_token);
        assert_eq!(token1, token2);
    }

    #[tokio::test]
    async fn test_oidc_verifier_simple_mock() {
        // Use the existing utility to set up mock server
        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create verifier and test that it can fetch JWKS
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Test that we can fetch JWKS successfully
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert_eq!(jwks.keys.len(), 1);
        // We can't easily check the key type without additional structure info,
        // but we can verify we have a key with an ID
        assert!(jwks.keys[0].common.key_id.is_some());
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwt_verification() {
        // Setup mock OIDC server with JWKS using the existing utility
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        // Create test claims
        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");

        // Create JWT token without kid (since we have only one key)
        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        // Create verifier
        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First test that JWKS can be fetched
        let jwks = verifier.fetch_jwks().await.unwrap();
        assert!(!jwks.keys.is_empty());

        // Now verify the token
        let verified_claims: TestClaims = verifier.get_claims(token).await.unwrap();
        assert_eq!(verified_claims.sub, "user123");
        assert_eq!(verified_claims.aud, "test-audience");
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwks_caching() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // First verification - should fetch JWKS
        let result1: TestClaims = verifier.get_claims(token.clone()).await.unwrap();
        assert_eq!(result1.sub, "user123");

        // Second verification - should use cached JWKS
        let result2: TestClaims = verifier.get_claims(token).await.unwrap();
        assert_eq!(result2.sub, "user123");
    }

    #[tokio::test]
    async fn test_oidc_verifier_invalid_token() {
        let (_private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Try to verify invalid token
        let result: Result<TestClaims, _> = verifier.get_claims("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oidc_verifier_missing_kid_single_key_works() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Create token without kid in header - this should work with single key
        let mut header = Header::new(Algorithm::RS256);
        header.kid = None; // Explicitly remove kid
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should succeed because kid is missing but there's only one key available
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
        if let Err(e) = &result {
            println!("Unexpected error: {:?}", e);
        }
        assert!(
            result.is_ok(),
            "Expected success with single key and no kid, got error: {:?}",
            result.err()
        );

        let verified_claims = result.unwrap();
        assert_eq!(verified_claims.sub, "user123");
        assert_eq!(verified_claims.aud, "test-audience");
    }

    #[tokio::test]
    async fn test_oidc_verifier_unsupported_key_type() {
        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock OIDC discovery endpoint
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{}/auth", issuer_url),
                "token_endpoint": format!("{}/oauth2/token", issuer_url),
                "jwks_uri": format!("{}/jwks.json", issuer_url),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"]
            })))
            .mount(&mock_server)
            .await;

        // Mock JWKS with unsupported key type
        Mock::given(method("GET"))
            .and(path("/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [{
                    "kty": "oct", // Symmetric key - not supported
                    "kid": "test-key-id",
                    "k": "test-key-value"
                }]
            })))
            .mount(&mock_server)
            .await;

        // Create a token with the test key ID
        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-id".to_string());

        // Use a dummy key for encoding (the test will fail at key type validation)
        // We need to use the proper algorithm for the encoding to work
        let header = Header::new(Algorithm::HS256); // Use HS256 for symmetric key
        let encoding_key = EncodingKey::from_secret("dummy-secret".as_ref());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key type is not supported
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oidc_verifier_key_not_found() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user123".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Create token with non-existent key ID
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("non-existent-key-id".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Should fail because key is not found in JWKS
        let result: Result<TestClaims, _> = verifier.get_claims(token).await;
        assert!(result.is_err_and(|e| matches!(e, AuthError::OidcKeyNotFound(_))));
    }

    #[tokio::test]
    async fn test_oidc_token_provider_creation() {
        // Use the existing setup function
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;
        let config = OidcProviderConfig {
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            issuer_url,
            scope: Some("scope".to_string()),
            timeout: None,
        };
        let provider_result = OidcTokenProvider::new(config);

        // Test that the provider can be created successfully with proper OIDC server
        match provider_result {
            Ok(mut provider) => {
                assert_eq!(provider.config.scope, Some("scope".to_string()));
                // Test that initialization works
                provider.initialize().await.unwrap();
            }
            Err(_e) => {
                panic!("provider creation should have succeeded");
            }
        }
    }

    #[test]
    fn test_oidc_verifier_creation() {
        let verifier = OidcVerifier::new("https://example.com", "audience");

        assert_eq!(verifier.issuer_url, "https://example.com");
        assert_eq!(verifier.audience, "audience");
        assert_eq!(verifier.jwks_ttl, Duration::from_secs(3600)); // Default 1 hour
    }

    #[test]
    fn test_oidc_verifier_custom_ttl() {
        let custom_ttl = Duration::from_secs(1800); // 30 minutes
        let verifier =
            OidcVerifier::new("https://example.com", "audience").with_jwks_ttl(custom_ttl);

        assert_eq!(verifier.issuer_url, "https://example.com");
        assert_eq!(verifier.audience, "audience");
        assert_eq!(verifier.jwks_ttl, custom_ttl);
    }

    #[test]
    fn test_jwks_cache_entry_reuse() {
        // Test that we're using the shared JwksCache struct from resolver.rs
        let jwks = JwkSet { keys: vec![] };
        let entry = JwksCache::new(jwks, Instant::now(), Duration::from_secs(1800));

        // Verify the struct has the expected fields
        assert_eq!(entry.ttl, Duration::from_secs(1800));
        assert!(entry.jwks.keys.is_empty());
    }

    #[tokio::test]
    async fn test_token_validity_check() {
        let (_mock_server, issuer_url, _expected_token) = setup_oidc_mock_server().await;

        let config = OidcProviderConfig {
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        };
        let mut provider = OidcTokenProvider::new(config).unwrap();
        provider.initialize().await.unwrap();

        let now = 1000;
        let expiry_valid = now + REFRESH_BUFFER_SECONDS + 100; // Valid token
        let expiry_invalid = now + REFRESH_BUFFER_SECONDS - 100; // Invalid token

        assert!(provider.is_token_valid(now, expiry_valid));
        assert!(!provider.is_token_valid(now, expiry_invalid));
    }

    #[tokio::test]
    async fn test_oidc_token_provider_error_handling() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock discovery endpoint returning error (404 Not Found)
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        // Manually create a provider without calling the constructor
        // to avoid the hanging issue during construction
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());
        let http_client = reqwest::Client::new();

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url: issuer_url.clone(),
            scope: None,
            timeout: None,
        };

        let provider = OidcTokenProvider {
            config,
            token_cache: token_cache.clone(),
            client: http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            signature_keys: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AtomicBool::new(false)),
        };

        // Test that fetch_new_token fails when discovery endpoint returns 404
        let result = provider.fetch_new_token().await;
        assert!(result.is_err());

        // Should get a ConfigError due to discovery failure
        match result {
            Err(AuthError::HttpError(_)) => {}
            other => {
                panic!(
                    "Expected ConfigError for discovery failure, but got: {:?}",
                    other
                );
            }
        }
    }

    #[tokio::test]
    async fn test_oidc_token_provider_invalid_token_response() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let issuer_url = mock_server.uri();

        // Mock discovery endpoint with required fields
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{}/auth", issuer_url),
                "token_endpoint": format!("{}/oauth2/token", issuer_url),
                "jwks_uri": format!("{}/oauth2/jwks.json", issuer_url),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "grant_types_supported": ["authorization_code", "client_credentials"]
            })))
            .mount(&mock_server)
            .await;

        // Mock token endpoint returning proper OAuth2 error (400 Bad Request)
        // This is how OAuth2 servers should return errors according to RFC 6749
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({
                        "error": "invalid_client",
                        "error_description": "Client authentication failed"
                    })),
            )
            .mount(&mock_server)
            .await;

        // Mock JWKS endpoint (required for discovery)
        Mock::given(method("GET"))
            .and(path("/oauth2/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": []
            })))
            .mount(&mock_server)
            .await;

        // Manually create a provider without calling the constructor
        // to avoid the hanging issue during construction
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let token_cache = Arc::new(OidcTokenCache::new());

        let http_client = reqwest::Client::new();

        let config = OidcProviderConfig {
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            issuer_url: issuer_url.clone(),
            scope: None,
            timeout: None,
        };

        let provider = OidcTokenProvider {
            config,
            token_cache: token_cache.clone(),
            client: http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            signature_keys: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AtomicBool::new(false)),
        };

        // Test that fetch_new_token fails with proper OAuth2 error
        let result = provider.fetch_new_token().await;
        assert!(result.is_err());

        // Should get an OAuth2Request (boxed RequestTokenError) due to the OAuth2 error response
        match result {
            Err(AuthError::OAuth2Request(e)) => {
                // The typed RequestTokenError implements Display; inspect its message
                let msg = e.to_string();
                assert!(
                    msg.contains("Server returned error response"),
                    "OAuth2 error message did not contain expected text: {}",
                    msg
                );
            }
            other => {
                panic!(
                    "Expected OAuth2Request containing OAuth2 error, but got: {:?}",
                    other
                );
            }
        }
    }

    #[test]
    fn test_oidc_verifier_try_get_claims_always_async() {
        // try_get_claims always signals WouldBlockOn so the middleware takes the async path,
        // which is required for the userinfo fetch.
        let verifier = OidcVerifier::new("https://example.com", "test-audience");
        let result: Result<TestClaims, _> = verifier.try_get_claims("any.token.value");
        assert!(matches!(result, Err(AuthError::WouldBlockOn)));
    }

    #[tokio::test]
    async fn test_oidc_verifier_initialize_noop() {
        let mut verifier = OidcVerifier::new("https://example.com", "audience");
        // initialize is a no-op; should succeed without error
        verifier.initialize().await.unwrap();
    }

    #[tokio::test]
    async fn test_oidc_verifier_verify_async() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user456".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Exercise Verifier::verify (async)
        verifier.verify(token).await.unwrap();
    }

    #[tokio::test]
    async fn test_oidc_verifier_try_verify_with_cached_jwks() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims {
            sub: "user789".to_string(),
            iss: issuer_url.clone(),
            aud: "test-audience".to_string(),
            exp: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600),
            iat: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Populate JWKS cache so try_verify can use it synchronously
        let _jwks = verifier.get_jwks().await.unwrap();

        // Exercise Verifier::try_verify (sync with warm cache)
        verifier.try_verify(&token).unwrap();
    }

    #[test]
    fn test_oidc_verifier_try_verify_without_cache_returns_would_block() {
        let verifier = OidcVerifier::new("https://example.com", "audience");
        // No JWKS cache — must return WouldBlockOn
        let result = verifier.try_verify("any.token.value");
        assert!(matches!(result, Err(AuthError::WouldBlockOn)));
    }

    // ---------------------------------------------------------------------
    // Composite credential: <access_token>~<base64url MLS public key>
    // ---------------------------------------------------------------------

    /// Nothing proved possession, so MLS must not accept the claim as a binding.
    /// Both places `from_json` looks must be cleared — clearing only the top
    /// level leaves the `custom_claims` fallback open.
    #[test]
    fn unbound_token_cannot_smuggle_a_pubkey_claim() {
        let (_, public_key) = crate::utils::generate_mls_signature_keys().unwrap();
        let encoded = BASE64_STD.encode(&public_key);

        for mut claims in [
            json!({ "sub": "attacker", "pubkey": encoded }),
            json!({ "sub": "attacker", "custom_claims": { "pubkey": encoded } }),
            json!({
                "sub": "attacker",
                "pubkey": encoded,
                "custom_claims": { "pubkey": encoded },
            }),
        ] {
            strip_unverified_pubkey(&mut claims);
            // Downstream then refuses the credential rather than trusting it.
            assert!(
                matches!(
                    crate::identity_claims::IdentityClaims::from_json(&claims),
                    Err(AuthError::PublicKeyNotFound)
                ),
                "unverified pubkey survived in {claims}"
            );
        }
    }

    /// The one legitimate `pubkey` must survive stripping.
    #[test]
    fn bound_pubkey_is_not_stripped() {
        let (_, public_key) = crate::utils::generate_mls_signature_keys().unwrap();
        let mut claims = json!({
            "sub": "user",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&public_key).unwrap() },
        });
        bind_presented_key(&mut claims, &BASE64_URL.encode(&public_key)).unwrap();

        let parsed = crate::identity_claims::IdentityClaims::from_json(&claims).unwrap();
        assert_eq!(parsed.public_key, BASE64_STD.encode(&public_key));
    }

    /// `cnf.jkt` cannot be re-bound locally, so swapping the key under a live
    /// token would break the identity for every peer, silently.
    #[tokio::test]
    async fn install_signature_keys_refuses_to_break_a_live_binding() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        let jkt = crate::dpop::jwk_thumbprint(&public).unwrap();
        // A token whose cnf.jkt binds the key installed below.
        let claims = BASE64_URL.encode(json!({ "sub": "u", "cnf": { "jkt": jkt } }).to_string());
        let access_token = format!("h.{claims}.s");

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token, "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        provider
            .install_signature_keys(secret.clone(), public.clone())
            .unwrap();
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        // An MLS key rotation would land here.
        let (other_secret, other_public) = crate::utils::generate_mls_signature_keys().unwrap();
        assert!(matches!(
            provider.install_signature_keys(other_secret, other_public),
            Err(AuthError::DpopThumbprintMismatch)
        ));

        // Re-installing the same key is not a change and stays allowed.
        assert!(provider.install_signature_keys(secret, public).is_ok());
    }

    /// On this grant it means a stale code, not a spent refresh token — the two
    /// need different recovery.
    #[tokio::test]
    async fn invalid_grant_on_auth_code_is_not_reported_as_revoked_refresh() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "Code not valid",
            })))
            .mount(&server)
            .await;

        match provider_for(&issuer)
            .exchange_authorization_code("stale", "v", "http://127.0.0.1/cb")
            .await
        {
            Err(AuthError::TokenEndpointError { body, .. }) => {
                assert!(body.contains("invalid_grant"), "got {body}");
                assert!(
                    body.contains("Code not valid"),
                    "description dropped: {body}"
                );
            }
            other => panic!("expected TokenEndpointError, got {other:?}"),
        }
    }

    /// The transport-auth path and every non-MLS caller.
    #[test]
    fn split_credential_passes_plain_token_through() {
        let (token, key) = split_credential("header.payload.signature");
        assert_eq!(token, "header.payload.signature");
        assert!(key.is_none());
    }

    #[test]
    fn split_credential_separates_presented_key() {
        let (token, key) = split_credential("header.payload.signature~QUJD");
        assert_eq!(token, "header.payload.signature");
        assert_eq!(key, Some("QUJD"));
    }

    /// A matching key is surfaced as `pubkey`, keeping consumers DPoP-unaware.
    #[test]
    fn bind_presented_key_injects_pubkey_when_thumbprint_matches() {
        let (_, public_key) = crate::utils::generate_mls_signature_keys().unwrap();
        let jkt = crate::dpop::jwk_thumbprint(&public_key).unwrap();

        let mut claims = json!({ "sub": "user-id", "cnf": { "jkt": jkt } });
        bind_presented_key(&mut claims, &BASE64_URL.encode(&public_key)).unwrap();

        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&public_key),
            "verified key must be surfaced in the encoding IdentityClaims expects"
        );
    }

    /// Credential theft: a valid token replayed with the attacker's own key.
    #[test]
    fn bind_presented_key_rejects_key_the_token_is_not_bound_to() {
        let (_, victim_key) = crate::utils::generate_mls_signature_keys().unwrap();
        let (_, attacker_key) = crate::utils::generate_mls_signature_keys().unwrap();

        let mut claims = json!({
            "sub": "victim",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&victim_key).unwrap() }
        });

        let result = bind_presented_key(&mut claims, &BASE64_URL.encode(&attacker_key));
        assert!(matches!(result, Err(AuthError::DpopThumbprintMismatch)));
        assert!(
            claims.get("pubkey").is_none(),
            "a rejected key must never leave a pubkey claim behind"
        );
    }

    /// No `cnf` means nothing to bind against; accepting would trust the key on
    /// the holder's say-so.
    #[test]
    fn bind_presented_key_rejects_token_without_confirmation_claim() {
        let (_, public_key) = crate::utils::generate_mls_signature_keys().unwrap();
        let mut claims = json!({ "sub": "user-id" });

        let result = bind_presented_key(&mut claims, &BASE64_URL.encode(&public_key));
        assert!(matches!(result, Err(AuthError::DpopMissingConfirmation)));
    }

    /// Attacker-controlled input reaching this before any check has passed.
    #[test]
    fn bind_presented_key_rejects_malformed_key() {
        let mut claims = json!({ "sub": "user-id", "cnf": { "jkt": "whatever" } });
        assert!(bind_presented_key(&mut claims, "not!base64url").is_err());

        let mut claims = json!({ "sub": "user-id", "cnf": { "jkt": "whatever" } });
        // Valid base64url, but not a key length DPoP can express.
        let result = bind_presented_key(&mut claims, &BASE64_URL.encode([0u8; 20]));
        assert!(matches!(result, Err(AuthError::DpopUnsupportedKeyType)));
    }

    /// What `get_token` emits must be what `bind_presented_key` accepts.
    #[tokio::test]
    async fn provider_credential_round_trips_through_binding() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        let access_token = bound_token(&public);

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token, "token_type": "Bearer", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        // Before MLS installs keys the credential is a plain bearer token.
        provider.fetch_new_token().await.unwrap();
        assert_eq!(provider.get_token().unwrap(), access_token);
        assert!(!provider.mls_signature_keys_installed());

        provider
            .set_signature_keys(secret, public.clone())
            .await
            .unwrap();
        assert!(provider.mls_signature_keys_installed());

        let credential = provider.get_token().unwrap();
        let (token, presented) = split_credential(&credential);
        assert_eq!(token, access_token);

        let mut claims = json!({
            "sub": "user-id",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&public).unwrap() }
        });
        bind_presented_key(&mut claims, presented.unwrap()).unwrap();
        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&public)
        );
    }

    /// The *only* path MLS has, since `validate_member` is sync. `WouldBlockOn`
    /// here fails every member of an OIDC-backed group.
    #[tokio::test]
    async fn try_get_claims_verifies_from_cached_jwks_without_a_claim_cache() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");
        assert!(
            verifier.claim_cache.is_none(),
            "precondition: no claim cache, as every constructor defaults"
        );

        // Warm the JWKS cache, as any prior async verification would.
        Verifier::verify(&verifier, &token).await.unwrap();

        // The sync path MLS depends on must now work, with no claim cache.
        let verified: TestClaims = verifier.try_get_claims(&token).unwrap();
        assert_eq!(verified.sub, "user123");
    }

    /// Refused at install time, not on every later verification.
    #[tokio::test]
    async fn set_signature_keys_rejects_unmappable_key_type() {
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, _) = setup_oidc_mock_server().await;
        let mut provider = OidcTokenProvider::new(OidcProviderConfig {
            client_id: "c".to_string(),
            client_secret: "s".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        })
        .unwrap();

        let result = provider
            .set_signature_keys(vec![0u8; 32], vec![0u8; 20])
            .await;
        assert!(matches!(result, Err(AuthError::DpopUnsupportedKeyType)));
        assert!(!provider.mls_signature_keys_installed());
    }

    // ---------------------------------------------------------------------
    // Authorization-code grant with DPoP
    // ---------------------------------------------------------------------

    /// Discovery + token endpoint; the caller mounts the token response.
    async fn mock_issuer() -> (MockServer, String) {
        let server = MockServer::start().await;
        let issuer = server.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{issuer}/token"),
                "authorization_endpoint": format!("{issuer}/auth"),
            })))
            .mount(&server)
            .await;
        (server, issuer)
    }

    /// A JWT-shaped token carrying `cnf.jkt` for `public_key`, as a DPoP-enabled
    /// IdP returns. Tests of the presented-key path need a bound token.
    fn bound_token(public_key: &[u8]) -> String {
        let jkt = crate::dpop::jwk_thumbprint(public_key).unwrap();
        let claims =
            BASE64_URL.encode(json!({ "sub": "user-id", "cnf": { "jkt": jkt } }).to_string());
        format!("header.{claims}.signature")
    }

    fn provider_for(issuer: &str) -> OidcTokenProvider {
        OidcTokenProvider::new(OidcProviderConfig {
            client_id: "slim-app".to_string(),
            client_secret: String::new(), // public client, as in interactive login
            issuer_url: issuer.to_string(),
            scope: None,
            timeout: None,
        })
        .unwrap()
    }

    /// Without a proof signed by the MLS key the IdP has nothing to bind.
    #[tokio::test]
    async fn authorization_code_exchange_sends_dpop_proof() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        let access_token = bound_token(&public);

        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::header_exists("DPoP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token,
                "refresh_token": "user-refresh-token",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .set_signature_keys(secret, public.clone())
            .await
            .unwrap();

        let token = provider
            .exchange_authorization_code("the-code", "the-verifier", "http://127.0.0.1:1234/cb")
            .await
            .unwrap();
        assert_eq!(token, access_token);

        // The credential handed onward presents the key the proof was signed with.
        let credential = provider.get_token().unwrap();
        let (access, presented) = split_credential(&credential);
        assert_eq!(access, access_token);
        assert_eq!(presented, Some(BASE64_URL.encode(&public).as_str()));
    }

    /// MLS installs its own pair whenever none is present. Pairing that with an
    /// unbound token would be rejected by every peer, on every message.
    #[tokio::test]
    async fn unbound_token_is_served_without_a_presented_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                // No cnf.jkt: an IdP with DPoP disabled.
                "access_token": "header.eyJzdWIiOiJ1In0.signature",
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        provider.set_signature_keys(secret, public).await.unwrap();
        provider.fetch_new_token().await.unwrap();

        let credential = provider.get_token().unwrap();
        assert_eq!(
            split_credential(&credential).1,
            None,
            "an unbound token must not carry a presented key"
        );
    }

    /// A structurally-present but unverifiable header is worthless.
    #[tokio::test]
    async fn dpop_proof_binds_the_token_request() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        provider
            .set_signature_keys(secret, public.clone())
            .await
            .unwrap();
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let request = requests.last().unwrap();
        let proof = request.headers.get("DPoP").unwrap().to_str().unwrap();
        let parts: Vec<&str> = proof.split('.').collect();

        let header: serde_json::Value =
            serde_json::from_slice(&BASE64_URL.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["typ"], "dpop+jwt");
        // Advertised key is the MLS key the token will be bound to.
        assert_eq!(
            BASE64_URL.encode(<sha2::Sha256 as sha2::Digest>::digest(
                serde_json::to_string(&header["jwk"]).unwrap().as_bytes()
            )),
            crate::dpop::jwk_thumbprint(&public).unwrap()
        );

        let payload: serde_json::Value =
            serde_json::from_slice(&BASE64_URL.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(payload["htm"], "POST");
        assert_eq!(payload["htu"], format!("{issuer}/token"));

        // Signature verifies under the advertised key.
        use p256::ecdsa::signature::Verifier;
        let signature =
            p256::ecdsa::Signature::from_slice(&BASE64_URL.decode(parts[2]).unwrap()).unwrap();
        crate::utils::p256_verifying_key(&public)
            .unwrap()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    /// RFC 9449 §8. Without the retry the flow hard-fails against a
    /// nonce-configured Keycloak.
    #[tokio::test]
    async fn retries_once_with_server_supplied_dpop_nonce() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        // First attempt: challenge. `up_to_n_times` so the retry falls through
        // to the success mock below.
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("DPoP-Nonce", "server-nonce-value")
                    .set_body_json(json!({ "error": "use_dpop_nonce" })),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "nonce-ok", "expires_in": 3600,
            })))
            .with_priority(2)
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        provider.set_signature_keys(secret, public).await.unwrap();

        let token = provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();
        assert_eq!(token, "nonce-ok");

        // The retry must carry the nonce the server asked for.
        let requests = server.received_requests().await.unwrap();
        let retry = requests.last().unwrap();
        let proof = retry.headers.get("DPoP").unwrap().to_str().unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&BASE64_URL.decode(proof.split('.').nth(1).unwrap()).unwrap())
                .unwrap();
        assert_eq!(payload["nonce"], "server-nonce-value");
    }

    /// A spent refresh token must surface as its own error so the caller can
    /// Must surface distinctly so the caller re-logins rather than retrying.
    #[tokio::test]
    async fn invalid_grant_on_refresh_maps_to_refresh_token_revoked() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "Token is not active",
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider.adopt_refresh_token("spent", None, None).unwrap();
        assert!(matches!(
            TokenProvider::initialize(&mut provider).await,
            Err(AuthError::RefreshTokenRevoked)
        ));
    }

    /// Reusing the key keeps `cnf.jkt` unchanged across renewal.
    #[tokio::test]
    async fn refresh_reuses_the_same_mls_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        let renewed_public = public.clone();

        // The DPoP header is required by the mock: a renewal that dropped the
        // proof would return an unbound token and silently break the identity.
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "refresh_token=rt-1",
            ))
            .and(wiremock::matchers::header_exists("DPoP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": bound_token(&renewed_public),
                "refresh_token": "rt-2",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .set_signature_keys(secret, public.clone())
            .await
            .unwrap();
        provider.adopt_refresh_token("rt-1", None, None).unwrap();

        TokenProvider::initialize(&mut provider).await.unwrap();

        // Same key still presented, so the thumbprint the IdP bound is still valid.
        let credential = provider.get_token().unwrap();
        let (access, presented) = split_credential(&credential);
        assert_eq!(access, bound_token(&public));
        assert_eq!(presented, Some(BASE64_URL.encode(&public).as_str()));
    }

    /// Client credentials would swap in the service account's `sub` and break
    /// every MLS binding made under the user's identity.
    #[tokio::test]
    async fn adopted_refresh_token_renews_with_the_refresh_grant() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        // Only the refresh grant is answered; a client-credentials request would
        // match no mock and fail.
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "grant_type=refresh_token",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "renewed", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider.adopt_refresh_token("rt", None, None).unwrap();
        TokenProvider::initialize(&mut provider).await.unwrap();

        assert_eq!(provider.get_token().unwrap(), "renewed");
        assert!(
            !server.received_requests().await.unwrap().iter().any(|r| {
                String::from_utf8_lossy(&r.body).contains("grant_type=client_credentials")
            }),
            "a user identity must never renew via client credentials"
        );
    }

    /// Serving the wrong cache presents a service token under a user's MLS key.
    #[tokio::test]
    async fn client_credentials_and_delegated_modes_do_not_cross_caches() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "grant_type=refresh_token",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "user-token", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        // Populate the client-credentials cache, as a service-mode provider would.
        provider.token_cache.store(
            provider.get_cache_key(),
            "service-token",
            u64::MAX,
            u64::MAX,
        );
        assert_eq!(provider.get_token().unwrap(), "service-token");

        // Once a refresh token is adopted, the delegate's cache is the only one read.
        provider.adopt_refresh_token("rt", None, None).unwrap();
        TokenProvider::initialize(&mut provider).await.unwrap();
        assert_eq!(provider.get_token().unwrap(), "user-token");
    }
}

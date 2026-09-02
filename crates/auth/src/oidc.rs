// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::AuthError;
use crate::jwt::extract_sub_claim_unsafe;
use crate::refresh_token::{RefreshTokenProvider, RefreshTokenProviderConfig};
use crate::resolver::JwksCache;
use crate::traits::{ExportedIdentity, TokenProvider, Verifier};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STD;
use display_error_chain::ErrorChainExt;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

use crate::jwt::key_alg_to_algorithm;
use crate::resolver::same_origin;
use oauth2::{AuthUrl, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl, basic::BasicClient};
use parking_lot::RwLock;
use reqwest::Client as ReqwestClient;
use reqwest::StatusCode;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use url::Url;

#[cfg(test)]
thread_local! {
    // require_https is synchronous and never awaits internally, so every call
    // it guards runs start-to-finish on whichever thread is currently live —
    // no risk of a `multi_thread` runtime hopping the check to another thread
    // mid-way, even for tests that use that flavor.
    static ALLOW_INSECURE_ISSUER_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// RAII test guard letting `require_https` accept the plain-http mock servers
/// wiremock starts (it has no TLS support wired up here). Keep the binding
/// alive (`let _guard = ...`, not `let _ = ...`) for as long as the test's
/// provider may still be reaching out — including its background refresh
/// task — since the guard only holds while it isn't dropped.
#[cfg(test)]
pub(crate) struct AllowInsecureIssuerForTest;

#[cfg(test)]
impl AllowInsecureIssuerForTest {
    pub(crate) fn new() -> Self {
        ALLOW_INSECURE_ISSUER_FOR_TEST.with(|f| f.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for AllowInsecureIssuerForTest {
    fn drop(&mut self) {
        ALLOW_INSECURE_ISSUER_FOR_TEST.with(|f| f.set(false));
    }
}

/// Returns an error if `url` does not use `https`.
pub(crate) fn require_https(url: &str) -> Result<Url, AuthError> {
    let parsed = Url::parse(url)?;
    #[cfg(test)]
    if ALLOW_INSECURE_ISSUER_FOR_TEST.with(|f| f.get()) {
        return Ok(parsed);
    }
    if parsed.scheme() != "https" {
        return Err(AuthError::OidcInsecureIssuerUrl(url.to_string()));
    }
    Ok(parsed)
}

/// Discovery URL for an issuer that has already been checked.
///
/// Takes the parsed `Url` rather than the raw config string so the only way to
/// build one is from a value that passed [`require_https`] (or the equivalent
/// check in `RefreshTokenProvider::new`). Nothing but the issuer origin and path
/// reaches the request — the document is fetched with no credential of any kind.
pub(crate) fn discovery_url(issuer: &Url) -> String {
    format!(
        "{}/.well-known/openid-configuration",
        issuer.as_str().trim_end_matches('/')
    )
}

// Default token refresh buffer (60 seconds before expiry)
const REFRESH_BUFFER_SECONDS: u64 = 60;

/// Attempts `OidcVerifier::revalidate` makes against the userinfo endpoint
/// before treating a run of failures as inconclusive rather than confirmed
/// revocation.
const REVALIDATE_ATTEMPTS: u32 = 3;
/// Delay between `revalidate` attempts.
const REVALIDATE_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Separates the access token from the MLS key attestation in the credential
/// providers hand out.
///
/// `cnf.jkt` is a one-way hash of the *identity* key, and the MLS layer needs a
/// *leaf* key that the identity key vouches for, so the holder presents an
/// attestation carrying both: the identity key inline as the JWS `jwk` header
/// (which the verifier re-hashes against `cnf.jkt`, the check an RFC 9449
/// resource server does on a proof) and the leaf key in the signed payload.
/// Riding in `SLIMHeader.identity`, already an opaque provider string, avoids a
/// new proto field that older relays would strip when re-encoding.
///
/// `~` is outside the JWT and base64url alphabets, so it cannot occur in either
/// half.
const DPOP_KEY_SEPARATOR: char = '~';

/// Split a credential into `(access_token, key_attestation)`. Without the
/// separator it is a plain bearer token and passes through untouched.
pub(crate) fn split_credential(credential: &str) -> (&str, Option<&str>) {
    match credential.split_once(DPOP_KEY_SEPARATOR) {
        Some((token, attestation)) => (token, Some(attestation)),
        None => (credential, None),
    }
}

/// Inverse of [`split_credential`]. Both DPoP-capable providers go through here
/// so the format cannot drift between the grant that mints the binding and the
/// one that renews it.
pub(crate) fn present_credential(access_token: &str, attestation: Option<&str>) -> String {
    // Only present an attestation the token commits to. A token with no
    // `cnf.jkt` cannot name the signer, so every peer would reject the pair;
    // serving the bare bearer token instead keeps the transport path working
    // and fails MLS admission loudly at `PublicKeyNotFound` rather than
    // obscurely at a thumbprint comparison.
    let attestation =
        attestation.filter(|_| crate::dpop::token_confirmation(access_token).is_some());

    match attestation {
        Some(attestation) => format!("{access_token}{DPOP_KEY_SEPARATOR}{attestation}"),
        None => access_token.to_string(),
    }
}

/// Verify a key attestation against the token that carries it, then surface the
/// attested MLS key as a `pubkey` claim so everything downstream stays
/// attestation-unaware.
///
/// Two links are checked here, and neither is sufficient alone:
///
/// 1. the attestation verifies under the key in its own `jwk` header, and
/// 2. that key's thumbprint is the token's `cnf.jkt`, so the IdP vouched for it.
///
/// Without (2) this would prove possession of an arbitrary key — which any group
/// member can do with a credential read off a peer's leaf.
fn bind_presented_key(claims: &mut serde_json::Value, attestation: &str) -> Result<(), AuthError> {
    let expected = claims
        .get("cnf")
        .and_then(|cnf| cnf.get("jkt"))
        .and_then(|jkt| jkt.as_str())
        .ok_or(AuthError::DpopMissingConfirmation)?;

    let attested = crate::dpop::verify_key_attestation(attestation)?;
    if attested.signer_jkt != expected {
        return Err(AuthError::DpopThumbprintMismatch);
    }

    if let Some(obj) = claims.as_object_mut() {
        obj.insert(
            crate::identity_claims::claim_keys::PUBKEY.to_string(),
            serde_json::Value::String(BASE64_STD.encode(&attested.leaf_public_key)),
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
/// `is_refresh_grant` is the caller's own knowledge of which grant `form`
/// carries — this is a generic transport helper shared by every grant type, so
/// it must not infer OAuth semantics by inspecting the form body itself.
///
/// Retries once on a `use_dpop_nonce` challenge (RFC 9449 §8). Only once: a
/// second challenge means a misbehaving endpoint, not a race.
pub async fn post_token_request_with_dpop(
    client: &ReqwestClient,
    token_endpoint: &str,
    form: &[(&str, &str)],
    signature_keys: Option<&(Vec<u8>, Vec<u8>)>,
    is_refresh_grant: bool,
) -> Result<serde_json::Value, AuthError> {
    let mut nonce: Option<String> = None;

    for attempt in 0..2 {
        // token_endpoint is https, enforced by require_https / same_origin
        // against an already-validated issuer. The DPoP proof below is a
        // signature derived from the signing key, not the key itself;
        // CodeQL's no-build extraction for Rust can't see either invariant.
        // codeql[rust/cleartext-transmission]
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
            if error == "use_dpop_nonce" && server_nonce.is_some() {
                // Retry once with the challenge nonce; a proof without it is
                // rejected by design, not because anything is wrong.
                if attempt == 0 {
                    nonce = server_nonce;
                    continue;
                }
                // A second challenge means a misbehaving endpoint, not a race.
                return Err(AuthError::TokenEndpointError {
                    status: status.as_u16(),
                    body: "authorization server kept demanding a new DPoP nonce".to_string(),
                });
            }
            // `invalid_grant` means "the refresh token is spent" only on the
            // refresh grant; on the authorization-code grant it means the code
            // was stale or replayed, and reporting that as a revoked refresh
            // token sends the caller down entirely the wrong path.
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

    unreachable!("the loop above always returns on both iterations")
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
    token: Arc<RwLock<Option<TokenCacheEntry>>>,
    client: ReqwestClient,
    /// Shutdown signal sender for the background refresh task
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Handle to the background refresh task
    refresh_task: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
    /// The DPoP-bound identity key this credential is confirmed for, plus the
    /// MLS leaf key it currently attests. Shared across clones so an MLS key
    /// rotation is visible to every session cloned from the same app. `None`
    /// keeps this a plain bearer token source — which is what the transport
    /// path wants, and why `create_provider` deliberately does not seed it.
    identity: Arc<RwLock<Option<crate::dpop::IdentityKey>>>,
    /// Renewal delegate for a *user* identity; `None` means client credentials.
    ///
    /// Renewal, its schedule, persistence and the cross-process rotation lock
    /// all live in [`RefreshTokenProvider`]. A second copy here is what once let
    /// a user identity renew as the service account.
    refresh: Arc<RwLock<Option<RefreshTokenProvider>>>,
    /// Guards the delegate's renewal loop against a second concurrent
    /// `initialize`. Held across the delegate's own `initialize().await`, so a
    /// concurrent caller waits for the in-flight fetch instead of observing
    /// success before a token exists.
    delegate_started: Arc<AsyncMutex<bool>>,
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

        Ok(Self {
            config,
            token: Arc::new(RwLock::new(None)),
            client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            identity: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AsyncMutex::new(false)),
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
            // Held across the `.await` below, so a concurrent second caller
            // blocks on the in-flight fetch instead of racing ahead and
            // observing success (via the swap-then-return-early pattern this
            // replaced) before the delegate has actually fetched a token.
            let mut started = self.delegate_started.lock().await;
            if *started {
                return Ok(());
            }
            // A failed first call must stay retryable, or the provider reports
            // success while `get_token` fails forever.
            delegate.initialize().await?;
            *started = true;
            return Ok(());
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

    /// Check if cached token is still valid
    #[cfg(test)]
    fn is_token_valid(&self, now: u64, expiry: u64) -> bool {
        expiry > now + REFRESH_BUFFER_SECONDS
    }

    /// Fetch the issuer's discovery document, with the parsed issuer URL, so a
    /// caller needing two endpoints from it pays for one round trip.
    pub(crate) async fn discovery_doc(&self) -> Result<(serde_json::Value, Url), AuthError> {
        let issuer_parsed = require_https(&self.config.issuer_url)?;
        let discovery_url = discovery_url(&issuer_parsed);
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

        *self.token.write() = Some(TokenCacheEntry {
            token: access_token.to_string(),
            expiry,
            refresh_at,
        });

        Ok(access_token.to_string())
    }

    /// POST a form to the token endpoint, carrying a DPoP proof when an identity
    /// key is installed. Only caller is [`Self::exchange_authorization_code`],
    /// so this is never the refresh grant.
    async fn post_token_request(
        &self,
        token_endpoint: &str,
        form: &[(&str, &str)],
    ) -> Result<serde_json::Value, AuthError> {
        // The proof is over the *identity* key: it is what the IdP will name in
        // `cnf.jkt`, and the MLS leaf key must never reach the token endpoint.
        let keys = self.identity.read().as_ref().map(|k| k.pop_keys().clone());
        post_token_request_with_dpop(&self.client, token_endpoint, form, keys.as_ref(), false).await
    }

    /// Cache the access token from a token-endpoint response, and adopt any
    /// refresh token — which is what marks this a user identity.
    fn store_token_response(&self, response: &serde_json::Value) -> Result<String, AuthError> {
        let access_token = response["access_token"]
            .as_str()
            .ok_or(AuthError::GetTokenError)?
            .to_owned();
        let expires_in = response["expires_in"].as_u64().unwrap_or(3600);

        match response["refresh_token"].as_str() {
            Some(refresh_token) => {
                // Seed the delegate with the token just issued, so `get_token`
                // serves it straight away rather than only after `initialize`.
                //
                // Update in place: rebuilding would drop the
                // `persist_credentials` callback, so rotations would stop
                // reaching disk.
                let existing = self.refresh.read().clone();
                match existing {
                    Some(delegate) => delegate.replace_refresh_token(refresh_token),
                    None => self.adopt_refresh_token(refresh_token, None, None)?,
                }
                if let Some(delegate) = self.refresh.read().as_ref() {
                    delegate.seed_access_token(&access_token, expires_in);
                }
            }
            // Already delegated and this response carries no rotation: the
            // delegate's own cache is the only one `get_token` reads once a
            // delegate exists, so there is nothing for this cache to do.
            None if self.has_refresh_delegate() => {}
            None => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                *self.token.write() = Some(TokenCacheEntry {
                    token: access_token.clone(),
                    expiry: now + expires_in,
                    refresh_at: now + (expires_in * 2 / 3),
                });
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

    /// Whether the background loop should attempt a refresh right now. `None`
    /// means the initial fetch in `initialize` failed (it deliberately doesn't
    /// fail startup), so this must return true or the provider is stuck
    /// tokenless forever.
    fn needs_background_refresh(&self, now: u64) -> bool {
        match self.token.read().as_ref() {
            None => true,
            Some(entry) => now >= entry.refresh_at && entry.expiry > now + REFRESH_BUFFER_SECONDS,
        }
    }

    /// Start the background refresh task
    fn start_refresh_task(&self, mut shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
        let provider_clone = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();

                        if provider_clone.needs_background_refresh(now)
                            && let Err(e) = provider_clone.refresh_token_background().await
                        {
                            tracing::error!(error = %e.chain(), "failed to refresh token in background");
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

    /// Install the DPoP-bound *identity* key (`K_pop`) this credential is
    /// confirmed for — the key `slimctl login` proved at the token exchange.
    ///
    /// Sync, so config can seed it outside an async context. Refuses a key the
    /// live token was not issued for: `cnf.jkt` cannot be re-bound locally, so a
    /// mismatch here means the store and the token came from different logins
    /// and every peer would reject the pair.
    pub fn install_identity_keys(
        &self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        // Reject a key type DPoP cannot express now, rather than at every later
        // signing attempt once a credential is already in flight.
        let thumbprint = crate::dpop::jwk_thumbprint(&public_key)?;

        if let Some(bound) = self.bound_thumbprint()
            && bound != thumbprint
        {
            return Err(AuthError::DpopThumbprintMismatch);
        }

        // Once a delegate exists, it renews and so it is the single source of
        // truth for the keys — write only there, rather than keeping a second
        // copy here that nothing reads but that could still drift out of sync.
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.install_identity_keys(private_key, public_key);
        }

        let mut guard = self.identity.write();
        // Re-installing the same key is a no-op: replacing the struct here would
        // discard any MLS leaf key already installed under it.
        if guard.as_ref().is_some_and(|k| k.pop_keys().1 == public_key) {
            return Ok(());
        }
        *guard = Some(crate::dpop::IdentityKey::new(private_key, public_key)?);
        Ok(())
    }

    /// Install the MLS leaf key the MLS layer generated, to be attested under
    /// the identity key.
    ///
    /// Sync counterpart to
    /// [`TokenProvider::set_signature_keys`](crate::traits::TokenProvider::set_signature_keys).
    ///
    /// Unlike the identity key this is *unconditional*: the leaf key is not what
    /// `cnf.jkt` names, so it may be replaced as often as `mls-rs` likes — a
    /// rotation, a restored snapshot, or a per-session key all just produce a
    /// new attestation. Requiring a login per MLS key was the whole cost this
    /// indirection removes.
    pub fn install_signature_keys(
        &self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.install_signature_keys(private_key, public_key);
        }

        // No identity key means nothing can vouch for this leaf key, and a
        // credential that presents an unattested key is rejected by every peer.
        // Fail here, naming the fix, rather than at the first MLS handshake.
        self.identity
            .write()
            .as_mut()
            .ok_or(AuthError::AttestationNoIdentityKey)?
            .install_leaf(private_key, public_key);
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

        // Move any installed keys into the delegate — it becomes the sole owner
        // from here on, so no second copy can drift out of sync.
        if let Some(identity) = self.identity.write().take() {
            delegate.adopt_identity_key(identity);
        }

        *self.refresh.write() = Some(delegate);

        // A client-credentials background task from an earlier `initialize()`
        // would otherwise keep renewing the service token forever after this
        // provider becomes a user identity — wasting requests against the token
        // endpoint and attaching a DPoP proof to a grant nothing downstream
        // reads. Retire it: renewal now belongs to the delegate, started the
        // next time `initialize()` runs.
        if let Some(task) = self.refresh_task.lock().take() {
            self.shutdown();
            task.abort();
        }

        Ok(())
    }

    /// Whether a DPoP-bound identity key is installed, i.e. whether this
    /// provider can attest an MLS key at all.
    ///
    /// Distinct from `mls_signature_keys_installed`, which reports the *leaf*
    /// key and stays false until the MLS layer generates one. Config checks this
    /// at construction: a store with no identity key builds fine and then fails
    /// on a peer's machine at the first group join.
    pub fn identity_key_installed(&self) -> bool {
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.identity_key_installed();
        }
        self.identity.read().is_some()
    }

    /// The MLS leaf key, but only once the MLS layer has installed its own —
    /// never the pair seeded at construction.
    ///
    /// `session_layer` persists whatever `export_identity` returns, and a
    /// placeholder persisted there would come back on restore looking like a key
    /// the MLS layer had chosen: `build_client` would adopt it instead of
    /// generating a ciphersuite-correct one.
    fn mls_installed_leaf_keys(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.mls_installed_leaf_keys();
        }
        let guard = self.identity.read();
        let key = guard.as_ref()?;
        key.leaf_installed_by_mls().then(|| key.leaf_keys())
    }

    /// Whether a refresh-token delegate has been adopted, i.e. this is a *user*
    /// identity that can renew itself. Config uses this to catch, at
    /// construction, a stored login that installed an identity key but never got
    /// a usable refresh token (e.g. the IdP granted no `offline_access`) — such a
    /// provider would otherwise build fine and only fail at the first token
    /// fetch, behind a swallowed warning.
    pub fn has_refresh_delegate(&self) -> bool {
        self.refresh.read().is_some()
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

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let token = self
            .token
            .read()
            .as_ref()
            .filter(|entry| entry.expiry > now + REFRESH_BUFFER_SECONDS)
            .map(|entry| entry.token.clone())
            .ok_or(AuthError::GetTokenError)?;

        // Write lock: minting an attestation caches it, and the cache is what
        // keeps this to one ECDSA operation per TTL rather than one per call.
        let attestation = self
            .identity
            .write()
            .as_mut()
            .map(|k| k.attestation())
            .transpose()?;

        Ok(present_credential(&token, attestation.as_deref()))
    }

    fn get_id(&self) -> Result<String, AuthError> {
        let credential = self.get_token()?;
        // Parse the access token, not the attestation suffix.
        extract_sub_claim_unsafe(split_credential(&credential).0)
    }

    fn get_signature_keys(&self) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
        // Once a delegate exists it is the sole owner of the keys (see
        // `install_signature_keys`/`adopt_refresh_token`). Returns the MLS leaf
        // key: the identity key is never handed to `mls-rs`.
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.get_signature_keys();
        }
        self.identity
            .read()
            .as_ref()
            .map(|k| k.leaf_keys())
            .ok_or(AuthError::MlsNotSupported)
    }

    fn mls_signature_keys_installed(&self) -> bool {
        if let Some(delegate) = self.refresh.read().as_ref() {
            return delegate.mls_signature_keys_installed();
        }
        // Reports whether the *MLS layer* installed its key, not whether a key
        // exists — one is always seeded at construction so control messages can
        // be signed before any group exists. The MLS layer branches on this to
        // decide adopt-vs-generate (`crates/mls/src/mls.rs`), and it must
        // generate: the seeded key follows the compile-time curve, MLS's follows
        // the runtime ciphersuite.
        self.identity
            .read()
            .as_ref()
            .is_some_and(|k| k.leaf_installed_by_mls())
    }

    async fn set_signature_keys(
        &mut self,
        private_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<(), AuthError> {
        self.install_signature_keys(private_key, public_key)
    }

    fn export_identity(&self) -> Option<ExportedIdentity> {
        // The token and the identity key both come back from the credentials
        // store on restart, so only the MLS keypair needs persisting. This was
        // impossible while the credential bound the MLS key directly: a restored
        // key was, by construction, not the one the live token named.
        let (secret, public) = self.mls_installed_leaf_keys()?;
        Some(ExportedIdentity {
            // Informational: `sub` comes from the token, not the snapshot.
            id: self.get_id().unwrap_or_default(),
            credential: Vec::new(),
            signature_secret_key: secret,
            signature_public_key: public,
        })
    }

    fn with_restored_identity(self, identity: ExportedIdentity) -> Result<Self, AuthError> {
        // Reinstall the persisted MLS keypair so a restored group's signer still
        // matches its leaf. Marked as MLS-installed, so `build_client` adopts it
        // rather than generating a fresh one — the whole point of persisting it.
        //
        // The identity key is deliberately not restored: it belongs to the
        // credentials store, and a snapshot's copy could be from an older login
        // whose token is long gone.
        if identity.signature_secret_key.is_empty() || identity.signature_public_key.is_empty() {
            return Ok(self);
        }
        self.install_signature_keys(identity.signature_secret_key, identity.signature_public_key)?;
        Ok(self)
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

/// One JWKS fetch, with its keys already converted for verification.
///
/// `DecodingKey::from_jwk` re-parses the key material (an RSA modulus and
/// exponent, say) on every call, so preparing the keys once per fetch takes that
/// work off the per-message verification path. Held behind an `Arc` so a verify
/// takes a refcount rather than deep-cloning the whole key set.
struct VerificationKeys {
    /// The raw key set plus its `by_kid` index, `fetched_at` and `ttl`.
    cache: JwksCache,
    /// `kid` → prepared key. A key whose JWK could not be converted is absent
    /// here, and verification falls back to converting on demand so the original
    /// error still surfaces.
    prepared: HashMap<String, (DecodingKey, Algorithm)>,
    /// The lone key, when the set holds exactly one — the only thing a token
    /// with no `kid` may resolve to.
    single: Option<(DecodingKey, Algorithm)>,
}

impl VerificationKeys {
    fn new(jwks: JwkSet, fetched_at: Instant, ttl: Duration) -> Self {
        let cache = JwksCache::new(jwks, fetched_at, ttl);

        // Convert everything convertible now. Failures are dropped rather than
        // propagated: one unsupported key in a JWKS must not fail verification
        // for tokens signed with the others.
        let prepared = cache
            .by_kid
            .iter()
            .filter_map(|(kid, jwk)| {
                let key = DecodingKey::from_jwk(jwk).ok()?;
                Some((kid.clone(), (key, alg_from_jwk(jwk).ok()?)))
            })
            .collect();

        let single = match cache.jwks.keys.as_slice() {
            [only] => DecodingKey::from_jwk(only)
                .ok()
                .zip(alg_from_jwk(only).ok()),
            _ => None,
        };

        Self {
            cache,
            prepared,
            single,
        }
    }

    fn is_fresh(&self) -> bool {
        self.cache.fetched_at.elapsed() <= self.cache.ttl
    }

    /// The JWK a token header selects, with the same rules the prepared lookup
    /// uses: by `kid` if present, else the lone key.
    fn lookup_jwk(&self, header: &jsonwebtoken::Header) -> Result<&Jwk, AuthError> {
        match &header.kid {
            // `by_kid` keeps the first key per `kid`, matching a linear scan.
            Some(kid) => self
                .cache
                .by_kid
                .get(kid)
                .ok_or_else(|| AuthError::OidcKeyNotFound(kid.clone())),
            None => match self.cache.jwks.keys.as_slice() {
                [single] => Ok(single),
                _ => Err(AuthError::OidcMissingKidWithMultipleKeys),
            },
        }
    }
}

/// OIDC Token Verifier that validates JWTs using JWKS
#[derive(Clone)]
pub struct OidcVerifier {
    issuer_url: String,
    audience: String,
    keys: Arc<RwLock<Option<Arc<VerificationKeys>>>>,
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
            keys: Arc::new(RwLock::new(None)),
            // A bounded timeout matters here specifically for `revalidate`'s
            // retry loop — a hung request must not stall an attempt forever.
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("failed to build reqwest client"),
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
        let discovery_url = discovery_url(&issuer_parsed);
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

    /// Prepared verification keys, still-fresh from cache or newly fetched.
    async fn verification_keys(&self) -> Result<Arc<VerificationKeys>, AuthError> {
        if let Some(cached) = self.cached_verification_keys() {
            return Ok(cached);
        }

        let jwks = self.fetch_jwks().await?;
        let keys = Arc::new(VerificationKeys::new(jwks, Instant::now(), self.jwks_ttl));
        *self.keys.write() = Some(keys.clone());
        Ok(keys)
    }

    /// The cached keys if they are still fresh. Clones the `Arc` and releases the
    /// lock, so verification no longer runs with the read guard held — a refresh
    /// writer would otherwise wait on every in-flight signature check.
    fn cached_verification_keys(&self) -> Option<Arc<VerificationKeys>> {
        self.keys.read().as_ref().filter(|k| k.is_fresh()).cloned()
    }

    /// Verify a full credential (see [`DPOP_KEY_SEPARATOR`]) against JWKS. Any
    /// presented key is checked against `cnf.jkt` and surfaced as `pubkey`.
    fn verify_token_util(
        &self,
        credential: &str,
        keys: &VerificationKeys,
    ) -> Result<serde_json::Value, AuthError> {
        let (token, presented_key) = split_credential(credential);
        let header = decode_header(token)?;

        let prepared = match &header.kid {
            Some(kid) => keys.prepared.get(kid),
            None => keys.single.as_ref(),
        };

        // A key absent from `prepared` is one that failed to convert at fetch
        // time. Convert it again here so the caller sees why, rather than the
        // "key not found" a missing entry would otherwise imply.
        let converted_now;
        let (decoding_key, alg) = match prepared {
            Some((key, alg)) => (key, *alg),
            None => {
                let jwk = keys.lookup_jwk(&header)?;
                converted_now = (DecodingKey::from_jwk(jwk)?, alg_from_jwk(jwk)?);
                (&converted_now.0, converted_now.1)
            }
        };

        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[&self.issuer_url]);

        let token_data = decode::<serde_json::Value>(token, decoding_key, &validation)?;
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

    /// How long claims for this token may be cached: the configured TTL, but
    /// never past the token's own `exp`.
    ///
    /// A cache hit short-circuits verification, so an entry outliving the token
    /// would keep an expired token verifying for the rest of the TTL. `exp` is
    /// read from the already-verified claims, so a token without one (the
    /// validator rejects those by default) just gets the plain TTL.
    fn claim_cache_lifetime(&self, claims: &serde_json::Value) -> Duration {
        let Some(exp) = claims.get("exp").and_then(|e| e.as_u64()) else {
            return self.claim_cache_ttl;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Duration::from_secs(exp.saturating_sub(now)).min(self.claim_cache_ttl)
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

    /// Confirm the identity provider hasn't revoked this token, retrying a
    /// bounded number of times before giving up. Unlike [`Self::userinfo_claims`],
    /// this does not swallow the outcome: a definitive rejection (401/403) is
    /// [`AuthError::IdentityRevoked`], but anything that stops the check from
    /// completing at all — no endpoint discovered yet, a connection error, a
    /// timeout, a 5xx — is inconclusive and returns `Ok(())`, since evicting
    /// someone over a network blip would be worse than the check that never
    /// ran.
    async fn check_not_revoked(&self, credential: &str) -> Result<(), AuthError> {
        let (access_token, _) = split_credential(credential);

        let Some(endpoint) = self.userinfo_endpoint.get() else {
            tracing::debug!("revalidate: userinfo_endpoint not discovered yet, skipping");
            return Ok(());
        };

        for attempt in 1..=REVALIDATE_ATTEMPTS {
            match self
                .http_client
                .get(endpoint)
                .bearer_auth(access_token)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp)
                    if resp.status() == StatusCode::UNAUTHORIZED
                        || resp.status() == StatusCode::FORBIDDEN =>
                {
                    tracing::warn!(
                        status = %resp.status(),
                        "revalidate: identity provider rejected the token"
                    );
                    return Err(AuthError::IdentityRevoked);
                }
                Ok(resp) => {
                    tracing::debug!(
                        status = %resp.status(), attempt,
                        "revalidate: unexpected status, retrying"
                    );
                }
                Err(e) => {
                    tracing::debug!(error = %e, attempt, "revalidate: request failed, retrying");
                }
            }

            if attempt < REVALIDATE_ATTEMPTS {
                tokio::time::sleep(REVALIDATE_RETRY_DELAY).await;
            }
        }

        tracing::warn!("revalidate: identity provider unreachable after retries, not evicting");
        Ok(())
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

        let keys = self.verification_keys().await?;
        let (access_token, presented_key) = split_credential(token);
        let mut claims = self.verify_token_util(token, &keys)?;

        // A presented key means a DPoP identity credential, and the identity path
        // reads only `sub` and the `pubkey` that `bind_presented_key` already
        // derived from the verified `cnf.jkt` — so userinfo would cost a network
        // round trip per message for claims nothing downstream reads. The sync
        // `try_get_claims` path skips it for the same reason.
        //
        // Deliberately scoped to the DPoP case: plain bearer credentials still
        // fetch userinfo, because transport-auth policies (Rego/CEL) may read
        // claims like `groups` that only userinfo carries. This assumes a
        // `~`-suffixed credential never reaches a policy-evaluated path — true
        // today, as transport providers never get MLS keys installed.
        //
        // Note this leaves the DPoP path with no per-message IdP contact. That
        // costs nothing today: `userinfo_claims` swallows every error, so a 401
        // for a revoked token is already ignored. If it is ever made to fail
        // closed as a revocation check, revisit this — DPoP credentials would
        // otherwise be silently exempt.
        if presented_key.is_none() {
            // Bearer-auth call: the access token alone, never the key suffix.
            let extra = self.userinfo_claims(access_token).await;
            if let (Some(obj), Some(extra_obj)) = (claims.as_object_mut(), extra.as_object()) {
                for (k, v) in extra_obj {
                    obj.entry(k).or_insert_with(|| v.clone());
                }
            }

            // The merge can reintroduce a `pubkey` that `verify_token_util` stripped.
            strip_unverified_pubkey(&mut claims);
        }

        if let Some(cache) = &self.claim_cache {
            cache.write().insert(
                token.to_owned(),
                (
                    claims.clone(),
                    Instant::now() + self.claim_cache_lifetime(&claims),
                ),
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

    /// Confirm the token is still good *right now*, not just that it once
    /// verified. Unlike [`Self::verify`]/[`Self::get_claims`], this bypasses
    /// the claim cache and always contacts the identity provider — including
    /// for a DPoP-presented credential, which the per-message path
    /// deliberately skips (see the module-level comment on `verify_token`).
    /// Acceptable only because the caller is expected to call this rarely
    /// (e.g. once per participant per MLS epoch change), not per message.
    async fn revalidate(&self, token: impl AsRef<str> + Send) -> Result<(), AuthError> {
        let token = token.as_ref();

        // Signature/`exp`/DPoP-thumbprint problems are locally detectable and
        // not a network call — surface them as-is, no retry.
        let keys = self.verification_keys().await?;
        self.verify_token_util(token, &keys)?;

        self.check_not_revoked(token).await
    }

    fn try_verify(&self, token: impl AsRef<str>) -> Result<(), AuthError> {
        let keys = self
            .cached_verification_keys()
            .ok_or(AuthError::WouldBlockOn)?;
        self.verify_token_util(token.as_ref(), &keys)?;
        Ok(())
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
        let keys = self
            .cached_verification_keys()
            .ok_or(AuthError::WouldBlockOn)?;
        let claims = self.verify_token_util(token.as_ref(), &keys)?;

        // Populate as well as read. MLS `validate_member` runs on this path for
        // every message, so without this a configured `claim_cache_ttl` never
        // covers the hot path — it only ever hit for credentials an async
        // `get_claims` had already warmed. Same `exp` clamp as `verify_token`:
        // an entry must not outlive the token it came from.
        //
        // Only for DPoP credentials. `verify_token` merges userinfo into a plain
        // bearer credential's claims and this path cannot (userinfo needs async),
        // so caching a bearer entry here would serve userinfo-less claims to a
        // later `get_claims`. For a presented key both paths skip userinfo, so
        // the entries are identical and the cache is safe to share.
        if split_credential(token.as_ref()).1.is_some()
            && let Some(cache) = &self.claim_cache
        {
            cache.write().insert(
                token.as_ref().to_owned(),
                (
                    claims.clone(),
                    Instant::now() + self.claim_cache_lifetime(&claims),
                ),
            );
        }

        Ok(serde_json::from_value(claims)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Use the test utilities from the testutils module
    use slim_testing::utils::{TestClaims, setup_oidc_mock_server, setup_test_jwt_resolver};

    #[test]
    fn require_https_rejects_non_https_issuers() {
        assert!(require_https("https://idp.example.com/realms/slim").is_ok());
        for bad in [
            "http://idp.example.com/realms/slim",
            "http://127.0.0.1:8080/realms/slim",
            "http://localhost:8080/realms/slim",
        ] {
            assert!(
                matches!(require_https(bad), Err(AuthError::OidcInsecureIssuerUrl(_))),
                "should reject {bad}"
            );
        }
    }

    /// Building the discovery URL from the *parsed* issuer must produce exactly
    /// what formatting the raw string did, including the bare-origin case where
    /// `Url` adds a path slash and the case where the issuer already ends in one.
    #[test]
    fn discovery_url_matches_the_issuer_it_was_checked_from() {
        for issuer in [
            "https://idp.example.com/realms/slim",
            "https://idp.example.com/realms/slim/",
            "https://idp.example.com",
            "https://idp.example.com/",
        ] {
            let parsed = require_https(issuer).expect("issuer should be accepted");
            assert_eq!(
                discovery_url(&parsed),
                format!(
                    "{}/.well-known/openid-configuration",
                    issuer.trim_end_matches('/')
                ),
                "issuer {issuer}"
            );
        }
    }

    #[tokio::test]
    async fn test_oidc_token_provider_client_credentials_flow() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, expected_token) = setup_oidc_mock_server().await;
        let _guard = AllowInsecureIssuerForTest::new();

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
        let _guard = AllowInsecureIssuerForTest::new();

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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
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

    /// Builds a verifier + signed token against `setup_test_jwt_resolver`'s
    /// mock server, with `userinfo_endpoint` wired to that same server so
    /// `revalidate` has somewhere to call.
    async fn verifier_and_token_for_revalidate() -> (OidcVerifier, wiremock::MockServer, String) {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");
        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url.clone(), "test-audience");
        verifier
            .userinfo_endpoint
            .set(format!("{issuer_url}/userinfo"))
            .unwrap();

        (verifier, mock_server, token)
    }

    /// A definitive rejection from the identity provider must be reported as
    /// a confirmed revocation, not retried away.
    #[tokio::test]
    async fn revalidate_returns_identity_revoked_on_401() {
        slim_config::tls::provider::initialize_crypto_provider();
        let _guard = AllowInsecureIssuerForTest::new();
        let (verifier, mock_server, token) = verifier_and_token_for_revalidate().await;

        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        assert!(matches!(
            verifier.revalidate(token).await,
            Err(AuthError::IdentityRevoked)
        ));
    }

    /// Transient failures (5xx here) must be retried, and a subsequent
    /// success must not be reported as revoked.
    #[tokio::test]
    async fn revalidate_retries_transient_errors_then_succeeds() {
        slim_config::tls::provider::initialize_crypto_provider();
        let _guard = AllowInsecureIssuerForTest::new();
        let (verifier, mock_server, token) = verifier_and_token_for_revalidate().await;

        // First two attempts see a 5xx; only the third (lower priority,
        // unlimited) sees success.
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .with_priority(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .with_priority(2)
            .mount(&mock_server)
            .await;

        assert!(verifier.revalidate(token).await.is_ok());
    }

    /// An identity provider that can't be reached at all is inconclusive —
    /// evicting someone over a network blip would be worse than not checking,
    /// so this must fail open rather than report a revocation.
    #[tokio::test]
    async fn revalidate_fails_open_when_identity_provider_unreachable() {
        slim_config::tls::provider::initialize_crypto_provider();
        let _guard = AllowInsecureIssuerForTest::new();
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let issuer_url = mock_server.uri();

        let claims = TestClaims::new("user123", issuer_url.clone(), "test-audience");
        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let verifier = OidcVerifier::new(issuer_url.clone(), "test-audience");
        // Nothing listens here — every attempt fails to connect.
        verifier
            .userinfo_endpoint
            .set("http://127.0.0.1:1/userinfo".to_string())
            .unwrap();

        assert!(verifier.revalidate(token).await.is_ok());
    }

    #[tokio::test]
    async fn test_oidc_verifier_jwks_caching() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
        let issuer_url = mock_server.uri();

        let verifier = OidcVerifier::new(issuer_url, "test-audience");

        // Try to verify invalid token
        let result: Result<TestClaims, _> = verifier.get_claims("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oidc_verifier_missing_kid_single_key_works() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();

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

    /// A failed initial fetch leaves `token` at `None`; the background loop
    /// must keep retrying rather than getting stuck waiting for a `refresh_at`
    /// that was never set.
    #[test]
    fn background_refresh_retries_after_a_failed_initial_fetch() {
        let provider = provider_for("https://idp.example");
        let now = 1_000;

        assert!(provider.needs_background_refresh(now));

        *provider.token.write() = Some(TokenCacheEntry {
            token: "t".to_string(),
            expiry: now + 3600,
            refresh_at: now + 2400,
        });
        assert!(!provider.needs_background_refresh(now));
        assert!(provider.needs_background_refresh(now + 2400));
    }

    #[tokio::test]
    async fn test_oidc_token_provider_error_handling() {
        // Initialize crypto provider for tests
        slim_config::tls::provider::initialize_crypto_provider();

        let mock_server = MockServer::start().await;
        let _guard = AllowInsecureIssuerForTest::new();
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
        let token = Arc::new(RwLock::new(None));
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
            token: token.clone(),
            client: http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            identity: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AsyncMutex::new(false)),
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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let token = Arc::new(RwLock::new(None));

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
            token: token.clone(),
            client: http_client,
            shutdown_tx: Arc::new(shutdown_tx),
            refresh_task: Arc::new(parking_lot::Mutex::new(None)),
            identity: Arc::new(RwLock::new(None)),
            refresh: Arc::new(RwLock::new(None)),
            delegate_started: Arc::new(AsyncMutex::new(false)),
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
        let _guard = AllowInsecureIssuerForTest::new();
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
        let _guard = AllowInsecureIssuerForTest::new();
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

        // Populate the key cache so try_verify can use it synchronously
        let _keys = verifier.verification_keys().await.unwrap();

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
    /// Keys are prepared eagerly per fetch, so one key that cannot be converted
    /// must not take the rest of the set with it — tokens signed by a sibling key
    /// still have to verify. It also has to stay reachable for error reporting,
    /// or selecting it would report "key not found" instead of why it is unusable.
    #[test]
    fn unconvertible_key_does_not_block_the_rest_of_the_set() {
        let with_kid = |kid: &str| jsonwebtoken::Header {
            kid: Some(kid.to_owned()),
            ..jsonwebtoken::Header::new(Algorithm::RS256)
        };

        // A usable RSA key (the RFC 7638 §3.1 example) alongside a symmetric key,
        // which has no place in a public JWKS and cannot be prepared.
        let jwks: JwkSet = serde_json::from_value(json!({
            "keys": [
                {
                    "kty": "RSA",
                    "kid": "rsa-1",
                    "alg": "RS256",
                    "e": "AQAB",
                    "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw"
                },
                { "kty": "oct", "kid": "sym-1", "k": "c2VjcmV0" }
            ]
        }))
        .unwrap();

        let keys = VerificationKeys::new(jwks, Instant::now(), Duration::from_secs(3600));

        assert!(
            keys.prepared.contains_key("rsa-1"),
            "the usable key must be prepared"
        );
        assert!(
            !keys.prepared.contains_key("sym-1"),
            "a symmetric key must never be prepared"
        );
        // Two keys in the set, so a token with no `kid` resolves to nothing.
        assert!(keys.single.is_none());

        // Still reachable, so verification reports why it is unusable...
        assert!(keys.lookup_jwk(&with_kid("sym-1")).is_ok());
        // ...while a genuinely absent `kid` is still a missing key.
        assert!(matches!(
            keys.lookup_jwk(&with_kid("absent")),
            Err(AuthError::OidcKeyNotFound(_))
        ));
    }

    /// A claim-cache hit short-circuits verification, so an entry must never
    /// outlive the token it was built from — otherwise a long `claim_cache_ttl`
    /// keeps an expired token verifying. Guards the clamp in
    /// `claim_cache_lifetime`, which a bare `Instant::now() + ttl` would drop.
    #[test]
    fn claim_cache_entry_never_outlives_the_token() {
        slim_config::tls::provider::initialize_crypto_provider();
        let verifier = OidcVerifier::new("https://idp.example.com", "aud")
            .with_claim_cache(Duration::from_secs(3600));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expiring well inside the TTL: the token's own exp must win.
        let soon = json!({ "sub": "u", "exp": now + 30 });
        let lifetime = verifier.claim_cache_lifetime(&soon);
        assert!(
            lifetime <= Duration::from_secs(30),
            "cache entry outlives the token: {lifetime:?}"
        );

        // Token outliving the TTL: the configured TTL is the cap.
        let distant = json!({ "sub": "u", "exp": now + 86_400 });
        assert_eq!(
            verifier.claim_cache_lifetime(&distant),
            Duration::from_secs(3600)
        );

        // Already expired: never cache it at all.
        let expired = json!({ "sub": "u", "exp": now - 1 });
        assert_eq!(verifier.claim_cache_lifetime(&expired), Duration::ZERO);

        // No exp to clamp against: fall back to the plain TTL.
        let no_exp = json!({ "sub": "u" });
        assert_eq!(
            verifier.claim_cache_lifetime(&no_exp),
            Duration::from_secs(3600)
        );
    }

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
        let pop = crate::utils::generate_identity_signature_keys().unwrap();
        let (_, leaf_public) = crate::utils::generate_mls_signature_keys().unwrap();
        let mut claims = json!({
            "sub": "user",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&pop.1).unwrap() },
        });
        let attestation = crate::dpop::build_key_attestation(&pop.0, &pop.1, &leaf_public).unwrap();
        bind_presented_key(&mut claims, &attestation).unwrap();

        let parsed = crate::identity_claims::IdentityClaims::from_json(&claims).unwrap();
        assert_eq!(parsed.public_key, BASE64_STD.encode(&leaf_public));
    }

    /// The thumbprint gate lives on the *identity* key, where `cnf.jkt` still
    /// cannot be re-bound locally: a store whose identity key is not the one the
    /// live token names came from a different login, and every peer would reject
    /// the pair.
    #[tokio::test]
    async fn install_identity_keys_refuses_a_key_the_live_token_is_not_bound_to() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        let access_token = bound_token(&pop.1);

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token, "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        let other = identity_keys();
        assert!(matches!(
            provider.install_identity_keys(other.0, other.1),
            Err(AuthError::DpopThumbprintMismatch)
        ));

        // MLS has since installed its leaf key.
        let leaf = identity_keys();
        provider
            .install_signature_keys(leaf.0.clone(), leaf.1.clone())
            .unwrap();

        // Re-installing the same key is not a change and stays allowed.
        // Asserted without formatting the operands: a failing test must not
        // print private key bytes into CI output.
        assert!(
            provider.install_identity_keys(pop.0, pop.1).is_ok(),
            "re-installing the key the live token is bound to must be allowed"
        );

        // ...and must not discard the leaf key already installed under it.
        assert!(provider.mls_signature_keys_installed());
        assert_eq!(provider.get_signature_keys().unwrap(), leaf);
    }

    /// The regression test for the attestation chain: rotating the MLS key is a
    /// new attestation, not a new login. Every one of these installs returned
    /// `DpopThumbprintMismatch` when the credential bound the MLS key directly.
    #[tokio::test]
    async fn mls_key_rotation_needs_no_new_login() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        let access_token = bound_token(&pop.1);

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token.clone(), "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        // Three successive rotations, as `rotate_identity_key` would drive them.
        for _ in 0..3 {
            let (leaf_secret, leaf_public) = leaf_keys();
            provider
                .set_signature_keys(leaf_secret, leaf_public.clone())
                .await
                .expect("an MLS rotation must not need a new login");

            // ...and the credential now attests the key just installed.
            let credential = provider.get_token().unwrap();
            let (access, attestation) = split_credential(&credential);
            assert_eq!(access, access_token);

            let mut claims = claims_bound_to(&pop);
            bind_presented_key(&mut claims, attestation.expect("attestation")).unwrap();
            assert_eq!(
                claims["pubkey"].as_str().unwrap(),
                BASE64_STD.encode(&leaf_public),
                "the credential must attest the freshly rotated key"
            );
        }
    }

    /// An MLS ciphersuite whose keys have no JOSE mapping is fine for the leaf
    /// key: it rides inside the attestation payload as opaque bytes. Under the
    /// single-key model this combination could not be expressed at all.
    #[tokio::test]
    async fn leaf_key_needs_no_jose_mapping() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": bound_token(&pop.1), "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        // 48 bytes: not a length `KeyCurve` recognises, so `jwk_thumbprint`
        // would reject it. Only the identity key has to be JOSE-expressible.
        let odd_leaf = vec![7u8; 48];
        provider
            .set_signature_keys(vec![9u8; 48], odd_leaf.clone())
            .await
            .expect("a leaf key with no JOSE mapping must still be attestable");

        let credential = provider.get_token().unwrap();
        let mut claims = claims_bound_to(&pop);
        bind_presented_key(&mut claims, split_credential(&credential).1.unwrap()).unwrap();
        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&odd_leaf)
        );
    }

    /// A leaf key with nothing to vouch for it must fail where the fix is
    /// obvious, not at a peer's thumbprint comparison.
    #[tokio::test]
    async fn leaf_key_without_an_identity_key_is_refused() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (_server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let mut provider = provider_for(&issuer);
        let (leaf_secret, leaf_public) = leaf_keys();
        assert!(matches!(
            provider.set_signature_keys(leaf_secret, leaf_public).await,
            Err(AuthError::AttestationNoIdentityKey)
        ));
    }

    /// Once a delegate exists it is the sole owner of the key: installing after
    /// adoption must land only there, and queries must reflect it — no second,
    /// independently-readable copy left behind to drift out of sync.
    #[test]
    fn signature_keys_have_a_single_owner_once_delegated() {
        slim_config::tls::provider::initialize_crypto_provider();
        let provider = provider_for("https://idp.example.com");

        // The order `apply_stored_credentials` uses: identity key first, then
        // the refresh token, which moves the key into the delegate.
        let pop = identity_keys();
        provider.install_identity_keys(pop.0, pop.1).unwrap();
        provider.adopt_refresh_token("rt", None, None).unwrap();

        assert!(!provider.mls_signature_keys_installed());

        let (secret, public) = crate::utils::generate_mls_signature_keys().unwrap();
        provider
            .install_signature_keys(secret.clone(), public.clone())
            .unwrap();

        assert!(provider.mls_signature_keys_installed());
        // `==` rather than assert_eq!: the latter Debug-formats both sides on
        // failure, which would put the private key in the panic message.
        assert!(
            provider.get_signature_keys().unwrap() == (secret, public),
            "installed signature keys must round-trip unchanged"
        );
    }

    /// On this grant it means a stale code, not a spent refresh token — the two
    /// need different recovery.
    #[tokio::test]
    async fn invalid_grant_on_auth_code_is_not_reported_as_revoked_refresh() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant", "error_description": "Code not valid",
            })))
            .mount(&server)
            .await;

        // Matched without formatting the unexpected value: a failing test must
        // not print a token or key material into CI output.
        let Err(AuthError::TokenEndpointError { body, .. }) = provider_for(&issuer)
            .exchange_authorization_code("stale", "v", "http://127.0.0.1/cb")
            .await
        else {
            panic!("expected TokenEndpointError");
        };
        assert!(body.contains("invalid_grant"), "got {body}");
        assert!(
            body.contains("Code not valid"),
            "description dropped: {body}"
        );
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
        let pop = identity_keys();
        let (_, leaf_public) = leaf_keys();

        let mut claims = claims_bound_to(&pop);
        bind_presented_key(&mut claims, &attest(&pop, &leaf_public)).unwrap();

        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&leaf_public),
            "attested key must be surfaced in the encoding IdentityClaims expects"
        );
    }

    /// The forgery the whole design exists to stop. Every group member holds the
    /// ratchet tree, so an attacker has the victim's token verbatim; she signs a
    /// perfectly valid attestation over her own MLS key, with her own identity
    /// key, and staples it to the victim's token.
    #[test]
    fn bind_presented_key_rejects_an_attestation_from_another_identity_key() {
        let victim = identity_keys();
        let attacker = identity_keys();
        let (_, attacker_leaf) = leaf_keys();

        let mut claims = json!({
            "sub": "victim",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&victim.1).unwrap() }
        });

        // Self-consistent: it verifies under the key it advertises. What it
        // cannot do is be the key the IdP named for this subject.
        let forged = attest(&attacker, &attacker_leaf);
        let result = bind_presented_key(&mut claims, &forged);
        assert!(matches!(result, Err(AuthError::DpopThumbprintMismatch)));
        assert!(
            claims.get("pubkey").is_none(),
            "a rejected attestation must never leave a pubkey claim behind"
        );
    }

    /// The same forgery from the other direction: the attacker keeps the
    /// victim's identity key in the header — so the thumbprint matches — but has
    /// no private half to sign a payload naming her own key.
    #[test]
    fn bind_presented_key_rejects_an_attestation_signed_by_the_wrong_key() {
        let victim = identity_keys();
        let attacker = identity_keys();
        let (_, attacker_leaf) = leaf_keys();

        // Header advertises the victim's key, signature is the attacker's.
        let genuine = attest(&victim, &attacker_leaf);
        let forged_sig = attest(&attacker, &attacker_leaf);
        let mut parts = genuine.split('.');
        let header = parts.next().unwrap();
        let payload = parts.next().unwrap();
        let stolen_signature = forged_sig.split('.').nth(2).unwrap();
        let spliced = format!("{header}.{payload}.{stolen_signature}");

        let mut claims = claims_bound_to(&victim);
        assert!(matches!(
            bind_presented_key(&mut claims, &spliced),
            Err(AuthError::AttestationSignatureInvalid)
        ));
    }

    /// No `cnf` means nothing to bind against; accepting would trust the key on
    /// the holder's say-so.
    #[test]
    fn bind_presented_key_rejects_token_without_confirmation_claim() {
        let pop = identity_keys();
        let (_, leaf_public) = leaf_keys();
        let mut claims = json!({ "sub": "user-id" });

        let result = bind_presented_key(&mut claims, &attest(&pop, &leaf_public));
        assert!(matches!(result, Err(AuthError::DpopMissingConfirmation)));
    }

    /// Attacker-controlled input reaching this before any check has passed.
    #[test]
    fn bind_presented_key_rejects_malformed_attestation() {
        for garbage in ["", "not-a-jws", "a.b", "a.b.c.d", "not!base64.x.y"] {
            let mut claims = json!({ "sub": "user-id", "cnf": { "jkt": "whatever" } });
            assert!(
                bind_presented_key(&mut claims, garbage).is_err(),
                "accepted {garbage:?}"
            );
            assert!(claims.get("pubkey").is_none());
        }
    }

    /// A DPoP proof is a JWS with a `jwk` header over the same key, so without
    /// the `typ` check one captured from a token request would pass as an
    /// attestation — and its payload names no key at all.
    #[test]
    fn bind_presented_key_rejects_a_dpop_proof_replayed_as_an_attestation() {
        let pop = identity_keys();
        let proof =
            crate::dpop::build_proof(&pop.0, &pop.1, "POST", "https://idp/token", None).unwrap();

        let mut claims = claims_bound_to(&pop);
        assert!(matches!(
            bind_presented_key(&mut claims, &proof),
            Err(AuthError::AttestationMalformed(_))
        ));
    }

    /// What `get_token` emits must be what `bind_presented_key` accepts.
    #[tokio::test]
    async fn provider_credential_round_trips_through_binding() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        let (leaf_secret, leaf_public) = leaf_keys();
        let access_token = bound_token(&pop.1);

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token, "token_type": "Bearer", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider.fetch_new_token().await.unwrap();
        // A leaf key is seeded at construction, so the credential already
        // attests one — control messages must be signable before any MLS group
        // exists. What is *not* yet true is that the MLS layer chose it.
        assert!(split_credential(&provider.get_token().unwrap()).1.is_some());
        assert!(!provider.mls_signature_keys_installed());

        provider
            .set_signature_keys(leaf_secret, leaf_public.clone())
            .await
            .unwrap();
        assert!(provider.mls_signature_keys_installed());

        let credential = provider.get_token().unwrap();
        let (token, attestation) = split_credential(&credential);
        assert_eq!(token, access_token);

        let mut claims = claims_bound_to(&pop);
        bind_presented_key(&mut claims, attestation.unwrap()).unwrap();
        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&leaf_public)
        );
    }

    /// The *only* path MLS has, since `validate_member` is sync. `WouldBlockOn`
    /// here fails every member of an OIDC-backed group.
    #[tokio::test]
    async fn try_get_claims_verifies_from_cached_jwks_without_a_claim_cache() {
        let (private_key, mock_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let _guard = AllowInsecureIssuerForTest::new();
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

    /// Refused at install time, not on every later signing attempt. Applies to
    /// the identity key only — it is the one that has to become a JWK.
    #[tokio::test]
    async fn install_identity_keys_rejects_unmappable_key_type() {
        slim_config::tls::provider::initialize_crypto_provider();

        let (_mock_server, issuer_url, _) = setup_oidc_mock_server().await;
        let _guard = AllowInsecureIssuerForTest::new();
        let provider = OidcTokenProvider::new(OidcProviderConfig {
            client_id: "c".to_string(),
            client_secret: "s".to_string(),
            issuer_url,
            scope: None,
            timeout: None,
        })
        .unwrap();

        let result = provider.install_identity_keys(vec![0u8; 32], vec![0u8; 20]);
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

    /// `(secret, public)` for a DPoP-bound identity key (`K_pop`) — always
    /// P-256, whatever the MLS ciphersuite is.
    fn identity_keys() -> (Vec<u8>, Vec<u8>) {
        crate::utils::generate_identity_signature_keys().unwrap()
    }

    /// `(secret, public)` for an MLS leaf key (`K_leaf`), as `mls-rs` hands one
    /// over through `set_signature_keys`.
    fn leaf_keys() -> (Vec<u8>, Vec<u8>) {
        crate::utils::generate_mls_signature_keys().unwrap()
    }

    /// An attestation of `leaf_public` signed by `pop`, exactly as a peer
    /// receives it in the credential.
    fn attest(pop: &(Vec<u8>, Vec<u8>), leaf_public: &[u8]) -> String {
        crate::dpop::build_key_attestation(&pop.0, &pop.1, leaf_public).unwrap()
    }

    /// Claims as a peer's verifier sees them for a token bound to `pop`.
    fn claims_bound_to(pop: &(Vec<u8>, Vec<u8>)) -> serde_json::Value {
        json!({
            "sub": "user-id",
            "cnf": { "jkt": crate::dpop::jwk_thumbprint(&pop.1).unwrap() },
        })
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

    /// Without a proof signed by the identity key the IdP has nothing to bind.
    #[tokio::test]
    async fn authorization_code_exchange_sends_dpop_proof() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        let (leaf_secret, leaf_public) = leaf_keys();
        let access_token = bound_token(&pop.1);

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
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();

        let token = provider
            .exchange_authorization_code("the-code", "the-verifier", "http://127.0.0.1:1234/cb")
            .await
            .unwrap();
        assert_eq!(token, access_token);

        // The credential handed onward attests the MLS key under the identity key
        // the proof was signed with.
        provider
            .set_signature_keys(leaf_secret, leaf_public.clone())
            .await
            .unwrap();
        let credential = provider.get_token().unwrap();
        let (access, attestation) = split_credential(&credential);
        assert_eq!(access, access_token);

        let attested = crate::dpop::verify_key_attestation(attestation.unwrap()).unwrap();
        assert_eq!(attested.leaf_public_key, leaf_public);
        assert_eq!(
            attested.signer_jkt,
            crate::dpop::jwk_thumbprint(&pop.1).unwrap()
        );
    }

    // ---------------------------------------------------------------------
    // Snapshot round-trip
    // ---------------------------------------------------------------------

    /// A restarted app must sign with the key its persisted group already has in
    /// its leaf. Impossible while the credential bound the MLS key directly: a
    /// restored key was, by construction, not the one the live token named.
    #[tokio::test]
    async fn snapshot_round_trips_the_mls_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": bound_token(&pop.1),
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider.fetch_new_token().await.unwrap();

        // Nothing to persist before MLS installs its own key: exporting the pair
        // seeded at construction would come back looking like MLS's choice.
        assert!(
            provider.export_identity().is_none(),
            "the construction-time placeholder must never be persisted"
        );

        let (leaf_secret, leaf_public) = leaf_keys();
        provider
            .set_signature_keys(leaf_secret.clone(), leaf_public.clone())
            .await
            .unwrap();

        let exported = provider
            .export_identity()
            .expect("an MLS-installed key must be persisted");
        assert!(
            exported.credential.is_empty(),
            "the token comes back from the credentials store, not the snapshot"
        );
        // `==` rather than assert_eq!: the latter Debug-formats both sides on
        // failure, which would put the private key in the panic message.
        assert!(
            exported.signature_secret_key == leaf_secret
                && exported.signature_public_key == leaf_public,
            "the exported keypair must be the one MLS installed"
        );

        // Restart: a fresh provider, seeded from the same store, then restored.
        let fresh = provider_for(&issuer);
        fresh
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        assert!(!fresh.mls_signature_keys_installed());

        let restored = fresh.with_restored_identity(exported).unwrap();
        assert!(
            restored.mls_signature_keys_installed(),
            "a restored key must look installed, or build_client generates a new \
             one and the persisted group's leaf no longer matches its signer"
        );
        assert!(restored.get_signature_keys().unwrap() == (leaf_secret, leaf_public));
    }

    /// A snapshot with no keys in it is a no-op, not a failure.
    #[tokio::test]
    async fn an_empty_snapshot_leaves_the_provider_alone() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (_server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let pop = identity_keys();
        let provider = provider_for(&issuer);
        provider.install_identity_keys(pop.0, pop.1).unwrap();

        let restored = provider
            .with_restored_identity(ExportedIdentity {
                id: String::new(),
                credential: Vec::new(),
                signature_secret_key: Vec::new(),
                signature_public_key: Vec::new(),
            })
            .unwrap();
        assert!(!restored.mls_signature_keys_installed());
    }

    // ---------------------------------------------------------------------
    // Full chain, with real signature verification at every link
    // ---------------------------------------------------------------------

    /// Claims as a DPoP-enabled IdP issues them, so the test can sign a real
    /// token carrying `cnf.jkt` with the mock JWKS key.
    #[derive(serde::Serialize)]
    struct BoundTokenClaims {
        sub: String,
        iss: String,
        aud: String,
        exp: u64,
        iat: u64,
        cnf: serde_json::Value,
    }

    /// A provider serving a token that is genuinely signed by `jwks_server`'s
    /// key and bound to `pop`, plus a verifier that fetches that JWKS. Nothing
    /// between the two is stubbed.
    /// The mock servers ride along in the return value: dropping a `MockServer`
    /// stops it, so the caller has to hold them for the length of the test.
    async fn bound_provider_and_verifier(
        pop: &(Vec<u8>, Vec<u8>),
    ) -> (
        OidcTokenProvider,
        OidcVerifier,
        String,
        (MockServer, MockServer),
    ) {
        slim_config::tls::provider::initialize_crypto_provider();

        let (private_key, jwks_server, _alg) = setup_test_jwt_resolver(Algorithm::RS256).await;
        let idp_issuer = jwks_server.uri();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = BoundTokenClaims {
            sub: "alice".to_string(),
            iss: idp_issuer.clone(),
            aud: "test-audience".to_string(),
            exp: now + 3600,
            iat: now,
            cnf: json!({ "jkt": crate::dpop::jwk_thumbprint(&pop.1).unwrap() }),
        };
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let access_token = encode(&Header::new(Algorithm::RS256), &claims, &encoding_key).unwrap();

        // A second server stands in for the token endpoint: the provider never
        // verifies what it is handed, so it needs no relationship to the JWKS.
        let (token_server, token_issuer) = mock_issuer().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token.clone(),
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&token_server)
            .await;

        let provider = provider_for(&token_issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider.fetch_new_token().await.unwrap();

        let verifier = OidcVerifier::new(idp_issuer, "test-audience");
        (
            provider,
            verifier,
            access_token,
            (jwks_server, token_server),
        )
    }

    /// End to end through every link, three rotations deep: an IdP-signed token
    /// verified against a real JWKS, an attestation minted by the provider, and
    /// `OidcVerifier` resolving it to the key the MLS layer installed — which
    /// `IdentityClaims` then hands to `validate_member`.
    ///
    /// Under the single-key model each rotation failed at the *first* step, in
    /// `set_signature_keys`.
    #[tokio::test]
    async fn full_chain_survives_mls_key_rotation() {
        let _guard = AllowInsecureIssuerForTest::new();
        let pop = identity_keys();
        let (mut provider, verifier, access_token, _servers) =
            bound_provider_and_verifier(&pop).await;

        for round in 0..3 {
            let (leaf_secret, leaf_public) = leaf_keys();
            provider
                .set_signature_keys(leaf_secret, leaf_public.clone())
                .await
                .unwrap_or_else(|e| panic!("rotation {round} refused: {e}"));

            let credential = provider.get_token().unwrap();
            assert_eq!(split_credential(&credential).0, access_token);

            // Real JWKS verification, real attestation verification.
            let claims: serde_json::Value =
                Verifier::get_claims(&verifier, &credential).await.unwrap();
            let parsed = crate::identity_claims::IdentityClaims::from_json(&claims).unwrap();

            assert_eq!(parsed.subject, "alice", "the person must not change");
            assert_eq!(
                parsed.public_key,
                BASE64_STD.encode(&leaf_public),
                "rotation {round}: the verifier must resolve the newly installed key"
            );
        }
    }

    /// The same chain, attacked: an authenticated group member holds the
    /// victim's credential verbatim and swaps in an attestation over her own MLS
    /// key, signed by her own identity key. Rejected by the real verifier.
    #[tokio::test]
    async fn full_chain_rejects_a_credential_re_attested_by_an_onlooker() {
        let _guard = AllowInsecureIssuerForTest::new();
        let victim = identity_keys();
        let (mut provider, verifier, access_token, _servers) =
            bound_provider_and_verifier(&victim).await;

        let (leaf_secret, leaf_public) = leaf_keys();
        provider
            .set_signature_keys(leaf_secret, leaf_public)
            .await
            .unwrap();
        // Precondition: the genuine credential verifies.
        let genuine = provider.get_token().unwrap();
        Verifier::get_claims::<serde_json::Value>(&verifier, &genuine)
            .await
            .expect("the genuine credential must verify");

        let attacker = identity_keys();
        let (_, attacker_leaf) = leaf_keys();
        let forged = format!("{access_token}~{}", attest(&attacker, &attacker_leaf));

        assert!(matches!(
            Verifier::get_claims::<serde_json::Value>(&verifier, &forged).await,
            Err(AuthError::DpopThumbprintMismatch)
        ));
    }

    /// A response that carries a refresh token must never populate the
    /// client-credentials cache — `get_token` never reads it again once a
    /// delegate exists, so writing there is a wasted store on every exchange.
    #[tokio::test]
    async fn authorization_code_response_with_refresh_token_never_touches_the_service_cache() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "delegated", "refresh_token": "rt", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await
            .unwrap();

        assert!(
            provider.token.read().is_none(),
            "the service cache must stay empty once a delegate owns renewal"
        );
    }

    /// MLS installs its own pair whenever none is present. Pairing that with an
    /// unbound token would be rejected by every peer, on every message.
    #[tokio::test]
    async fn unbound_token_is_served_without_a_presented_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

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
        let pop = identity_keys();
        provider.install_identity_keys(pop.0, pop.1).unwrap();
        let (leaf_secret, leaf_public) = leaf_keys();
        provider
            .set_signature_keys(leaf_secret, leaf_public)
            .await
            .unwrap();
        provider.fetch_new_token().await.unwrap();

        let credential = provider.get_token().unwrap();
        assert_eq!(
            split_credential(&credential).1,
            None,
            "an unbound token has no cnf.jkt to name the signer, so presenting an \
             attestation would only fail obscurely at every peer"
        );
    }

    /// A structurally-present but unverifiable header is worthless.
    #[tokio::test]
    async fn dpop_proof_binds_the_token_request() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at", "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        let pop = identity_keys();
        let public = pop.1.clone();
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
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
        // Advertised key is the identity key the token will be bound to — never
        // the MLS key, which must not reach the token endpoint.
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
        let _guard = AllowInsecureIssuerForTest::new();

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

        let provider = provider_for(&issuer);
        let pop = identity_keys();
        provider.install_identity_keys(pop.0, pop.1).unwrap();

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

    /// A second consecutive `use_dpop_nonce` challenge means a misbehaving
    /// endpoint, not a race — it must be reported clearly, not folded into the
    /// generic `use_dpop_nonce: <description>` error.
    #[tokio::test]
    async fn second_consecutive_dpop_nonce_challenge_is_reported_clearly() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("DPoP-Nonce", "server-nonce-value")
                    .set_body_json(json!({ "error": "use_dpop_nonce" })),
            )
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        let pop = identity_keys();
        provider.install_identity_keys(pop.0, pop.1).unwrap();

        let result = provider
            .exchange_authorization_code("c", "v", "http://127.0.0.1/cb")
            .await;

        // Matched without formatting the unexpected value: a failing test must
        // not print a token or key material into CI output.
        let Err(AuthError::TokenEndpointError { body, .. }) = result else {
            panic!("expected a clear misbehaving-endpoint error");
        };
        assert!(body.contains("kept demanding a new DPoP nonce"), "{body}");
    }

    /// A spent refresh token must surface as its own error so the caller can
    /// Must surface distinctly so the caller re-logins rather than retrying.
    #[tokio::test]
    async fn invalid_grant_on_refresh_maps_to_refresh_token_revoked() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

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

    /// Re-proving the identity key keeps `cnf.jkt` unchanged across renewal, so
    /// attestations minted before and after a refresh are equally valid.
    #[tokio::test]
    async fn refresh_reproves_the_same_identity_key() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();
        let pop = identity_keys();
        let (leaf_secret, leaf_public) = leaf_keys();

        // The DPoP header is required by the mock: a renewal that dropped the
        // proof would return an unbound token and silently break the identity.
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "refresh_token=rt-1",
            ))
            .and(wiremock::matchers::header_exists("DPoP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": bound_token(&pop.1),
                "refresh_token": "rt-2",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let mut provider = provider_for(&issuer);
        provider
            .install_identity_keys(pop.0.clone(), pop.1.clone())
            .unwrap();
        provider.adopt_refresh_token("rt-1", None, None).unwrap();
        // Installed after adoption, so it lands in the delegate that now owns
        // the identity key — the MLS layer does not know a delegate exists.
        provider
            .set_signature_keys(leaf_secret, leaf_public.clone())
            .await
            .unwrap();

        TokenProvider::initialize(&mut provider).await.unwrap();

        // Same identity key re-proved, so the thumbprint the IdP bound still
        // matches and the attestation over the MLS key still verifies.
        let credential = provider.get_token().unwrap();
        let (access, attestation) = split_credential(&credential);
        assert_eq!(access, bound_token(&pop.1));

        let mut claims = claims_bound_to(&pop);
        bind_presented_key(&mut claims, attestation.unwrap()).unwrap();
        assert_eq!(
            claims["pubkey"].as_str().unwrap(),
            BASE64_STD.encode(&leaf_public)
        );
    }

    /// Client credentials would swap in the service account's `sub` and break
    /// every MLS binding made under the user's identity.
    #[tokio::test]
    async fn adopted_refresh_token_renews_with_the_refresh_grant() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

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
        let _guard = AllowInsecureIssuerForTest::new();

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
        *provider.token.write() = Some(TokenCacheEntry {
            token: "service-token".to_string(),
            expiry: u64::MAX,
            refresh_at: u64::MAX,
        });
        assert_eq!(provider.get_token().unwrap(), "service-token");

        // Once a refresh token is adopted, the delegate's cache is the only one read.
        provider.adopt_refresh_token("rt", None, None).unwrap();
        TokenProvider::initialize(&mut provider).await.unwrap();
        assert_eq!(provider.get_token().unwrap(), "user-token");
    }

    /// A client-credentials background task started by an earlier `initialize()`
    /// must not keep renewing the service token forever after a refresh token
    /// is later adopted and this provider becomes a user identity.
    #[tokio::test]
    async fn adopting_a_refresh_token_retires_the_client_credentials_background_task() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (_server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        let mut provider = provider_for(&issuer);
        TokenProvider::initialize(&mut provider).await.unwrap();
        assert!(
            provider.refresh_task.lock().is_some(),
            "initialize() must start the client-credentials background task"
        );

        provider.adopt_refresh_token("rt", None, None).unwrap();

        assert!(
            provider.refresh_task.lock().is_none(),
            "adopting a refresh token must retire the client-credentials background task"
        );
    }

    /// A second concurrent `initialize()` must wait for an in-flight delegate
    /// fetch, not report success before a token actually exists.
    #[tokio::test]
    async fn concurrent_initialize_waits_for_the_in_flight_delegate_fetch() {
        slim_config::tls::provider::initialize_crypto_provider();
        let (server, issuer) = mock_issuer().await;
        let _guard = AllowInsecureIssuerForTest::new();

        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "grant_type=refresh_token",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(150))
                    .set_body_json(json!({ "access_token": "delegated", "expires_in": 3600 })),
            )
            .mount(&server)
            .await;

        let provider = provider_for(&issuer);
        provider.adopt_refresh_token("rt", None, None).unwrap();

        let mut first = provider.clone();
        let first_task = tokio::spawn(async move { TokenProvider::initialize(&mut first).await });

        // Give the first call time to acquire the lock and start its (slow) fetch.
        tokio::time::sleep(Duration::from_millis(30)).await;

        let mut second = provider.clone();
        TokenProvider::initialize(&mut second)
            .await
            .expect("second initialize must succeed once it stops waiting");

        // The second call only returns once the delegate actually has a token.
        assert_eq!(provider.get_token().unwrap(), "delegated");

        first_task.await.unwrap().unwrap();
    }
}

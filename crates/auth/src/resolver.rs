// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, KeyAlgorithm};
use jsonwebtoken::{
    Algorithm, DecodingKey, Header,
    jwk::{Jwk, JwkSet},
};
use parking_lot::RwLock;
use reqwest::{Client as ReqwestClient, StatusCode};
use url::Url;

use crate::errors::AuthError;
use crate::key_refresh::RefreshTask;

/// True iff `candidate` shares the same scheme, host, and port as `base`.
pub(crate) fn same_origin(base: &Url, candidate: &Url) -> bool {
    base.scheme() == candidate.scheme()
        && base.host() == candidate.host()
        && base.port_or_known_default() == candidate.port_or_known_default()
}

/// Returns an error if `url` does not use `https`, unless the host is localhost.
pub(crate) fn require_https(url: &str) -> Result<Url, AuthError> {
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

/// The peer a static trust-list key belongs to: its `kid`, and the `did:key`
/// its public key encodes (Ed25519 keys only).
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyOwner {
    kid: Option<String>,
    did: Option<String>,
}

impl KeyOwner {
    fn of(jwk: &Jwk) -> Self {
        let did = match &jwk.algorithm {
            AlgorithmParameters::OctetKeyPair(okp) if okp.curve == EllipticCurve::Ed25519 => {
                URL_SAFE_NO_PAD
                    .decode(&okp.x)
                    .ok()
                    .filter(|public| public.len() == 32)
                    .map(|public| crate::did_key::ed25519_did_key(&public))
            }
            _ => None,
        };
        KeyOwner {
            kid: jwk.common.key_id.clone(),
            did,
        }
    }

    /// Whether `sub` names this key's owner.
    pub(crate) fn owns(&self, sub: &str) -> bool {
        self.kid.as_deref() == Some(sub) || self.did.as_deref() == Some(sub)
    }

    pub(crate) fn kid(&self) -> Option<&str> {
        self.kid.as_deref()
    }
}

/// A key that may have signed a token.
pub(crate) struct Candidate {
    pub(crate) key: DecodingKey,
    pub(crate) algorithm: Algorithm,
    /// The key's owner when it comes from a static trust list, where each key
    /// is one peer's. `None` for an identity provider's keys, which sign
    /// tokens for many subjects.
    pub(crate) owner: Option<KeyOwner>,
}

impl Candidate {
    fn into_key(self) -> (DecodingKey, Algorithm) {
        (self.key, self.algorithm)
    }
}

/// Cache entry for a JWKS.
///
/// `by_kid` is a precomputed index (`kid` -> JWK) built once when the entry is
/// created, so a token that carries a `kid` resolves its key in O(1) instead of
/// scanning every JWK on each verification.
#[derive(Clone, Debug)]
pub struct JwksCache {
    pub jwks: JwkSet,
    pub by_kid: HashMap<String, Jwk>,
    pub fetched_at: Instant,
    pub ttl: Duration,
}

impl JwksCache {
    /// Build a cache entry, indexing keys that declare a `kid` for O(1) lookup.
    pub fn new(jwks: JwkSet, fetched_at: Instant, ttl: Duration) -> Self {
        let mut by_kid = HashMap::with_capacity(jwks.keys.len());
        for key in &jwks.keys {
            if let Some(kid) = &key.common.key_id {
                // First writer wins on duplicate kids; duplicates are also still
                // reachable via the algorithm-scan fallback.
                by_kid.entry(kid.clone()).or_insert_with(|| key.clone());
            }
        }
        Self {
            jwks,
            by_kid,
            fetched_at,
            ttl,
        }
    }
}

/// This struct provides methods to resolve JWT decoding keys from various sources.
///
/// The `KeyResolver` is responsible for fetching and caching JSON Web Keys (JWK)
/// from OpenID Connect providers. It supports:
///
/// 1. OpenID Connect Discovery via the standard `.well-known/openid-configuration` endpoint
/// 2. Direct retrieval from the `.well-known/jwks.json` endpoint as a fallback
/// 3. Caching of retrieved keys to minimize network requests
///
/// Example usage:
///
/// ```rust,no_run
/// use std::time::Duration;
/// use slim_auth::builder::JwtBuilder;
/// use slim_auth::resolver::KeyResolver;
///
/// let resolver = KeyResolver::new()
///     .with_jwks_ttl(Duration::from_secs(1800));  // 30 minute cache TTL
///
/// let verifier = JwtBuilder::new()
///     .issuer("https://your-oidc-provider.com")
///     .auto_resolve_keys(true)
///     .build()?
///     .with_key_resolver(resolver);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct KeyResolver {
    client: ReqwestClient,
    jwks_cache: RwLock<HashMap<String, JwksCache>>,
    default_jwks_ttl: Duration,
    /// Issuers whose JWKS is being fetched in the background right now: those that
    /// are not `followed`.
    refreshing: RwLock<HashSet<String>>,
    /// The issuer whose keys `refresh` keeps fresh, once [`Self::follow`] succeeded.
    followed: RwLock<Option<String>>,
    /// Keeps the keys of the followed issuer fresh, and stops when the resolver is
    /// dropped.
    refresh: RefreshTask,
}

/// Marks `issuer` as no longer being fetched when dropped.
struct InFlight<'a> {
    resolver: &'a KeyResolver,
    issuer: &'a str,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.resolver.refreshing.write().remove(self.issuer);
    }
}

impl Default for KeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyResolver {
    const STATIC_JWKS_ENTRY: &'static str = "static_jwks";

    /// Bound on each request to an issuer, so a hung one cannot keep its background
    /// fetch (and the next attempt) waiting forever.
    const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

    /// Create a new KeyResolver with default settings
    pub fn new() -> Self {
        // Create a reqwest client with default TLS configuration
        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth")
            .timeout(Self::FETCH_TIMEOUT)
            .build()
            .expect("Failed to create reqwest client");

        Self {
            client,
            jwks_cache: RwLock::new(HashMap::new()),
            default_jwks_ttl: Duration::from_secs(3600), // 1 hour default TTL
            refreshing: RwLock::new(HashSet::new()),
            followed: RwLock::new(None),
            refresh: RefreshTask::new(),
        }
    }

    pub fn with_jwks(jwks: JwkSet) -> Self {
        // Initialize the cache with the provided JWKS
        let mut cache = HashMap::new();
        cache.insert(
            Self::STATIC_JWKS_ENTRY.to_string(),
            JwksCache::new(
                jwks,
                Instant::now(),
                Duration::from_secs(u64::MAX), // static JWKS, infinite TTL
            ),
        );

        let client = ReqwestClient::builder()
            .user_agent("AGNTCY Slim Auth")
            .timeout(Self::FETCH_TIMEOUT)
            .build()
            .expect("Failed to create reqwest client");

        Self {
            client,
            jwks_cache: RwLock::new(cache),
            default_jwks_ttl: Duration::from_secs(3600), // 1 hour default TTL
            refreshing: RwLock::new(HashSet::new()),
            followed: RwLock::new(None),
            refresh: RefreshTask::new(),
        }
    }

    /// Set the default TTL for cached JWKS
    pub fn with_jwks_ttl(mut self, ttl: Duration) -> Self {
        self.default_jwks_ttl = ttl;
        self
    }

    /// Resolve a decoding key from various sources
    ///
    /// This function will attempt to resolve the key in the following order:
    /// 1. If a decoding key is already provided, return it
    /// 2. If a kid (Key ID) is specified in the token header, fetch the key from the JWKS endpoint
    /// 3. If no kid is specified, use the first suitable key from the JWKS endpoint
    ///
    /// # Arguments
    /// * `issuer` - The token issuer URL
    /// * `token_header` - The JWT header containing the algorithm and key ID (if available)
    pub async fn resolve_key(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<(DecodingKey, Algorithm), AuthError> {
        self.resolve_keys(issuer, token_header)
            .await?
            .into_iter()
            .next()
            .ok_or(AuthError::JwksNoSuitableKey)
    }

    /// Resolve every candidate decoding key for a token header.
    ///
    /// Unlike [`Self::resolve_key`], this returns *all* keys that could plausibly
    /// have signed the token (exact `kid` match first, then any key whose
    /// algorithm matches the token). The caller is expected to try each in turn
    /// until one verifies the signature. This is what makes a multi-key JWKS
    /// allow-list usable when tokens carry no `kid`: there is no way to pick the
    /// right key from the header alone, so the signature itself is the selector.
    pub async fn resolve_keys(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<Vec<(DecodingKey, Algorithm)>, AuthError> {
        let candidates = self.resolve_candidates(issuer, token_header).await?;
        Ok(candidates.into_iter().map(Candidate::into_key).collect())
    }

    /// [`Self::resolve_keys`], with the owner of each static trust-list key.
    pub(crate) async fn resolve_candidates(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<Vec<Candidate>, AuthError> {
        // Check if we have a static JWKS entry
        if let Some(cache_entry) = self.jwks_cache.read().get(Self::STATIC_JWKS_ENTRY) {
            // If we have a static JWKS, use it directly
            return self.candidates_from_cache(cache_entry, token_header, true);
        }

        // Try to get cached keys if available
        if let Ok(candidates) = self.cached_candidates(issuer, token_header) {
            return Ok(candidates);
        }

        // Try to fetch the keys from the well-known JWKS endpoint (also caches it
        // with its `by_kid` index for subsequent O(1) lookups).
        let jwks = download_jwks_of(&self.client, issuer).await?;
        self.jwks_cache.write().insert(
            issuer.to_string(),
            JwksCache::new(jwks, Instant::now(), self.default_jwks_ttl),
        );
        self.cached_candidates(issuer, token_header)
    }

    /// Convert a JWK to a DecodingKey
    fn jwk_to_decoding_key(&self, jwk: &Jwk) -> Result<DecodingKey, AuthError> {
        let ret = DecodingKey::from_jwk(jwk)?;
        Ok(ret)
    }

    fn key_alg_to_algorithm(&self, alg: &KeyAlgorithm) -> Result<Algorithm, AuthError> {
        match alg {
            KeyAlgorithm::HS256 => Ok(Algorithm::HS256),
            KeyAlgorithm::HS384 => Ok(Algorithm::HS384),
            KeyAlgorithm::HS512 => Ok(Algorithm::HS512),
            KeyAlgorithm::ES256 => Ok(Algorithm::ES256),
            KeyAlgorithm::ES384 => Ok(Algorithm::ES384),
            KeyAlgorithm::RS256 => Ok(Algorithm::RS256),
            KeyAlgorithm::RS384 => Ok(Algorithm::RS384),
            KeyAlgorithm::RS512 => Ok(Algorithm::RS512),
            KeyAlgorithm::PS256 => Ok(Algorithm::PS256),
            KeyAlgorithm::PS384 => Ok(Algorithm::PS384),
            KeyAlgorithm::PS512 => Ok(Algorithm::PS512),
            KeyAlgorithm::EdDSA => Ok(Algorithm::EdDSA),
            _ => Err(AuthError::JwtUnsupportedKeyAlgorithm(*alg)),
        }
    }

    /// Collect the candidate decoding key(s) for a token header, most likely first.
    ///
    /// Fast path (O(1)): if the token carries a `kid` and the cache's `by_kid`
    /// index holds it, that single key is returned directly — one JWK conversion,
    /// one signature check. slim's signer stamps `kid = sub`, so this is the normal
    /// case for a multi-member allow-list.
    ///
    /// Fallback (O(n)): a `kid` may be absent (legacy tokens) or not match any
    /// indexed key. Then we return every key whose algorithm matches the token so
    /// verification can try each until the signature checks out — because the right
    /// key cannot be chosen from the header alone, the signature itself is the
    /// selector. The previous behaviour returned only the *first* algorithm match,
    /// so with a multi-key allow-list every member other than keys[0] failed with
    /// `InvalidSignature`.
    ///
    /// `peer_keys` marks a static trust list, where each key belongs to one
    /// peer: its candidates then carry their [`KeyOwner`].
    fn candidates_from_cache(
        &self,
        cache: &JwksCache,
        token_header: &Header,
        peer_keys: bool,
    ) -> Result<Vec<Candidate>, AuthError> {
        let candidate = |jwk: &Jwk, algorithm| -> Result<Candidate, AuthError> {
            Ok(Candidate {
                key: self.jwk_to_decoding_key(jwk)?,
                algorithm,
                owner: peer_keys.then(|| KeyOwner::of(jwk)),
            })
        };

        // Fast path: exact `kid` hit via the precomputed index.
        if let Some(kid) = &token_header.kid
            && let Some(jwk) = cache.by_kid.get(kid)
        {
            let alg = jwk
                .common
                .key_algorithm
                .as_ref()
                .ok_or(AuthError::JwksNoSuitableKey)
                .and_then(|a| self.key_alg_to_algorithm(a))?;
            return Ok(vec![candidate(jwk, alg)?]);
        }

        // Fallback: every key whose algorithm matches the token.
        let mut candidates = Vec::new();
        for key in &cache.jwks.keys {
            if let Some(alg) = &key.common.key_algorithm
                && let Ok(algorithm) = self.key_alg_to_algorithm(alg)
                && algorithm == token_header.alg
            {
                candidates.push(candidate(key, algorithm)?);
            }
        }

        if candidates.is_empty() {
            return Err(AuthError::JwksNoSuitableKey);
        }
        Ok(candidates)
    }

    /// Check the cache for a JWKS entry (single-key, backwards compatible).
    pub fn get_cached_key(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<(DecodingKey, Algorithm), AuthError> {
        self.get_cached_keys(issuer, token_header)?
            .into_iter()
            .next()
            .ok_or(AuthError::JwksNoSuitableKey)
    }

    /// Check the cache for a JWKS entry, returning every candidate key.
    pub fn get_cached_keys(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<Vec<(DecodingKey, Algorithm)>, AuthError> {
        let candidates = self.cached_candidates(issuer, token_header)?;
        Ok(candidates.into_iter().map(Candidate::into_key).collect())
    }

    /// [`Self::get_cached_keys`], with the owner of each static trust-list key.
    pub(crate) fn cached_candidates(
        &self,
        issuer: &str,
        token_header: &Header,
    ) -> Result<Vec<Candidate>, AuthError> {
        // Check if we have a cached JWKS that's still valid
        let cache = self.jwks_cache.read();

        // Check static JWKS entry first
        if let Some(cache_entry) = cache.get(Self::STATIC_JWKS_ENTRY) {
            // no need to check the elapsed time for static JWKS
            return self.candidates_from_cache(cache_entry, token_header, true);
        }

        let cache_entry = cache.get(issuer);
        if cache_entry.is_none() {
            return Err(AuthError::JwksCacheMiss {
                issuer: issuer.to_string(),
            });
        }

        let cache_entry = cache_entry.unwrap();

        if cache_entry.fetched_at.elapsed() > cache_entry.ttl {
            return Err(AuthError::JwksCacheExpired {
                issuer: issuer.to_string(),
            });
        }

        // If we have a valid cache entry, collect the candidate keys
        self.candidates_from_cache(cache_entry, token_header, false)
    }

    /// Whether the keys of `issuer` are refreshed by the task started by [`Self::follow`].
    fn is_followed(&self, issuer: &str) -> bool {
        self.followed.read().as_deref() == Some(issuer)
    }

    /// Whether the resolver holds a fixed key set and never fetches.
    pub(crate) fn is_static(&self) -> bool {
        self.jwks_cache.read().contains_key(Self::STATIC_JWKS_ENTRY)
    }

    /// Whether the cached keys of `issuer` have a key with this `kid`.
    pub(crate) fn has_kid(&self, issuer: &str, kid: &str) -> bool {
        self.jwks_cache
            .read()
            .get(issuer)
            .is_some_and(|entry| entry.by_kid.contains_key(kid))
    }

    /// Fetch the keys of `issuer` now, and keep them fresh from then on.
    ///
    /// Returns once the first keys are cached, and fails if they cannot be fetched. A
    /// task then refreshes them at 80% of their TTL, retries a failed refresh with a
    /// backoff, and stops when the resolver is dropped (see [`RefreshTask`]). Does nothing
    /// for a static JWKS, which needs no fetch, and nothing if the task already runs.
    pub(crate) async fn follow(self: &Arc<Self>, issuer: &str) -> Result<(), AuthError> {
        if self.is_static() {
            return Ok(());
        }
        self.refresh
            .start(
                self.default_jwks_ttl,
                {
                    // The task keeps a client and the issuer, not the resolver: dropping
                    // the resolver must be able to cancel it.
                    let (client, issuer) = (self.client.clone(), issuer.to_string());
                    move || {
                        let (client, issuer) = (client.clone(), issuer.clone());
                        async move { download_jwks_of(&client, &issuer).await }
                    }
                },
                {
                    let (resolver, issuer, ttl) = (
                        Arc::downgrade(self),
                        issuer.to_string(),
                        self.default_jwks_ttl,
                    );
                    move |jwks| Self::store(&resolver, &issuer, jwks, ttl)
                },
            )
            .await?;
        // Only once the task runs: an issuer whose first fetch failed has no task, and
        // must keep fetching from the request path.
        *self.followed.write() = Some(issuer.to_string());
        Ok(())
    }

    /// Cache `jwks` for `issuer`. Returns whether the resolver still exists.
    fn store(resolver: &Weak<Self>, issuer: &str, jwks: JwkSet, ttl: Duration) -> bool {
        let Some(resolver) = resolver.upgrade() else {
            return false;
        };
        resolver.jwks_cache.write().insert(
            issuer.to_string(),
            JwksCache::new(jwks, Instant::now(), ttl),
        );
        true
    }

    /// A token named a key that the cached keys of `issuer` lack: the issuer may have
    /// rotated them. Asks the task for an early refresh, which it rate-limits. Does
    /// nothing for an issuer that is not followed.
    pub(crate) fn request_key_refresh(&self, issuer: &str) {
        if self.is_followed(issuer) {
            self.refresh.request_refresh();
        }
    }

    /// Make sure the keys of an issuer that is not followed get fetched: a verifier
    /// without a configured issuer (`allow_any_issuer`) learns its issuers from the
    /// tokens, so it has no task to rely on and fetches on a miss or at 80% of the TTL.
    /// The keys of a followed issuer are the task's business.
    pub(crate) fn ensure_fresh(self: &Arc<Self>, issuer: &str) {
        if !self.is_followed(issuer) && self.needs_refresh(issuer) {
            self.refresh_in_background(issuer);
        }
    }

    /// Whether the cached JWKS of `issuer` is missing or past 80% of its TTL, so a
    /// refresh should start before keys run out. A static JWKS never needs one.
    fn needs_refresh(&self, issuer: &str) -> bool {
        let cache = self.jwks_cache.read();
        if cache.contains_key(Self::STATIC_JWKS_ENTRY) {
            return false;
        }
        match cache.get(issuer) {
            None => true,
            Some(entry) => entry.fetched_at.elapsed() > entry.ttl.mul_f32(0.8),
        }
    }

    /// Fetch the JWKS of `issuer` in the background, without waiting for it.
    ///
    /// The path of an issuer that is not followed. At most one fetch per issuer runs at
    /// a time, and nothing happens outside a Tokio runtime. A failed fetch is logged;
    /// the next miss tries again.
    fn refresh_in_background(self: &Arc<Self>, issuer: &str) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if !self.refreshing.write().insert(issuer.to_string()) {
            return;
        }
        let resolver = self.clone();
        let issuer = issuer.to_string();
        runtime.spawn(async move {
            // Dropped on every exit, even if the task is cancelled or panics, so the
            // issuer is never left marked as being fetched.
            let _in_flight = InFlight {
                resolver: &resolver,
                issuer: &issuer,
            };
            match download_jwks_of(&resolver.client, &issuer).await {
                Ok(jwks) => {
                    let ttl = resolver.default_jwks_ttl;
                    Self::store(&Arc::downgrade(&resolver), &issuer, jwks, ttl);
                }
                Err(e) => tracing::warn!(%issuer, error = %e, "background JWKS fetch failed"),
            }
        });
    }

    /// Build the JWKS URI from the issuer.
    #[cfg(test)]
    async fn build_jwks_uri(&self, issuer: &str) -> Result<String, AuthError> {
        build_jwks_uri(&self.client, issuer).await
    }

    /// Fetch JWKS from the specified URI.
    #[cfg(test)]
    async fn fetch_jwks_from_uri(&self, uri: &str) -> Result<JwkSet, AuthError> {
        fetch_jwks_from_uri(&self.client, uri).await
    }
}

/// Fetch the JWKS of `issuer`: discover its URI (OpenID Connect Discovery, or the standard
/// well-known location), then download it.
async fn download_jwks_of(client: &ReqwestClient, issuer: &str) -> Result<JwkSet, AuthError> {
    let jwks_uri = build_jwks_uri(client, issuer).await?;
    fetch_jwks_from_uri(client, &jwks_uri).await
}

/// Build the JWKS URI from the issuer
///
/// This function first tries to discover the JWKS URI via OpenID Connect Discovery
/// (.well-known/openid-configuration), and falls back to the standard .well-known/jwks.json
/// location if that fails.
async fn build_jwks_uri(client: &ReqwestClient, issuer: &str) -> Result<String, AuthError> {
    // Parse the issuer URL
    // Use typed URL parse error propagation
    let mut issuer_url = Url::parse(issuer)?;

    // First try OpenID Connect Discovery endpoint
    let mut openid_config_url = issuer_url.clone();
    let mut openid_path = openid_config_url.path().trim_end_matches('/').to_owned();
    openid_path.push_str("/.well-known/openid-configuration");
    openid_config_url.set_path(&openid_path);

    // Try to fetch the OpenID configuration
    let openid_config_response = client.get(openid_config_url.to_string()).send().await;

    // If we successfully got the OpenID configuration, validate and extract the jwks_uri
    if let Ok(response) = openid_config_response
        && response.status() == StatusCode::OK
        && let Ok(config) = response.json::<serde_json::Value>().await
    {
        // Validate 'issuer' field if present (required by OIDC spec).
        if let Some(doc_issuer) = config.get("issuer").and_then(|v| v.as_str())
            && doc_issuer.trim_end_matches('/') != issuer.trim_end_matches('/')
        {
            return Err(AuthError::OidcDiscoveryIssuerMismatch {
                expected: issuer.to_string(),
                got: doc_issuer.to_string(),
            });
        }
        // Only use jwks_uri if it shares the same origin as the issuer.
        if let Some(jwks_uri) = config.get("jwks_uri").and_then(|v| v.as_str()) {
            let jwks_url = Url::parse(jwks_uri)?;
            if same_origin(&issuer_url, &jwks_url) {
                return Ok(jwks_uri.to_string());
            } else {
                return Err(AuthError::OidcDiscoveryUrlOriginMismatch {
                    field: "jwks_uri",
                    url: jwks_uri.to_string(),
                });
            }
        }
    }

    // Fallback to standard well-known JWKS location
    let mut path = issuer_url.path().trim_end_matches('/').to_owned();
    path.push_str("/.well-known/jwks.json");
    issuer_url.set_path(&path);

    Ok(issuer_url.to_string())
}

/// Fetch JWKS from the specified URI
async fn fetch_jwks_from_uri(client: &ReqwestClient, uri: &str) -> Result<JwkSet, AuthError> {
    // Send the GET request using reqwest
    let response = client.get(uri).send().await?;

    // Check the response status
    if response.status() != StatusCode::OK {
        return Err(AuthError::JwtFetchJwksFailed(response.status()));
    }

    // Get the response body as bytes
    let body = response.bytes().await?;

    // Parse the JWKS
    let jwks: JwkSet = serde_json::from_slice(&body)?;

    Ok(jwks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Helper to create a test resolver with a client that can talk to the mock server
    async fn create_test_resolver() -> (KeyResolver, MockServer) {
        slim_config::tls::provider::initialize_crypto_provider();
        let server = MockServer::start().await;
        let resolver = KeyResolver::new();
        (resolver, server)
    }

    #[tokio::test]
    async fn test_build_jwks_uri_with_openid_discovery() {
        let (resolver, mock_server) = create_test_resolver().await;

        let mock_uri = mock_server.uri();
        let jwks_uri = format!("{}/custom/path/to/jwks.json", mock_uri);
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": mock_uri,
                "jwks_uri": jwks_uri,
            })))
            .mount(&mock_server)
            .await;

        let uri = resolver.build_jwks_uri(&mock_uri).await.unwrap();
        assert_eq!(uri, jwks_uri);
    }

    #[tokio::test]
    async fn test_build_jwks_uri_fallback() {
        let (resolver, mock_server) = create_test_resolver().await;

        // Setup mock to return 404 for OpenID discovery endpoint
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        // Test that we fall back to the standard JWKS URI
        let uri = resolver.build_jwks_uri(&mock_server.uri()).await.unwrap();
        assert_eq!(uri, format!("{}/.well-known/jwks.json", mock_server.uri()));
    }

    #[tokio::test]
    async fn test_fetch_jwks_from_uri() {
        let (resolver, mock_server) = create_test_resolver().await;

        // Setup mock for JWKS endpoint
        let jwks = json!({
            "keys": [
                {
                    "kty": "RSA",
                    "kid": "test-key",
                    "n": "some-modulus",
                    "e": "AQAB"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&mock_server)
            .await;

        // Fetch the JWKS from the mock server
        let jwks_uri = format!("{}/.well-known/jwks.json", mock_server.uri());
        let fetched_jwks = resolver.fetch_jwks_from_uri(&jwks_uri).await.unwrap();

        assert_eq!(fetched_jwks.keys.len(), 1);
        assert_eq!(
            fetched_jwks.keys[0].common.key_id.as_deref(),
            Some("test-key")
        );
    }

    /// A miss is filled by a background fetch, and `needs_refresh` reports it until then.
    #[tokio::test]
    async fn test_background_refresh_fills_the_cache() {
        let (resolver, mock_server) = create_test_resolver().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [{ "kty": "RSA", "kid": "test-key", "n": "some-modulus", "e": "AQAB" }]
            })))
            .mount(&mock_server)
            .await;

        let resolver = Arc::new(resolver);
        let issuer = mock_server.uri();
        assert!(resolver.needs_refresh(&issuer));

        resolver.refresh_in_background(&issuer);

        tokio::time::timeout(Duration::from_secs(5), async {
            while resolver.needs_refresh(&issuer) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the background fetch should fill the cache");
    }

    /// Refresh starts ahead of expiry, once an entry is past 80% of its TTL.
    #[tokio::test]
    async fn test_needs_refresh_before_the_ttl_runs_out() {
        let (resolver, _mock_server) = create_test_resolver().await;
        let ttl = Duration::from_secs(100);
        let fresh = JwksCache::new(JwkSet { keys: vec![] }, Instant::now(), ttl);
        let aging = JwksCache::new(
            JwkSet { keys: vec![] },
            Instant::now() - Duration::from_secs(90),
            ttl,
        );
        resolver.jwks_cache.write().insert("fresh".into(), fresh);
        resolver.jwks_cache.write().insert("aging".into(), aging);

        assert!(!resolver.needs_refresh("fresh"));
        assert!(resolver.needs_refresh("aging"));
        assert!(resolver.needs_refresh("missing"));
    }

    /// A static key set is complete: it never needs a refresh.
    #[tokio::test]
    async fn test_a_static_jwks_never_needs_a_refresh() {
        let resolver = KeyResolver::with_jwks(JwkSet { keys: vec![] });
        assert!(!resolver.needs_refresh("any-issuer"));
    }

    /// A failed `follow` leaves the issuer unfollowed, so its keys keep being fetched from
    /// the request path, and a later `follow` can succeed.
    #[tokio::test]
    async fn test_a_failed_follow_leaves_the_issuer_unfollowed() {
        slim_config::tls::provider::initialize_crypto_provider();
        let resolver = Arc::new(KeyResolver::new());
        assert!(resolver.follow("http://127.0.0.1:1").await.is_err());
        assert!(!resolver.is_followed("http://127.0.0.1:1"));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "keys": [{ "kty": "RSA", "kid": "k", "n": "some-modulus", "e": "AQAB" }]
            })))
            .mount(&server)
            .await;
        resolver.follow(&server.uri()).await.unwrap();
        assert!(resolver.is_followed(&server.uri()));
        assert!(resolver.has_kid(&server.uri(), "k"));
    }

    /// A static key set needs no task and no fetch: `follow` does nothing.
    #[tokio::test]
    async fn test_a_static_jwks_is_not_followed() {
        let server = MockServer::start().await;
        let resolver = Arc::new(KeyResolver::with_jwks(JwkSet { keys: vec![] }));
        resolver.follow(&server.uri()).await.unwrap();
        assert!(!resolver.is_followed(&server.uri()));
        assert!(!resolver.refresh.is_running());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

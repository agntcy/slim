// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;
use std::pin::Pin;
use std::{
    convert::TryFrom,
    task::{Context, Poll},
};

use headers::{HeaderMapExt, authorization::Bearer};
use http::HeaderValue;
use http::{Request, Response};
use pin_project::pin_project;
use serde::Deserialize;
use tower_layer::Layer;
use tower_service::Service;

use crate::errors::AuthError;
use crate::metadata::MetadataMap;
use crate::traits::{TokenProvider, Verifier};
use display_error_chain::ErrorChainExt;
use futures::future::{Either, Ready, ready};

/// Layer to sign JWT tokens with a signing key. Custom claims can be added to the token.
#[derive(Clone)]
pub struct AddJwtLayer<T>
where
    T: TokenProvider + Clone,
{
    provider: T,
}

impl<T: TokenProvider + Clone> AddJwtLayer<T> {
    pub fn new(signer: T) -> Self {
        Self { provider: signer }
    }
    /// Asynchronously initialize the underlying `TokenProvider` before
    /// the layer is used to construct services. This must be called for
    /// providers that perform network or other async setup (e.g. OIDC,
    /// SPIFFE) because `Layer::layer` and `TokenProvider::get_token` are
    /// both synchronous.
    ///
    /// If the provider does not need initialization this will be a no-op.
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        self.provider.initialize().await
    }
}

/// Layer implementation for `SignJwtLayer` that adds JWT tokens to requests
impl<S, T: TokenProvider + Clone> Layer<S> for AddJwtLayer<T> {
    type Service = AddJwtToken<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        AddJwtToken {
            inner,
            provider: self.provider.clone(),
        }
    }
}

/// Middleware for adding a JWT token to the request headers
#[derive(Clone)]
pub struct AddJwtToken<S, T: TokenProvider + Clone> {
    inner: S,
    provider: T,
}

impl<S, T: TokenProvider + Clone> AddJwtToken<S, T> {
    /// Get a JWT token, either from cache or from provider
    pub fn get_token(&mut self) -> Result<HeaderValue, AuthError> {
        let token = self.provider.get_token()?;
        let header_value = HeaderValue::try_from(format!("Bearer {}", token))?;
        Ok(header_value)
    }
}

impl<S, T, ReqBody, ResBody> Service<Request<ReqBody>> for AddJwtToken<S, T>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    T: TokenProvider + Clone,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    /// Poll the inner service to see if it is ready to accept requests
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    /// Call the inner service with the request, adding the JWT token to the headers
    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        if let Ok(token) = self.get_token() {
            req.headers_mut().insert(http::header::AUTHORIZATION, token);
        }
        // Even if we fail to get a token, we still proceed with the request
        // as the downstream service may handle the lack of authorization
        self.inner.call(req)
    }
}

/// Layer to validate JWT tokens.
#[derive(Clone)]
pub struct ValidateJwtLayer<Claim, V: Verifier> {
    /// Provided verifier
    verifier: V,

    /// Claims to validate against. Not used for now.
    _claims: Claim,
}

impl<Claim: Clone, V: Verifier + Clone> ValidateJwtLayer<Claim, V> {
    /// Create a new layer to validate JWT tokens with the given decoding key
    /// Tokens will only be accepted if they pass the validation
    pub fn new(verifier: V, claims: Claim) -> Self {
        Self {
            verifier,
            _claims: claims,
        }
    }

    /// Asynchronously initialize the underlying `Verifier` before
    /// constructing services. Call this during application startup.
    /// Safe to call multiple times; subsequent calls should be cheap
    /// as implementors may simply return `Ok(())` if already initialized.
    pub async fn initialize(&mut self) -> Result<(), AuthError> {
        self.verifier.initialize().await
    }
}

impl<S, Claim: Clone, V: Verifier + Clone> Layer<S> for ValidateJwtLayer<Claim, V> {
    type Service = ValidateJwt<S, Claim, V>;

    fn layer(&self, inner: S) -> Self::Service {
        ValidateJwt {
            inner,
            verifier: std::sync::Arc::new(self.verifier.clone()),
            _phantom: PhantomData::<Claim>,
        }
    }
}

/// This middleware validates JWT tokens in the request headers.
#[derive(Clone)]
pub struct ValidateJwt<S, Claim, V: Clone> {
    inner: S,
    verifier: std::sync::Arc<V>,
    _phantom: PhantomData<Claim>,
}

type AsyncTraitFuture<A> = Pin<Box<dyn Future<Output = A> + Send>>;

#[pin_project(project = JwtFutureProj, project_replace = JwtFutureProjOwn)]
#[allow(clippy::large_enum_variant)]
pub enum JwtFuture<
    TService: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ReqBody,
    ResBody,
    Claim,
> {
    // If there was an error return a UNAUTHORIZED.
    Error,

    // We are ready to call the inner service.
    WaitForFuture {
        #[pin]
        future: TService::Future,
    },

    // We have a token, but we need to verify it.
    WaitForKey {
        request: Request<ReqBody>,
        #[pin]
        verifier_future: AsyncTraitFuture<Result<Claim, AuthError>>,
        service: TService,
        _phantom: PhantomData<Claim>,
    },
}

impl<TService, ReqBody, ResBody, Claim> Future for JwtFuture<TService, ReqBody, ResBody, Claim>
where
    TService: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
    for<'de> Claim: Deserialize<'de> + Send + Sync + Clone + 'static,
{
    type Output = Result<TService::Response, TService::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            JwtFutureProj::Error => {
                let response = Response::builder()
                    .status(http::StatusCode::UNAUTHORIZED)
                    .body(Default::default())
                    .unwrap();

                Poll::Ready(Ok(response))
            }
            JwtFutureProj::WaitForFuture { future } => future.poll(cx),
            JwtFutureProj::WaitForKey {
                verifier_future, ..
            } => match verifier_future.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(_e)) => {
                    let response = Response::builder()
                        .status(http::StatusCode::UNAUTHORIZED)
                        .body(Default::default())
                        .unwrap();

                    Poll::Ready(Ok(response))
                }
                Poll::Ready(Ok(claims)) => {
                    let owned = self.as_mut().project_replace(JwtFuture::Error);
                    match owned {
                        JwtFutureProjOwn::WaitForKey {
                            mut request,
                            mut service,
                            ..
                        } => {
                            request.extensions_mut().insert(claims);
                            let future = service.call(request);
                            self.as_mut().set(JwtFuture::WaitForFuture { future });
                            self.poll(cx)
                        }
                        _ => unreachable!(
                            "This should not happen, we should always be in the HasTokenWaitingForDecodingKey state."
                        ),
                    }
                }
            },
        }
    }
}

impl<S, ReqBody, ResBody, Claim, V: Clone> Service<Request<ReqBody>> for ValidateJwt<S, Claim, V>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Send + Clone + 'static,
    S::Future: Send + 'static,
    ResBody: Default,
    V: Verifier + Send + Sync + 'static,
    for<'de> Claim: Deserialize<'de> + Send + Sync + Clone + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = JwtFuture<S, ReqBody, ResBody, Claim>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        match req
            .headers()
            .typed_try_get::<headers::Authorization<Bearer>>()
        {
            Ok(Some(bearer)) => {
                let bearer_token = bearer.token().to_string();

                // Let's first try to perform the verification without cloning the verifier
                match self.verifier.try_get_claims::<Claim>(&bearer_token) {
                    Ok(claims) => {
                        // Store claims in request extensions
                        req.extensions_mut().insert(claims);

                        // Call the inner service directly
                        Self::Future::WaitForFuture {
                            future: self.inner.call(req),
                        }
                    }
                    Err(e) => {
                        // WouldBlockOn is the expected cold-cache path; anything else is surprising
                        if matches!(e, AuthError::WouldBlockOn) {
                            tracing::debug!(
                                "ValidateJwt: JWKS cache cold, using async verification"
                            );
                        } else {
                            tracing::warn!(error = %e.chain(), "ValidateJwt: sync try_get_claims failed, trying async");
                        }
                        let verifier = self.verifier.clone();
                        let clone = self.inner.clone();
                        let inner = std::mem::replace(&mut self.inner, clone);

                        Self::Future::WaitForKey {
                            request: req,
                            verifier_future: Box::pin(async move {
                                // Perform the verification asynchronously
                                let result = verifier.get_claims::<Claim>(&bearer_token).await;
                                if let Err(ref e) = result {
                                    tracing::warn!(error = %e.chain(), "ValidateJwt: async get_claims also failed");
                                }
                                result
                            }),
                            service: inner,
                            _phantom: self._phantom,
                        }
                    }
                }
            }
            _ => {
                tracing::warn!("ValidateJwt: no Authorization Bearer header found in request");
                Self::Future::Error
            }
        }
    }
}

#[derive(Clone)]
enum PolicyEngine {
    Rego(Box<regorus::Engine>),
    Cel(std::sync::Arc<cel::Program>),
}

/// Layer that enforces a Rego or CEL policy against verified JWT claims.
/// Must sit *after* [`ValidateJwtLayer`] so claims are in extensions before this runs.
#[derive(Clone)]
pub struct PolicyCheckLayer {
    engine: Option<PolicyEngine>,
}

impl PolicyCheckLayer {
    /// No policy — all authenticated requests pass through.
    pub fn none() -> Self {
        Self { engine: None }
    }

    /// Inline Rego policy. Fails at construction if the policy doesn't compile.
    /// Policy must define `package slim.auth` with `default allow = false`.
    /// Claims are available as `input.claims.*`.
    pub fn rego(text: &str) -> Result<Self, AuthError> {
        let mut e = regorus::Engine::new();
        e.add_policy("slim/auth.rego".to_string(), text.to_string())
            .map_err(|e| AuthError::PolicyCompile(e.to_string()))?;
        Ok(Self {
            engine: Some(PolicyEngine::Rego(Box::new(e))),
        })
    }

    /// Rego policy loaded from a file. Fails at construction if the file is
    /// unreadable or the policy doesn't compile.
    pub fn rego_file(path: &std::path::Path) -> Result<Self, AuthError> {
        let mut e = regorus::Engine::new();
        e.add_policy_from_file(path)
            .map_err(|e| AuthError::PolicyCompile(e.to_string()))?;
        Ok(Self {
            engine: Some(PolicyEngine::Rego(Box::new(e))),
        })
    }

    /// CEL expression that must evaluate to `true` to allow the request.
    /// Claims are available as `claims.*` (e.g. `"admin" in claims.groups`).
    /// Fails at construction if the expression doesn't parse.
    pub fn cel(expression: &str) -> Result<Self, AuthError> {
        let program = cel::Program::compile(expression)
            .map_err(|e| AuthError::PolicyCompile(e.to_string()))?;
        Ok(Self {
            engine: Some(PolicyEngine::Cel(std::sync::Arc::new(program))),
        })
    }
}

impl<S> Layer<S> for PolicyCheckLayer {
    type Service = PolicyCheck<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PolicyCheck {
            inner,
            engine: self.engine.clone(),
        }
    }
}

/// Middleware that evaluates a Rego or CEL policy against JWT claims.
/// Passes through when no policy is configured.
#[derive(Clone)]
pub struct PolicyCheck<S> {
    inner: S,
    engine: Option<PolicyEngine>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for PolicyCheck<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Either<S::Future, Ready<Result<S::Response, S::Error>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let Some(engine) = &mut self.engine else {
            return Either::Left(self.inner.call(req));
        };

        let deny = || {
            Either::Right(ready(Ok(Response::builder()
                .status(http::StatusCode::FORBIDDEN)
                .body(Default::default())
                .unwrap())))
        };

        let Some(c) = req.extensions().get::<MetadataMap>() else {
            tracing::warn!(
                "PolicyCheck: no JWT claims in request — ValidateJwtLayer must wrap PolicyCheckLayer"
            );
            return deny();
        };

        let allowed = match engine {
            PolicyEngine::Rego(e) => serde_json::to_string(&serde_json::json!({ "claims": c }))
                .inspect_err(|e| tracing::error!(error=%e, "PolicyCheck: claim serialization failed"))
                .ok()
                .and_then(|json| {
                    e.set_input_json(&json)
                        .inspect_err(|e| tracing::error!(error=%e, "PolicyCheck: set_input_json failed"))
                        .ok()
                })
                .and_then(|_| {
                    e.eval_bool_query("data.slim.auth.allow".to_string(), false)
                        .inspect_err(|e| tracing::error!(error=%e, "PolicyCheck: rego eval error — verify policy uses 'package slim.auth'"))
                        .ok()
                })
                .unwrap_or(false),
            PolicyEngine::Cel(program) => {
                let mut ctx = cel::Context::default();
                ctx.add_variable("claims", c)
                    .inspect_err(|e| tracing::error!(error=%e, "PolicyCheck: failed to add claims to CEL context"))
                    .ok()
                    .and_then(|_| {
                        program
                            .execute(&ctx)
                            .inspect_err(|e| tracing::error!(error=%e, "PolicyCheck: CEL eval error"))
                            .ok()
                            .map(|v| v == cel::Value::Bool(true))
                    })
                    .unwrap_or(false)
            }
        };

        if allowed {
            Either::Left(self.inner.call(req))
        } else {
            tracing::warn!("PolicyCheck: policy denied request");
            deny()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::jwt::{Algorithm, KeyFormat};
    use crate::metadata::MetadataMap;
    use crate::traits::{Signer, StandardClaims};
    use crate::{builder::JwtBuilder, jwt::Key, jwt::KeyData};
    use futures::future::{self, Ready};
    use http::{Request, Response, StatusCode};
    use tower::{Service, ServiceBuilder};

    use super::*;

    // Define a Body type for testing
    type Body = Vec<u8>;

    // A simple test service that returns a 200 OK response
    #[derive(Clone)]
    struct TestService;

    impl Service<Request<Body>> for TestService {
        type Response = Response<Body>;
        type Error = std::convert::Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<Body>) -> Self::Future {
            // Check the authorization header is there
            // and starts with "Bearer "

            let auth_header = req.headers().get(http::header::AUTHORIZATION);
            let has_bearer = auth_header
                .and_then(|h| h.to_str().ok())
                .map(|s| s.starts_with("Bearer "))
                .unwrap_or(false);

            if !has_bearer {
                return future::ready(Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("Missing or invalid Authorization header"))
                    .unwrap()));
            }

            future::ready(Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Body::from("OK"))
                .unwrap()))
        }
    }

    #[tokio::test]
    async fn test_add_jwt_token() {
        // Set up a JWT signer
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        // Create our test service with the JWT signer layer
        let mut service = ServiceBuilder::new()
            .layer(AddJwtLayer::new(signer))
            .service(TestService);

        // Make a request
        let req = Request::builder()
            .uri("https://example.com")
            .body(vec![])
            .unwrap();

        // Service should add JWT to the request and return a 200 OK
        let response = service.call(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // An enhanced test service that checks for the Authorization header
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

    #[tokio::test]
    async fn test_jwt_verification() {
        // Set up a JWT signer and verifier with the same key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("shared-secret".to_string()),
            })
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("shared-secret".to_string()),
            })
            .build()
            .unwrap();

        // Create claims and token
        let claims = signer.create_claims();
        let token = signer.sign(&claims).unwrap();

        // Create a request with the token
        let req = Request::builder()
            .uri("https://example.com")
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::new())
            .unwrap();

        // Create our test service with the JWT verifier layer
        let mut service = ServiceBuilder::new()
            .layer(ValidateJwtLayer::new(verifier.clone(), claims.clone()))
            .service(TestService);

        // Service should verify the JWT and return a 200 OK
        let response = service.call(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Test with an invalid token
        let req = Request::builder()
            .uri("https://example.com")
            .header(http::header::AUTHORIZATION, "Bearer invalid.token")
            .body(Body::new())
            .unwrap();

        // Service should reject the invalid token with 401 Unauthorized
        let response = service.call(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_token_caching() {
        // Set up a JWT verifier
        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        // Create standard claims with a 10-second expiration from now
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Set up a JWT signer with the same key as the verifier
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("test-key".to_string()),
            })
            .build()
            .unwrap();

        // Sign the token
        let token = signer.sign_standard_claims().unwrap();
        let auth_header = format!("Bearer {}", token);

        // Create our service with the verifier
        let standard_claims = signer.create_claims();
        let mut service = ServiceBuilder::new()
            .layer(ValidateJwtLayer::new(
                verifier.clone(),
                standard_claims.clone(),
            ))
            .service(TestService);

        // Create a test request with the signed token
        let req1 = Request::builder()
            .uri("https://example.com")
            .header(http::header::AUTHORIZATION, &auth_header)
            .body(Body::new())
            .unwrap();

        // Service should accept the valid token
        let _response1 = service.call(req1).await.unwrap();

        // Make a second request with the same token
        // This should use the cached verification result
        let req2 = Request::builder()
            .uri("https://example.com")
            .header(http::header::AUTHORIZATION, &auth_header)
            .body(Body::new())
            .unwrap();

        // Service should accept the cached token
        let response2 = service.call(req2).await.unwrap();
        assert_eq!(response2.status(), StatusCode::OK);

        // Create a test with an expired token
        let mut expired_claims = standard_claims.clone();
        expired_claims.exp = now - 120; // Expired 120 seconds ago

        // Sign the expired token
        let expired_token = signer.sign(&expired_claims).unwrap();
        let expired_auth_header = format!("Bearer {}", expired_token);

        // Create a request with the expired token
        let req3 = Request::builder()
            .uri("https://example.com")
            .header(http::header::AUTHORIZATION, &expired_auth_header)
            .body(Body::new())
            .unwrap();

        // Service should reject the expired token
        let response3 = service.call(req3).await.unwrap();
        assert_eq!(response3.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_end_to_end() {
        // Create custom claims
        let mut claims = MetadataMap::new();
        claims.insert("claim1", 2u64);
        claims.insert("claim2", MetadataMap::from_iter(vec![("key", "value")]));

        // Set up a JWT signer and verifier with the same key
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .private_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("shared-secret".to_string()),
            })
            .custom_claims(claims.clone())
            .build()
            .unwrap();

        let verifier = JwtBuilder::new()
            .issuer("test-issuer")
            .audience(&["test-audience"])
            .subject("test-subject")
            .public_key(&Key {
                algorithm: Algorithm::HS256,
                format: KeyFormat::Pem,
                key: KeyData::Data("shared-secret".to_string()),
            })
            .build()
            .unwrap();

        // Construct a client service that adds a JWT token
        let mut client = ServiceBuilder::new()
            .layer(AddJwtLayer::new(signer.clone()))
            .service(TestService);

        // Construct a server service that verifies the JWT token
        let mut server = ServiceBuilder::new()
            .layer(ValidateJwtLayer::new(verifier.clone(), claims.clone()))
            .service(HeaderCheckService);

        // Create a simple client request without Authorization header
        let client_req = Request::builder()
            .uri("https://example.com/api")
            .body(Body::new())
            .unwrap();

        // Send client request through the client service, which should add JWT
        let client_response = client.call(client_req).await.unwrap();
        assert_eq!(client_response.status(), StatusCode::OK);

        // Now create a request that simulates passing through the client service
        // The client service would have added the Authorization header with JWT
        let mut server_req = Request::builder()
            .uri("https://example.com/api")
            .body(Body::new())
            .unwrap();

        // Get a signed token and add it to the request
        let token = signer.sign_standard_claims().unwrap();
        server_req.headers_mut().insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        let ret_claims = verifier
            .try_get_claims::<StandardClaims>(&token)
            .expect("Failed to verify token");

        // Check the claims are as expected
        assert_eq!(ret_claims.iss, Some("test-issuer".to_string()));
        assert_eq!(ret_claims.aud, Some(vec!["test-audience".to_string()]));
        assert_eq!(ret_claims.sub, Some("test-subject".to_string()));

        let custom_claim: u64 = ret_claims
            .custom_claims
            .get("claim1")
            .and_then(|v| v.as_number())
            .and_then(|n| n.as_u64())
            .unwrap();
        assert_eq!(custom_claim, 2u64);

        let claim2_value = ret_claims
            .custom_claims
            .get("claim2")
            .and_then(|v| v.as_map())
            .and_then(|m| m.get("key"))
            .and_then(|v| v.as_str())
            .expect("claim2.key not found or not a string");
        assert_eq!(claim2_value, "value");

        // Send the request through the server service, which should verify JWT
        let server_response = server.call(server_req).await.unwrap();
        assert_eq!(server_response.status(), StatusCode::OK);

        // Test with a malformed Authorization header
        let mut bad_req = Request::builder()
            .uri("https://example.com/api")
            .body(Body::new())
            .unwrap();

        bad_req.headers_mut().insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("NotBearer something"),
        );

        // Server should reject with 401 Unauthorized
        let bad_response = server.call(bad_req).await.unwrap();
        assert_eq!(bad_response.status(), StatusCode::UNAUTHORIZED);
    }
}

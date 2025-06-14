// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use duration_str::deserialize_duration;
use http::HeaderValue;
use http::{Request, Response};
use serde::Deserialize;
use slim_auth::{
    jwt::Jwt,
    traits::{Signer, StandardClaims, Verifier},
};
use std::{
    convert::TryFrom,
    task::{Context, Poll},
};
use tower_http::auth::{AddAuthorizationLayer, require_authorization::Bearer};
use tower_http::validate_request::ValidateRequestHeaderLayer;
use tower_layer::Layer;
use tower_service::Service;

use super::{AuthError, ClientAuthenticator, ServerAuthenticator};
use crate::opaque::OpaqueString;
use slim_auth::jwt::{Algorithm, Key};

#[derive(Clone)]
pub struct SignerLayer<T>
where
    T: Signer + Clone,
{
    signer: T,
    custom_claims: std::collections::HashMap<String, serde_json::Value>,
    duration: u64, // Duration in seconds
}

#[derive(Clone)]
pub struct VerifierLayer<T: Verifier + Clone> {
    verifier: T,
    claims: StandardClaims,
}

impl<T: Signer + Clone> SignerLayer<T> {
    pub fn new(
        signer: T,
        custom_claims: std::collections::HashMap<String, serde_json::Value>,
        duration: u64,
    ) -> Self {
        Self {
            signer,
            custom_claims,
            duration,
        }
    }
}

impl<T: Verifier + Clone> VerifierLayer<T> {
    pub fn new(verifier: T, claims: StandardClaims) -> Self {
        Self { verifier, claims }
    }
}

impl<S, T: Signer + Clone> Layer<S> for SignerLayer<T> {
    type Service = AddJwtToken<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        AddJwtToken {
            inner,
            signer: self.signer.clone(),
            custom_claims: self.custom_claims.clone(),
            cached_token: None,
            valid_until: None,
            duration: self.duration,
        }
    }
}

// impl<S, T: Verifier + Clone> Layer<S> for VerifierLayer<T> {
//     type Service = VerifyJwtToken<S, T>;

//     fn layer(&self, inner: S) -> Self::Service {
//         VerifyJwtToken {
//             inner,
//             verifier: self.verifier.clone(),
//             claims: self.claims.clone(),
//             cached_token: None,
//             cached_claims: None,
//             valid_until: None,
//         }
//     }
// }

#[derive(Clone)]
pub struct AddJwtToken<S, T: Signer + Clone> {
    inner: S,
    signer: T,
    custom_claims: std::collections::HashMap<String, serde_json::Value>,

    cached_token: Option<HeaderValue>,
    valid_until: Option<u64>, // UNIX timestamp in seconds
    duration: u64,            // Duration in seconds
}

impl<S, T: Signer + Clone> AddJwtToken<S, T> {
    /// Get a JWT token, either from cache or by signing a new one
    pub fn get_token(&mut self) -> Result<HeaderValue, AuthError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| AuthError::ConfigError(format!("Failed to get current time: {}", e)))?
            .as_secs();

        if let Some(cached_token) = &self.cached_token {
            if let Some(valid_until) = self.valid_until {
                // We sign a new token if the cached token is about to expire in less than 2/3 of its lifetime
                let remaining = valid_until - now;
                print!(
                    "remaining: {}, 2/3duration: {}",
                    remaining,
                    self.duration * 2 / 3
                );
                if remaining > self.duration * 2 / 3 {
                    return Ok(cached_token.clone());
                }
            }
        }

        // Update claims with current time values
        let claims = self
            .signer
            .create_standard_claims(Some(self.custom_claims.clone()));

        let token = self
            .signer
            .sign(&claims)
            .map_err(|e| AuthError::SigningError(e.to_string()))?;

        let header_value = HeaderValue::try_from(format!("Bearer {}", token))
            .map_err(|e| AuthError::InvalidHeader(e.to_string()))?;

        self.cached_token = Some(header_value.clone());
        self.valid_until = Some(now + self.duration);

        Ok(header_value)
    }
}

impl<S, T, ReqBody, ResBody> Service<Request<ReqBody>> for AddJwtToken<S, T>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    T: Signer + Clone,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        if let Ok(token) = self.get_token() {
            req.headers_mut().insert(http::header::AUTHORIZATION, token);
        }
        // Even if we fail to get a token, we still proceed with the request
        // as the downstream service may handle the lack of authorization
        self.inner.call(req)
    }
}

// struct VerifyJwtTokenInternal<S, T: Verifier + Clone> {
//     inner: S,
//     verifier: T,
//     claims: StandardClaims,

//     // Caching fields
//     cached_token: Option<String>,
//     cached_claims: Option<StandardClaims>,
//     valid_until: Option<u64>, // UNIX timestamp in seconds
// }

// #[derive(Clone)]
// pub struct VerifyJwtToken<S, T: Verifier + Clone> {
//     inner: S,
//     verifier: T,
//     claims: StandardClaims,

//     // Caching fields
//     cached_token: Option<String>,
//     cached_claims: Option<StandardClaims>,
//     valid_until: Option<u64>, // UNIX timestamp in seconds
// }

// impl<S, T: Verifier + Clone> VerifyJwtToken<S, T> {
//     /// Check if a token is cached and still valid
//     fn check_token_cache(&self, token: &str) -> Option<StandardClaims> {
//         // Get current time
//         let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
//             Ok(n) => n.as_secs(),
//             Err(_) => return None,
//         };

//         // Check if the token matches our cached token and is still valid
//         if let Some(cached_token) = &self.cached_token {
//             if *cached_token == token {
//                 // Token matches, check if it's still valid
//                 if let Some(valid_until) = self.valid_until {
//                     if now < valid_until {
//                         // Token is cached and still valid
//                         if let Some(cached_claims) = &self.cached_claims {
//                             return Some(cached_claims.clone());
//                         }
//                     }
//                 }
//             }
//         }

//         None
//     }

//     /// Verify a token and update the cache
//     async fn verify_and_cache_token(&mut self, token: &str) -> Result<StandardClaims, String> {
//         // Verify the token
//         let claims: StandardClaims = self
//             .verifier
//             .verify(token)
//             .await
//             .map_err(|e| format!("Token verification failed: {}", e))?;

//         // Cache the token and claims
//         self.cached_token = Some(token.to_string());
//         self.cached_claims = Some(claims.clone());

//         // Set the valid_until timestamp from the expiration claim
//         self.valid_until = Some(claims.exp);

//         Ok(claims)
//     }
// }

#[cfg(test)]
mod tests {
    use futures::future::{self, Ready};
    use http::{Request, Response, StatusCode};
    use slim_auth::builder::JwtBuilder;
    use slim_auth::jwt::{Algorithm, Jwt};
    use slim_auth::traits::Claimer;
    use std::collections::HashMap;
    use tower::{Service, ServiceBuilder, ServiceExt};

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
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS256, "test-key")
            .build()
            .unwrap();

        let claims = std::collections::HashMap::<String, serde_json::Value>::new();
        let duration = 3600; // 1 hour

        // Create our test service with the JWT signer layer
        let mut service = ServiceBuilder::new()
            .layer(SignerLayer::new(signer, claims, duration))
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

    #[tokio::test]
    async fn test_jwt_token_caching() {
        // Set up a JWT signer
        let signer = JwtBuilder::new()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS256, "test-key")
            .build()
            .unwrap();

        let claims = std::collections::HashMap::<String, serde_json::Value>::new();
        let duration = 3600; // 1 hour

        // Create our AddJwtToken service directly to test token caching
        let mut add_jwt = AddJwtToken {
            inner: TestService,
            signer: signer.clone(),
            custom_claims: claims,
            cached_token: None,
            valid_until: None,
            duration,
        };

        // Get a token
        let token1 = add_jwt.get_token().unwrap();

        // Change the custom claims to force a new token.
        // This should not happen at runtime, but we do it here for testing.
        add_jwt
            .custom_claims
            .insert("new_claim".to_string(), "value".into());

        // Get another token - should be the same cached token
        let token2 = add_jwt.get_token().unwrap();
        assert_eq!(token1, token2);

        // Manually expire the token
        add_jwt.valid_until = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + duration / 4,
        );

        // Get another token - should be a new token due to imminent expiry
        let token3 = add_jwt.get_token().unwrap();
        assert_ne!(token1, token3);
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

    // #[tokio::test]
    // async fn test_jwt_verification() {
    //     // Set up a JWT signer and verifier with the same key
    //     let signer = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .private_key(Algorithm::HS256, "shared-secret")
    //         .build()
    //         .unwrap();

    //     let verifier = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .public_key(Algorithm::HS256, "shared-secret")
    //         .build()
    //         .unwrap();

    //     // Create claims and token
    //     let claims = signer.create_standard_claims(None);
    //     let token = signer.sign(&claims).unwrap();

    //     // Create a request with the token
    //     let req = Request::builder()
    //         .uri("https://example.com")
    //         .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
    //         .body(Body::new())
    //         .unwrap();

    //     // Create our test service with the JWT verifier layer
    //     let mut service = ServiceBuilder::new()
    //         .layer(VerifierLayer::new(verifier.clone(), claims.clone()))
    //         .service(TestService);

    //     // Service should verify the JWT and return a 200 OK
    //     let response = service.call(req).await.unwrap();
    //     assert_eq!(response.status(), StatusCode::OK);

    //     // Test with an invalid token
    //     let req = Request::builder()
    //         .uri("https://example.com")
    //         .header(http::header::AUTHORIZATION, "Bearer invalid.token")
    //         .body(Body::new())
    //         .unwrap();

    //     // Service should reject the invalid token with 401 Unauthorized
    //     let response = service.call(req).await.unwrap();
    //     assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // }

    // #[tokio::test]
    // async fn test_token_caching() {
    //     // Set up a JWT verifier
    //     let verifier = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .public_key(Algorithm::HS256, "test-key")
    //         .build()
    //         .unwrap();

    //     // Create standard claims with a 10-second expiration from now
    //     let now = std::time::SystemTime::now()
    //         .duration_since(std::time::UNIX_EPOCH)
    //         .unwrap()
    //         .as_secs();
    //     let mut standard_claims = StandardClaims {
    //         iss: Some("test-issuer".to_string()),
    //         sub: Some("test-subject".to_string()),
    //         aud: Some("test-audience".to_string()),
    //         exp: now + 10, // Expires in 10 seconds
    //         iat: Some(now),
    //         nbf: Some(now),
    //         jti: None,
    //         custom_claims: std::collections::HashMap::new(),
    //     };

    //     // Set up a JWT signer with the same key as the verifier
    //     let signer = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .private_key(Algorithm::HS256, "test-key")
    //         .build()
    //         .unwrap();

    //     // Sign the token
    //     let token = signer.sign(&standard_claims).unwrap();
    //     let auth_header = format!("Bearer {}", token);

    //     // Create our service with the verifier
    //     let service = ServiceBuilder::new()
    //         .layer(VerifierLayer::new(
    //             verifier.clone(),
    //             standard_claims.clone(),
    //         ))
    //         .service(TestService);

    //     // Create a test request with the signed token
    //     let req1 = Request::builder()
    //         .uri("https://example.com")
    //         .header(http::header::AUTHORIZATION, &auth_header)
    //         .body(Body::new())
    //         .unwrap();

    //     // Service should accept the valid token
    //     let response1 = ServiceExt::<Request<Body>>::oneshot(service.clone(), req1)
    //         .await
    //         .unwrap();
    //     assert_eq!(response1.status(), StatusCode::OK);

    //     // Make a second request with the same token
    //     // This should use the cached verification result
    //     let req2 = Request::builder()
    //         .uri("https://example.com")
    //         .header(http::header::AUTHORIZATION, &auth_header)
    //         .body(Body::new())
    //         .unwrap();

    //     // Service should accept the cached token
    //     let response2 = ServiceExt::<Request<Body>>::oneshot(service.clone(), req2)
    //         .await
    //         .unwrap();
    //     assert_eq!(response2.status(), StatusCode::OK);

    //     // Create a test with an expired token
    //     let mut expired_claims = standard_claims.clone();
    //     expired_claims.exp = now - 10; // Expired 10 seconds ago

    //     // Sign the expired token
    //     let expired_token = signer.sign(&expired_claims).unwrap();
    //     let expired_auth_header = format!("Bearer {}", expired_token);

    //     // Create a request with the expired token
    //     let req3 = Request::builder()
    //         .uri("https://example.com")
    //         .header(http::header::AUTHORIZATION, &expired_auth_header)
    //         .body(Body::new())
    //         .unwrap();

    //     // Service should reject the expired token
    //     let response3 = ServiceExt::<Request<Body>>::oneshot(service.clone(), req3)
    //         .await
    //         .unwrap();
    //     assert_eq!(response3.status(), StatusCode::UNAUTHORIZED);
    // }

    // #[tokio::test]
    // async fn test_end_to_end() {
    //     // Set up a JWT signer and verifier with the same key
    //     let signer = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .private_key(Algorithm::HS256, "shared-secret")
    //         .build()
    //         .unwrap();

    //     let verifier = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .public_key(Algorithm::HS256, "shared-secret")
    //         .build()
    //         .unwrap();

    //     // Create standard claims
    //     let claims = signer.create_standard_claims(None);

    //     // Construct a client service that adds a JWT token
    //     let client = ServiceBuilder::new()
    //         .layer(SignerLayer::new(signer.clone(), claims.clone(), 3600))
    //         .service(TestService);

    //     // Construct a server service that verifies the JWT token
    //     let server = ServiceBuilder::new()
    //         .layer(VerifierLayer::new(verifier.clone(), claims.clone()))
    //         .service(HeaderCheckService);

    //     // Create a simple client request without Authorization header
    //     let client_req = Request::builder()
    //         .uri("https://example.com/api")
    //         .body(Body::new())
    //         .unwrap();

    //     // Send client request through the client service, which should add JWT
    //     let client_response = client.clone().oneshot(client_req).await.unwrap();
    //     assert_eq!(client_response.status(), StatusCode::OK);

    //     // Now create a request that simulates passing through the client service
    //     // The client service would have added the Authorization header with JWT
    //     let mut server_req = Request::builder()
    //         .uri("https://example.com/api")
    //         .body(Body::new())
    //         .unwrap();

    //     // Get a signed token and add it to the request
    //     let token = signer.sign(&claims).unwrap();
    //     server_req.headers_mut().insert(
    //         http::header::AUTHORIZATION,
    //         HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
    //     );

    //     // Send the request through the server service, which should verify JWT
    //     let server_response = server.oneshot(server_req).await.unwrap();
    //     assert_eq!(server_response.status(), StatusCode::OK);

    //     // Test with a malformed Authorization header
    //     let mut bad_req = Request::builder()
    //         .uri("https://example.com/api")
    //         .body(Body::new())
    //         .unwrap();

    //     bad_req.headers_mut().insert(
    //         http::header::AUTHORIZATION,
    //         HeaderValue::from_static("NotBearer something"),
    //     );

    //     // Server should reject with 401 Unauthorized
    //     let bad_response = server.oneshot(bad_req).await.unwrap();
    //     assert_eq!(bad_response.status(), StatusCode::UNAUTHORIZED);
    // }

    // #[tokio::test]
    // async fn test_custom_claims() {
    //     // Set up a JWT signer
    //     let signer = JwtBuilder::new()
    //         .issuer("test-issuer")
    //         .audience("test-audience")
    //         .subject("test-subject")
    //         .private_key(Algorithm::HS256, "test-key")
    //         .build()
    //         .unwrap();

    //     // Create custom claims
    //     let mut custom_claims = HashMap::new();
    //     custom_claims.insert(
    //         "role".to_string(),
    //         serde_json::Value::String("admin".to_string()),
    //     );

    //     let mut claims = signer.create_standard_claims(Some(custom_claims));

    //     // Create our AddJwtToken service directly
    //     let mut add_jwt = AddJwtToken {
    //         inner: TestService,
    //         signer: signer.clone(),
    //         claims,
    //         cached_token: None,
    //         valid_until: None,
    //         duration: 3600,
    //     };

    //     // Get a token and extract it to verify the custom claims are included
    //     let token_header = add_jwt.get_token().unwrap();
    //     let token_str = token_header.to_str().unwrap().trim_start_matches("Bearer ");

    //     // Split the token to get the payload part
    //     let parts: Vec<&str> = token_str.split('.').collect();
    //     assert_eq!(parts.len(), 3, "JWT should have three parts");

    //     // Decode the payload (second part)
    //     let payload = base64::decode_config(parts[1], base64::URL_SAFE_NO_PAD).unwrap();
    //     let payload_json: serde_json::Value = serde_json::from_slice(&payload).unwrap();

    //     // Verify the custom claim is included in the token
    //     assert_eq!(
    //         payload_json.get("role").and_then(|v| v.as_str()),
    //         Some("admin"),
    //         "Custom claim 'role' should be in the token payload"
    //     );
    // }
}

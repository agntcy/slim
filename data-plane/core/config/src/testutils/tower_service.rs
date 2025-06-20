// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use futures::future::{self, Ready};
use futures::task::Poll;
use http::{Request, Response, StatusCode};
use std::task::Context;
use tower::Service;

// Define a Body type for testing
pub type Body = Vec<u8>;

// A simple test service that returns a 200 OK response
#[derive(Clone)]
pub struct HeaderCheckService;
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

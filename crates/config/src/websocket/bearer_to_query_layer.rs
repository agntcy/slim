// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Client-side counterpart of [`QueryTokenToAuthHeaderLayer`]: mirrors an
//! `Authorization: Bearer …` header into a `?<param>=<token>` query parameter
//! before the request is sent.
//!
//! Used when the WebSocket client is configured with
//! `ClientConfig::websocket_auth_query_param`. Browser clients constructed
//! via `new WebSocket(url)` cannot carry custom headers; the server's
//! `QueryTokenToAuthHeaderLayer` reads the token back off the query string.
//! For non-browser clients (Rust, Go, etc.) we still send the Authorization
//! header, but mirror the token into the URI so a single deployment can
//! accept both kinds of clients.
//!
//! Only `Bearer` tokens are mirrored — there is no sensible way to encode a
//! `Basic` credential as a single query parameter, and Basic auth predates
//! browser WebSocket clients anyway.

use std::str::FromStr;
use std::task::{Context, Poll};

use http::Request;
use http::header::AUTHORIZATION;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use tower::{Layer, Service};

const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

#[derive(Clone)]
pub struct BearerToQueryLayer {
    param_name: Option<String>,
}

impl BearerToQueryLayer {
    pub fn new(param_name: impl Into<String>) -> Self {
        Self {
            param_name: Some(param_name.into()),
        }
    }

    /// No-op variant — used when `websocket_auth_query_param` is unset so the
    /// same `ServiceBuilder` chain works without conditional types.
    pub fn passthrough() -> Self {
        Self { param_name: None }
    }
}

impl<S> Layer<S> for BearerToQueryLayer {
    type Service = BearerToQuery<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BearerToQuery {
            inner,
            param_name: self.param_name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BearerToQuery<S> {
    inner: S,
    param_name: Option<String>,
}

impl<S, B> Service<Request<B>> for BearerToQuery<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        if let Some(param) = self.param_name.as_deref()
            && let Some(token) = bearer_token(&req)
            && let Some(new_uri) = inject_token_into_uri(req.uri(), param, &token)
        {
            *req.uri_mut() = new_uri;
        }
        self.inner.call(req)
    }
}

fn bearer_token<B>(req: &Request<B>) -> Option<String> {
    let value = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.to_string())
}

fn inject_token_into_uri(uri: &http::Uri, param: &str, token: &str) -> Option<http::Uri> {
    if param.is_empty() || token.is_empty() {
        return None;
    }
    let mut s = String::new();
    if let Some(scheme) = uri.scheme_str() {
        s.push_str(scheme);
        s.push_str("://");
    }
    if let Some(authority) = uri.authority() {
        s.push_str(authority.as_str());
    }
    s.push_str(uri.path());
    if let Some(q) = uri.query() {
        s.push('?');
        s.push_str(q);
        s.push('&');
    } else {
        s.push('?');
    }
    for chunk in utf8_percent_encode(param, QUERY_ENCODE_SET) {
        s.push_str(chunk);
    }
    s.push('=');
    for chunk in utf8_percent_encode(token, QUERY_ENCODE_SET) {
        s.push_str(chunk);
    }
    http::Uri::from_str(&s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::Infallible;

    use http::{Request, Response, StatusCode};
    use tower::{ServiceBuilder, ServiceExt, service_fn};

    async fn echo_uri(req: Request<()>) -> Result<Response<String>, Infallible> {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(req.uri().to_string())
            .unwrap())
    }

    async fn call_with(layer: BearerToQueryLayer, req: Request<()>) -> String {
        let mut s = ServiceBuilder::new()
            .layer(layer)
            .service(service_fn(echo_uri));
        let resp = s.ready().await.unwrap().call(req).await.unwrap();
        resp.into_body()
    }

    #[tokio::test]
    async fn no_bearer_param_configured_passes_through() {
        let req = Request::builder().uri("http://x/y").body(()).unwrap();
        assert_eq!(
            call_with(BearerToQueryLayer::new("token"), req).await,
            "http://x/y"
        );
    }

    #[tokio::test]
    async fn bearer_no_param_passes_through() {
        let req = Request::builder()
            .uri("http://x/y")
            .header(AUTHORIZATION, "Bearer abc")
            .body(())
            .unwrap();
        assert_eq!(
            call_with(BearerToQueryLayer::passthrough(), req).await,
            "http://x/y"
        );
    }

    #[tokio::test]
    async fn bearer_with_param_injects_query() {
        let req = Request::builder()
            .uri("http://x/y")
            .header(AUTHORIZATION, "Bearer abc")
            .body(())
            .unwrap();
        assert_eq!(
            call_with(BearerToQueryLayer::new("token"), req).await,
            "http://x/y?token=abc"
        );
    }

    #[tokio::test]
    async fn bearer_with_existing_query_appends() {
        let req = Request::builder()
            .uri("http://x/y?k=v")
            .header(AUTHORIZATION, "Bearer abc")
            .body(())
            .unwrap();
        assert_eq!(
            call_with(BearerToQueryLayer::new("token"), req).await,
            "http://x/y?k=v&token=abc"
        );
    }

    #[tokio::test]
    async fn basic_auth_is_not_mirrored() {
        let req = Request::builder()
            .uri("http://x/y")
            .header(AUTHORIZATION, "Basic dXNlcjpwYXNz")
            .body(())
            .unwrap();
        assert_eq!(
            call_with(BearerToQueryLayer::new("token"), req).await,
            "http://x/y"
        );
    }

    #[tokio::test]
    async fn token_with_special_chars_is_percent_encoded() {
        let req = Request::builder()
            .uri("http://x/y")
            .header(AUTHORIZATION, "Bearer a+b=c&d")
            .body(())
            .unwrap();
        let out = call_with(BearerToQueryLayer::new("token"), req).await;
        assert!(out.starts_with("http://x/y?token="), "got {out}");
        assert!(!out.contains("a+b=c&d"), "got {out}");
    }
}

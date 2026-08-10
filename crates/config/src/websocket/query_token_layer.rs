// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Promotes a `?token=<jwt>` query parameter to an `Authorization: Bearer …`
//! header so the shared `ValidateJwtLayer` (which only inspects the
//! Authorization header) can authenticate browser WebSocket clients —
//! browsers cannot set custom headers when constructing `new WebSocket(url)`.
//!
//! If the request already carries an Authorization header, it wins and the
//! query parameter is ignored. If no token is present anywhere, the request
//! passes through unchanged and the downstream JWT layer will reject it.

use std::task::{Context, Poll};

use http::{Request, header};
use tower::{Layer, Service};

use crate::websocket::common::extract_query_param;

/// Rebuild `uri` with the `token` query parameter removed.
fn strip_token_param(uri: &http::Uri) -> http::Uri {
    let query = match uri.query() {
        Some(q) => q,
        None => return uri.clone(),
    };
    let filtered: String = query
        .split('&')
        .filter(|seg| {
            let key = seg.split_once('=').map(|(k, _)| k).unwrap_or(seg);
            key != "token"
        })
        .collect::<Vec<_>>()
        .join("&");
    let path = uri.path();
    let new_pq: http::uri::PathAndQuery = if filtered.is_empty() {
        path.parse().unwrap_or_else(|_| "/".parse().unwrap())
    } else {
        format!("{}?{}", path, filtered)
            .parse()
            .unwrap_or_else(|_| path.parse().unwrap())
    };
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(new_pq);
    http::Uri::from_parts(parts).unwrap_or_else(|_| uri.clone())
}

#[derive(Clone, Default)]
pub struct QueryTokenToAuthHeaderLayer;

impl QueryTokenToAuthHeaderLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for QueryTokenToAuthHeaderLayer {
    type Service = QueryTokenToAuthHeader<S>;

    fn layer(&self, inner: S) -> Self::Service {
        QueryTokenToAuthHeader { inner }
    }
}

#[derive(Clone)]
pub struct QueryTokenToAuthHeader<S> {
    inner: S,
}

impl<S, B> Service<Request<B>> for QueryTokenToAuthHeader<S>
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
        if !req.headers().contains_key(header::AUTHORIZATION)
            && let Some(token) = extract_query_param(req.uri().query(), "token")
            && let Ok(value) = header::HeaderValue::from_str(&format!("Bearer {token}"))
        {
            req.headers_mut().insert(header::AUTHORIZATION, value);
            *req.uri_mut() = strip_token_param(req.uri());
        }
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::Infallible;

    use http::{Request, Response, StatusCode};
    use tower::{ServiceBuilder, ServiceExt, service_fn};

    async fn echo_auth(req: Request<()>) -> Result<Response<String>, Infallible> {
        let auth = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // Return "<auth>|<uri>" so tests can assert both.
        let body = format!("{}|{}", auth, req.uri());
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(body)
            .unwrap())
    }

    async fn call_with(req: Request<()>) -> (String, String) {
        let mut s = ServiceBuilder::new()
            .layer(QueryTokenToAuthHeaderLayer::new())
            .service(service_fn(echo_auth));
        let body = s
            .ready()
            .await
            .unwrap()
            .call(req)
            .await
            .unwrap()
            .into_body();
        let (auth, uri) = body.split_once('|').unwrap_or(("", &body));
        (auth.to_string(), uri.to_string())
    }

    #[tokio::test]
    async fn no_header_no_query_passes_through() {
        let req = Request::builder().uri("/").body(()).unwrap();
        let (auth, uri) = call_with(req).await;
        assert_eq!(auth, "");
        assert_eq!(uri, "/");
    }

    #[tokio::test]
    async fn query_token_promoted_to_header_and_stripped_from_uri() {
        let req = Request::builder().uri("/?token=abc").body(()).unwrap();
        let (auth, uri) = call_with(req).await;
        assert_eq!(auth, "Bearer abc");
        assert_eq!(uri, "/");
    }

    #[tokio::test]
    async fn token_stripped_other_params_preserved() {
        let req = Request::builder()
            .uri("/?foo=1&token=abc&bar=2")
            .body(())
            .unwrap();
        let (auth, uri) = call_with(req).await;
        assert_eq!(auth, "Bearer abc");
        assert_eq!(uri, "/?foo=1&bar=2");
    }

    #[tokio::test]
    async fn existing_header_wins_over_query() {
        let req = Request::builder()
            .uri("/?token=fromquery")
            .header(header::AUTHORIZATION, "Bearer fromheader")
            .body(())
            .unwrap();
        let (auth, uri) = call_with(req).await;
        assert_eq!(auth, "Bearer fromheader");
        // URI unchanged when header already present
        assert_eq!(uri, "/?token=fromquery");
    }

    #[tokio::test]
    async fn percent_encoded_token_is_decoded() {
        let req = Request::builder().uri("/?token=ab%20c").body(()).unwrap();
        let (auth, uri) = call_with(req).await;
        assert_eq!(auth, "Bearer ab c");
        assert_eq!(uri, "/");
    }
}

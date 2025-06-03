// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    fmt,
    task::{Context, Poll},
};

use http::{Request, Response, header::HeaderMap};
use tower_layer::Layer;
use tower_service::Service;

/// Layer that sets a header map on the request.
#[derive(Clone)]
pub struct SetRequestHeaderLayer {
    header_map: HeaderMap,
}

/// Middleware that sets a header on the request.
impl fmt::Debug for SetRequestHeaderLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetRequestHeaderLayer")
            .field("header_map", &self.header_map)
            .finish()
    }
}

impl SetRequestHeaderLayer {
    /// Create a new `SetRequestHeaderLayer` that sets the provided header map on the request.
    pub fn new(header_map: HeaderMap) -> Self {
        Self { header_map }
    }
}

/// Implement `Layer` for `SetRequestHeaderLayer`.
/// This will allow the `SetRequestHeaderLayer` to be used as a middleware.
impl<S> Layer<S> for SetRequestHeaderLayer {
    type Service = SetRequestHeader<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SetRequestHeader {
            inner,
            header_map: self.header_map.clone(),
        }
    }
}

/// Middleware that sets a header on the request.
#[derive(Clone)]
pub struct SetRequestHeader<S> {
    /// The inner service.
    inner: S,

    /// The header map to set on the request.
    header_map: HeaderMap,
}

/// Implement `Service` for `SetRequestHeader`.
impl<S> SetRequestHeader<S> {
    /// Apply the header map to the request.
    pub fn apply<B>(&self, req: &mut Request<B>) {
        for (key, value) in self.header_map.iter() {
            req.headers_mut().insert(key, value.clone());
        }
    }
}

/// Implement `Debug` for `SetRequestHeader`.
impl<S> fmt::Debug for SetRequestHeader<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetRequestHeader")
            .field("inner", &self.inner)
            .field("header_map", &self.header_map)
            .finish()
    }
}

/// Implement `Service` for `SetRequestHeader`.
impl<ReqBody, ResBody, S> Service<Request<ReqBody>> for SetRequestHeader<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    /// Poll the inner service.
    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    /// Apply the header map to the request and call the inner service.
    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        self.apply(&mut req);
        self.inner.call(req)
    }
}

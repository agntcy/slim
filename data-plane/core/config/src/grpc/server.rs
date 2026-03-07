// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub use crate::server::*;

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use tokio_util::sync::CancellationToken;

use super::errors::ConfigError;

/// ServerFuture is a type alias for a boxed future that returns a tonic server result.
type ServerFuture = Pin<Box<dyn Future<Output = Result<(), tonic::transport::Error>> + Send>>;

impl ServerConfig {
    pub async fn to_server_future<S>(&self, svc: &[S]) -> Result<ServerFuture, ConfigError>
    where
        S: tower_service::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            >
            + tonic::server::NamedService
            + Clone
            + Send
            + 'static
            + Sync,
        S::Future: Send + 'static,
    {
        self.to_grpc_server_future(svc).await
    }

    pub async fn run_server<S>(
        &self,
        svc: &[S],
        drain_rx: drain::Watch,
    ) -> Result<CancellationToken, ConfigError>
    where
        S: tower_service::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            >
            + tonic::server::NamedService
            + Clone
            + Send
            + 'static
            + Sync,
        S::Future: Send + 'static,
    {
        self.run_grpc_server(svc, drain_rx).await
    }
}

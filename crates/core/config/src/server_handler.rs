// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic server handler.
//!
//! Implementations supply only the transport adapters they care about; the
//! correct one is picked by [`crate::server::ServerConfig::run_server`] based
//! on `config.transport`. Adding a new transport means adding a method to
//! this trait — existing handlers stay source-compatible thanks to the
//! `None` defaults.

use crate::websocket::server::OnAcceptedWebSocket;

pub trait ServerHandler: Send + Sync + 'static {
    /// gRPC transport: tonic `Routes` bundle to install on the server.
    /// Default `None` means "this handler does not support gRPC" — running
    /// against a gRPC `ServerConfig` will then surface
    /// `ConfigError::HandlerMissingGrpcSupport`.
    fn grpc_routes(&self) -> Option<tonic::service::Routes> {
        None
    }

    /// WebSocket transport: per-connection callback invoked after a successful
    /// upgrade. Same `None` semantics as [`Self::grpc_routes`].
    fn on_websocket_accepted(&self) -> Option<OnAcceptedWebSocket> {
        None
    }
}

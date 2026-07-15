// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling — UniFFI wrapper.
//!
//! [`Server`] is a thin newtype over [`slim_rpc::Server`]. All the request
//! dispatch logic lives in the `agntcy-slim-rpc` crate; this wrapper only
//! adapts the public UniFFI API: constructors take FFI `crate::App` /
//! `crate::Name` values and unwrap them into the native SLIM types, and every
//! other method delegates to the inner server.

use std::sync::Arc;

use super::{
    RpcError, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler,
};

/// RPC server for handling incoming requests over SLIM.
///
/// See [`slim_rpc::Server`] for the full behaviour. This is a UniFFI-exported
/// newtype wrapper that delegates every call to the inner server.
#[derive(uniffi::Object)]
pub struct Server(slim_rpc::Server);

impl Server {
    /// List the registered `service/method` handler paths.
    pub fn methods(&self) -> Vec<String> {
        self.0.methods()
    }
}

#[uniffi::export]
impl Server {
    /// Create a new RPC server.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance that provides the underlying
    ///   network transport and session management
    /// * `base_name` - The base name for this service (e.g., org.namespace.service).
    ///   This name is used to construct subscription names for RPC methods.
    #[uniffi::constructor]
    pub fn new(app: &Arc<crate::App>, base_name: Arc<crate::Name>) -> Self {
        Self::new_with_connection(app, base_name, None)
    }

    /// Create a new RPC server with an optional connection ID.
    ///
    /// The connection ID is used to set up routing before serving RPC requests,
    /// enabling multi-hop RPC calls through specific connections.
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: &Arc<crate::App>,
        base_name: Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let app_inner = app.inner();
        let rx = app.notification_receiver();

        Self(slim_rpc::Server::new_with_shared_rx_and_connection(
            app_inner,
            base_name.as_ref().into(),
            connection_id,
            rx,
            Some(crate::get_runtime()),
        ))
    }

    /// Register a unary-to-unary RPC handler.
    pub fn register_unary_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryUnaryHandler>,
    ) {
        self.0
            .register_unary_unary(service_name, method_name, handler);
    }

    /// Register a unary-to-stream RPC handler.
    pub fn register_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryStreamHandler>,
    ) {
        self.0
            .register_unary_stream(service_name, method_name, handler);
    }

    /// Register a stream-to-unary RPC handler.
    pub fn register_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamUnaryHandler>,
    ) {
        self.0
            .register_stream_unary(service_name, method_name, handler);
    }

    /// Register a stream-to-stream RPC handler.
    pub fn register_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamStreamHandler>,
    ) {
        self.0
            .register_stream_stream(service_name, method_name, handler);
    }

    /// Start serving RPC requests (blocking version).
    pub fn serve(&self) -> Result<(), RpcError> {
        self.0.serve()
    }

    /// Start serving RPC requests (async version).
    pub async fn serve_async(&self) -> Result<(), RpcError> {
        self.0.serve_async().await
    }

    /// Shutdown the server gracefully (blocking version).
    pub fn shutdown(&self) {
        self.0.shutdown();
    }

    /// Shutdown the server gracefully (async version).
    pub async fn shutdown_async(&self) {
        self.0.shutdown_async().await;
    }
}

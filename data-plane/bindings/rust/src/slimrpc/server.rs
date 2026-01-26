// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Server bindings for UniFFI
//!
//! Provides a UniFFI-compatible wrapper around the core SlimRPC Server type.

use std::sync::Arc;

use crate::errors::SlimError;
use crate::{App, Name, get_runtime};

use agntcy_slimrpc::Server as CoreServer;

use super::context::RpcContext;
use super::error::Status;
use super::rpc::{
    RequestStreamWrapper, ResponseStreamSender, StreamStreamHandler, StreamUnaryHandler,
    UnaryStreamHandler, UnaryUnaryHandler,
};

/// Handler type for RPC methods (UniFFI-compatible enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HandlerType {
    /// Unary request, unary response
    UnaryUnary,
    /// Unary request, streaming response
    UnaryStream,
    /// Streaming request, unary response
    StreamUnary,
    /// Streaming request, streaming response
    StreamStream,
}

impl From<HandlerType> for agntcy_slimrpc::HandlerType {
    fn from(handler_type: HandlerType) -> Self {
        match handler_type {
            HandlerType::UnaryUnary => agntcy_slimrpc::HandlerType::UnaryUnary,
            HandlerType::UnaryStream => agntcy_slimrpc::HandlerType::UnaryStream,
            HandlerType::StreamUnary => agntcy_slimrpc::HandlerType::StreamUnary,
            HandlerType::StreamStream => agntcy_slimrpc::HandlerType::StreamStream,
        }
    }
}

impl From<agntcy_slimrpc::HandlerType> for HandlerType {
    fn from(handler_type: agntcy_slimrpc::HandlerType) -> Self {
        match handler_type {
            agntcy_slimrpc::HandlerType::UnaryUnary => HandlerType::UnaryUnary,
            agntcy_slimrpc::HandlerType::UnaryStream => HandlerType::UnaryStream,
            agntcy_slimrpc::HandlerType::StreamUnary => HandlerType::StreamUnary,
            agntcy_slimrpc::HandlerType::StreamStream => HandlerType::StreamStream,
        }
    }
}

/// Response type from RPC handlers (UniFFI-compatible enum)
#[derive(Debug, Clone, uniffi::Enum)]
pub enum HandlerResponse {
    /// Single response
    Unary { data: Vec<u8> },
    /// Streaming response (list of responses)
    Stream { data: Vec<Vec<u8>> },
}

/// Type alias for RPC handler function (Rust-side only, not exposed via UniFFI)
///
/// UniFFI doesn't support function pointers/callbacks well, so handlers
/// must be registered on the Rust side before exposing the server.
pub type RpcHandler =
    Box<dyn Fn(Vec<u8>, RpcContext) -> Result<Vec<u8>, Status> + Send + Sync + 'static>;

/// Type alias for streaming RPC handler (Rust-side only)
pub type RpcResponseStream = Vec<Result<Vec<u8>, super::error::Status>>;

/// RPC Server (UniFFI-compatible)
///
/// Manages the lifecycle of an RPC server that can handle incoming requests.
/// Handlers can be registered directly on the server before calling serve().
#[derive(uniffi::Object)]
pub struct RpcServer {
    /// The underlying core server
    server: Arc<parking_lot::Mutex<CoreServer>>,
}

#[uniffi::export]
impl RpcServer {
    /// Create a new RPC server
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for the server (service name)
    #[uniffi::constructor]
    pub fn new(app: Arc<App>, base_name: Arc<Name>) -> Arc<Self> {
        let slim_name = base_name.as_slim_name();
        let notification_rx = app.notification_receiver();

        // Create the core server
        let server =
            CoreServer::new_with_shared_rx(app.inner_app().clone(), slim_name, notification_rx);

        Arc::new(Self {
            server: Arc::new(parking_lot::Mutex::new(server)),
        })
    }

    /// Create a new RPC server with connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for the server (service name)
    /// * `connection_id` - Optional connection ID for session propagation
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: Arc<App>,
        base_name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Arc<Self> {
        let slim_name = base_name.as_slim_name();
        let notification_rx = app.notification_receiver();

        // Create the core server
        let server = CoreServer::new_with_shared_rx_and_connection(
            app.inner_app().clone(),
            slim_name,
            connection_id,
            notification_rx,
        );

        Arc::new(Self {
            server: Arc::new(parking_lot::Mutex::new(server)),
        })
    }

    /// Register a unary-unary handler
    ///
    /// # Arguments
    /// * `service_name` - Name of the service (e.g., "MyService")
    /// * `method_name` - Name of the method (e.g., "MyMethod")
    /// * `handler` - Implementation of the UnaryUnaryHandler trait
    pub fn register_unary_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryUnaryHandler>,
    ) {
        let server = self.server.lock();
        server.registry().register_unary_unary(
            &service_name,
            &method_name,
            move |request: Vec<u8>, ctx: agntcy_slimrpc::Context| {
                let handler = Arc::clone(&handler);
                let rpc_ctx = Arc::new(RpcContext::from_core_context(ctx));
                async move {
                    let result = handler.handle(request, rpc_ctx).await;
                    result.map_err(|e| e.into_core_status())
                }
            },
        );
    }

    /// Register a unary-stream handler
    ///
    /// # Arguments
    /// * `service_name` - Name of the service (e.g., "MyService")
    /// * `method_name` - Name of the method (e.g., "MyMethod")
    /// * `handler` - Implementation of the UnaryStreamHandler trait
    pub fn register_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryStreamHandler>,
    ) {
        let server = self.server.lock();
        server.registry().register_unary_stream(
            &service_name,
            &method_name,
            move |request: Vec<u8>, ctx: agntcy_slimrpc::Context| {
                let handler = Arc::clone(&handler);
                let rpc_ctx = Arc::new(RpcContext::from_core_context(ctx));
                async move {
                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                    let response_stream = Arc::new(ResponseStreamSender {
                        tx: Arc::new(tokio::sync::Mutex::new(tx)),
                    });

                    let handler_clone = Arc::clone(&handler);
                    let rpc_ctx_clone = Arc::clone(&rpc_ctx);
                    let response_stream_clone = Arc::clone(&response_stream);

                    // Spawn handler task
                    tokio::spawn(async move {
                        let _ = handler_clone
                            .handle(request, rpc_ctx_clone, response_stream_clone)
                            .await;
                    });

                    // Convert receiver to stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
                    Ok(stream)
                }
            },
        );
    }

    /// Register a stream-unary handler
    ///
    /// # Arguments
    /// * `service_name` - Name of the service (e.g., "MyService")
    /// * `method_name` - Name of the method (e.g., "MyMethod")
    /// * `handler` - Implementation of the StreamUnaryHandler trait
    pub fn register_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamUnaryHandler>,
    ) {
        let server = self.server.lock();
        server.registry().register_stream_unary(
            &service_name,
            &method_name,
            move |stream: std::pin::Pin<
                Box<dyn futures::Stream<Item = Result<Vec<u8>, agntcy_slimrpc::Status>> + Send>,
            >,
                  ctx: agntcy_slimrpc::Context| {
                let handler = Arc::clone(&handler);
                let rpc_ctx = Arc::new(RpcContext::from_core_context(ctx));
                async move {
                    // Wrap the stream in a RequestStream trait object
                    let request_stream = Arc::new(RequestStreamWrapper {
                        stream: Arc::new(tokio::sync::Mutex::new(stream)),
                    });

                    let result = handler.handle(request_stream, rpc_ctx).await;
                    result.map_err(|e| e.into_core_status())
                }
            },
        );
    }

    /// Register a stream-stream handler
    ///
    /// # Arguments
    /// * `service_name` - Name of the service (e.g., "MyService")
    /// * `method_name` - Name of the method (e.g., "MyMethod")
    /// * `handler` - Implementation of the StreamStreamHandler trait
    pub fn register_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamStreamHandler>,
    ) {
        let server = self.server.lock();
        server.registry().register_stream_stream(
            &service_name,
            &method_name,
            move |stream: std::pin::Pin<
                Box<dyn futures::Stream<Item = Result<Vec<u8>, agntcy_slimrpc::Status>> + Send>,
            >,
                  ctx: agntcy_slimrpc::Context| {
                let handler = Arc::clone(&handler);
                let rpc_ctx = Arc::new(RpcContext::from_core_context(ctx));
                async move {
                    // Wrap the request stream
                    let request_stream = Arc::new(RequestStreamWrapper {
                        stream: Arc::new(tokio::sync::Mutex::new(stream)),
                    });

                    // Create response stream
                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                    let response_stream = Arc::new(ResponseStreamSender {
                        tx: Arc::new(tokio::sync::Mutex::new(tx)),
                    });

                    let handler_clone = Arc::clone(&handler);
                    let rpc_ctx_clone = Arc::clone(&rpc_ctx);
                    let request_stream_clone = Arc::clone(&request_stream);
                    let response_stream_clone = Arc::clone(&response_stream);

                    // Spawn handler task
                    tokio::spawn(async move {
                        let _ = handler_clone
                            .handle(request_stream_clone, rpc_ctx_clone, response_stream_clone)
                            .await;
                    });

                    // Convert receiver to stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
                    Ok(stream)
                }
            },
        );
    }

    /// Start serving RPC requests (blocking)
    ///
    /// This method blocks until the server is shut down.
    /// All handlers must be registered before calling this method.
    pub fn serve(&self) -> Result<(), SlimError> {
        let runtime = get_runtime();
        runtime.block_on(self.serve_async())
    }

    /// Start serving RPC requests (async)
    ///
    /// This method returns when the server is shut down.
    /// All handlers must be registered before calling this method.
    pub async fn serve_async(&self) -> Result<(), SlimError> {
        let server = self.server.lock().clone();
        server.serve().await.map_err(|e| SlimError::RpcError {
            message: format!("Server error: {}", e),
        })
    }

    /// Shutdown the server gracefully (blocking)
    pub fn shutdown(&self) {
        let runtime = get_runtime();
        runtime.block_on(self.shutdown_async())
    }

    /// Shutdown the server gracefully (async)
    pub async fn shutdown_async(&self) {
        let server = self.server.lock().clone();
        server.shutdown().await
    }

    /// Get a list of all registered method names
    ///
    /// Returns method names in the format "ServiceName/MethodName"
    pub fn methods(&self) -> Vec<String> {
        self.server.lock().registry().methods()
    }
}

impl Clone for RpcServer {
    fn clone(&self) -> Self {
        Self {
            server: self.server.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_type_conversion() {
        let unary_unary = HandlerType::UnaryUnary;
        let core: agntcy_slimrpc::HandlerType = unary_unary.into();
        let back: HandlerType = core.into();
        assert_eq!(back, HandlerType::UnaryUnary);

        let stream_stream = HandlerType::StreamStream;
        let core: agntcy_slimrpc::HandlerType = stream_stream.into();
        let back: HandlerType = core.into();
        assert_eq!(back, HandlerType::StreamStream);
    }

    #[test]
    fn test_handler_response_enum() {
        let unary = HandlerResponse::Unary {
            data: vec![1, 2, 3],
        };
        match unary {
            HandlerResponse::Unary { data } => assert_eq!(data, vec![1, 2, 3]),
            _ => panic!("Expected Unary variant"),
        }

        let stream = HandlerResponse::Stream {
            data: vec![vec![1, 2], vec![3, 4]],
        };
        match stream {
            HandlerResponse::Stream { data } => assert_eq!(data.len(), 2),
            _ => panic!("Expected Stream variant"),
        }
    }
}

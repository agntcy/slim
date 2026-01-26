// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server wrapper for SlimRPC UniFFI bindings
//!
//! Provides a UniFFI-compatible server interface that wraps the core SlimRPC server
//! and bridges foreign language handler implementations.

use std::sync::Arc;

use tokio_stream::wrappers::UnboundedReceiverStream;

use slim_rpc::{HandlerResponse as CoreHandlerResponse, Server as CoreServer};
use slim_session::notification::Notification;
use slim_session::errors::SessionError;

use crate::slimrpc::context::Context;
use crate::slimrpc::error::RpcError;
use crate::slimrpc::handler::{
    StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler,
};
use crate::slimrpc::types::{RequestStream, ResponseSink};
use crate::{App, Name};

/// RPC Server for handling incoming RPC calls
///
/// Wraps the core SlimRPC server and provides UniFFI-compatible registration
/// and serving methods.
#[derive(Clone, uniffi::Object)]
pub struct Server {
    /// Wrapped core server
    inner: CoreServer,
}

impl Server {
    /// Create a new RPC server
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance
    /// * `base_name` - Base name for the service (e.g., org.namespace.service)
    /// * `notification_rx` - Channel receiver for session notifications
    ///
    /// # Returns
    /// A new server instance
    ///
    /// # Note
    /// This constructor is not exposed through UniFFI since tokio::sync::mpsc::Receiver
    /// cannot be passed through FFI. For language bindings, servers should be created
    /// through language-specific factory methods.
    pub fn new(
        app: Arc<App>,
        base_name: Arc<Name>,
        notification_rx: tokio::sync::mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Arc<Self> {
        let slim_name = base_name.as_ref().clone().into();
        let inner = CoreServer::new(app.inner().clone(), slim_name, notification_rx);
        
        Arc::new(Self { inner })
    }
}

#[uniffi::export]
impl Server {

    /// Register a unary-to-unary RPC handler
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `handler` - Implementation of the UnaryUnaryHandler trait
    pub fn register_unary_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryUnaryHandler>,
    ) {
        let handler_clone = handler.clone();
        
        self.inner.register_unary_unary(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: slim_rpc::Context| {
                let handler = handler_clone.clone();
                let ctx = Context::from_inner(context);
                
                Box::pin(async move {
                    let result = handler.handle(request, Arc::new(ctx)).await;
                    result.map_err(|e| e.into())
                })
            },
        );
    }

    /// Register a unary-to-stream RPC handler
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `handler` - Implementation of the UnaryStreamHandler trait
    pub fn register_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn UnaryStreamHandler>,
    ) {
        let handler_clone = handler.clone();
        
        self.inner.register_unary_stream(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: slim_rpc::Context| {
                let handler = handler_clone.clone();
                let ctx = Context::from_inner(context);
                
                Box::pin(async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);
                    
                    // Spawn a task to run the handler
                    let _handler_task = {
                        let sink = sink_arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handler.handle(request, Arc::new(ctx), sink.clone()).await {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };
                    
                    // Convert the receiver to a stream
                    let stream = UnboundedReceiverStream::new(rx);
                    Ok(stream)
                })
            },
        );
    }

    /// Register a stream-to-unary RPC handler
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `handler` - Implementation of the StreamUnaryHandler trait
    pub fn register_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamUnaryHandler>,
    ) {
        let handler_clone = handler.clone();
        
        self.inner.register_stream_unary(
            &service_name,
            &method_name,
            move |stream: Box<dyn futures::Stream<Item = Result<Vec<u8>, slim_rpc::Status>> + Send + Unpin>,
                  context: slim_rpc::Context| {
                let handler = handler_clone.clone();
                let ctx = Context::from_inner(context);
                let request_stream = Arc::new(RequestStream::new(stream));
                
                Box::pin(async move {
                    let result = handler.handle(request_stream, Arc::new(ctx)).await;
                    result.map_err(|e| e.into())
                })
            },
        );
    }

    /// Register a stream-to-stream RPC handler
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `handler` - Implementation of the StreamStreamHandler trait
    pub fn register_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn StreamStreamHandler>,
    ) {
        let handler_clone = handler.clone();
        
        self.inner.register_stream_stream(
            &service_name,
            &method_name,
            move |stream: Box<dyn futures::Stream<Item = Result<Vec<u8>, slim_rpc::Status>> + Send + Unpin>,
                  context: slim_rpc::Context| {
                let handler = handler_clone.clone();
                let ctx = Context::from_inner(context);
                let request_stream = Arc::new(RequestStream::new(stream));
                
                Box::pin(async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);
                    
                    // Spawn a task to run the handler
                    let _handler_task = {
                        let sink = sink_arc.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handler.handle(request_stream, Arc::new(ctx), sink.clone()).await {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };
                    
                    // Convert the receiver to a stream
                    let stream = UnboundedReceiverStream::new(rx);
                    Ok(stream)
                })
            },
        );
    }

    /// Get list of registered methods
    ///
    /// Returns a list of registered method names.
    pub fn methods(&self) -> Vec<String> {
        self.inner.methods()
    }

    /// Start serving RPC requests (blocking version)
    ///
    /// This is a blocking method that runs until the server is shut down.
    /// It listens for incoming RPC calls and dispatches them to registered handlers.
    pub fn serve(&self) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.serve_async())
    }

    /// Start serving RPC requests (async version)
    ///
    /// This is an async method that runs until the server is shut down.
    /// It listens for incoming RPC calls and dispatches them to registered handlers.
    pub async fn serve_async(&self) -> Result<(), RpcError> {
        self.inner
            .serve()
            .await
            .map_err(|e| RpcError::new(crate::slimrpc::error::RpcCode::Internal, e.to_string()))
    }

    /// Shutdown the server gracefully (blocking version)
    ///
    /// This signals the server to stop accepting new requests and wait for
    /// in-flight requests to complete.
    pub fn shutdown(&self) {
        crate::get_runtime().block_on(self.shutdown_async())
    }

    /// Shutdown the server gracefully (async version)
    ///
    /// This signals the server to stop accepting new requests and wait for
    /// in-flight requests to complete.
    pub async fn shutdown_async(&self) {
        self.inner
            .shutdown()
            .await
    }
}

impl Server {
    /// Get reference to inner server (for internal use)
    pub(crate) fn inner(&self) -> &CoreServer {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic compilation tests
    #[test]
    fn test_server_type_compiles() {
        // This test ensures the Server type compiles correctly with UniFFI attributes
        // Actual functionality tests require a full SLIM app setup
    }
}
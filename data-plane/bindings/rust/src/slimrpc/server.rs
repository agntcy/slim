// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling implementation
//!
//! Provides a Server type for handling incoming RPC requests and dispatching
//! them to registered service implementations.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::Stream;
use futures::StreamExt;
use parking_lot::RwLock;
use tokio::task::JoinHandle;

use crate::slimrpc::{
    codec::{Decoder, Encoder},
    Code, Context, Status, STATUS_CODE_KEY,
};
use crate::{App, Name, Session};

/// Handler function type for RPC methods
pub type RpcHandler = Arc<
    dyn Fn(Vec<u8>, Context) -> std::pin::Pin<
            Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
        > + Send
        + Sync,
>;

/// Response from an RPC handler
pub enum HandlerResponse {
    /// Single response message
    Unary(Vec<u8>),
    /// Stream of response messages
    Stream(Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send + Unpin>),
}

/// Type of RPC handler
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Registry for RPC service methods
pub struct ServiceRegistry {
    /// Map of method paths to handlers
    handlers: Arc<RwLock<HashMap<String, (RpcHandler, HandlerType)>>>,
}

impl ServiceRegistry {
    /// Create a new service registry
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a unary-unary handler
    pub fn register_unary_unary<F, Req, Res, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Req, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(move |bytes: Vec<u8>, ctx: Context| {
            let handler = Arc::clone(&handler);
            Box::pin(async move {
                let request = Req::decode(&bytes)?;
                let response = handler(request, ctx).await?;
                let response_bytes = response.encode_to_vec()?;
                Ok(HandlerResponse::Unary(response_bytes))
            }) as std::pin::Pin<Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>>
        });

        self.handlers
            .write()
            .insert(method_path, (wrapper, HandlerType::UnaryUnary));
    }

    /// Register a unary-stream handler
    pub fn register_unary_stream<F, Req, Res, S, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Req, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + Unpin + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(move |bytes: Vec<u8>, ctx: Context| {
            let handler = Arc::clone(&handler);
            Box::pin(async move {
                let request = Req::decode(&bytes)?;
                let response_stream = handler(request, ctx).await?;
                let byte_stream = response_stream.map(|res| {
                    res.and_then(|r| r.encode_to_vec())
                });
                Ok(HandlerResponse::Stream(Box::new(byte_stream)))
            }) as std::pin::Pin<Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>>
        });

        self.handlers
            .write()
            .insert(method_path, (wrapper, HandlerType::UnaryStream));
    }

    /// Register a stream-unary handler
    pub fn register_stream_unary<F, Req, Res, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let _handler = Arc::new(handler);
        let wrapper = Arc::new(move |_bytes: Vec<u8>, _ctx: Context| {
            Box::pin(async move {
                // For stream-unary, the first message is handled separately
                // The actual stream will be provided by the server loop
                // This is a placeholder that will be replaced in handle_session
                let response_bytes = Vec::new();
                Ok(HandlerResponse::Unary(response_bytes))
            }) as std::pin::Pin<Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>>
        });

        self.handlers
            .write()
            .insert(method_path, (wrapper, HandlerType::StreamUnary));
    }

    /// Register a stream-stream handler
    pub fn register_stream_stream<F, Req, Res, S, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + Unpin + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let _handler = Arc::new(handler);
        let wrapper = Arc::new(move |_bytes: Vec<u8>, _ctx: Context| {
            Box::pin(async move {
                // For stream-stream, similar to stream-unary
                // The actual implementation is handled in the server loop
                let response_bytes = Vec::new();
                Ok(HandlerResponse::Unary(response_bytes))
            }) as std::pin::Pin<Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>>
        });

        self.handlers
            .write()
            .insert(method_path, (wrapper, HandlerType::StreamStream));
    }

    /// Get a handler by method path
    fn get_handler(&self, method_path: &str) -> Option<(RpcHandler, HandlerType)> {
        self.handlers.read().get(method_path).cloned()
    }

    /// Get all registered method paths
    pub fn methods(&self) -> Vec<String> {
        self.handlers.read().keys().cloned().collect()
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// RPC Server
///
/// Handles incoming RPC requests by creating sessions and dispatching
/// to registered service handlers.
pub struct Server {
    /// The SLIM app instance
    app: Arc<App>,
    /// Service registry containing all registered handlers
    registry: Arc<ServiceRegistry>,
    /// Base service name
    base_name: Arc<Name>,
    /// Running task handles
    tasks: Arc<RwLock<Vec<JoinHandle<()>>>>,
    /// Optional connection ID for subscription propagation
    connection_id: Option<u64>,
}

impl Server {
    /// Create a new RPC server
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    pub fn new(app: Arc<App>, base_name: Name) -> Self {
        Self::new_with_connection(app, base_name, None)
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    pub fn new_with_connection(app: Arc<App>, base_name: Name, connection_id: Option<u64>) -> Self {
        Self {
            app,
            registry: Arc::new(ServiceRegistry::new()),
            base_name: Arc::new(base_name),
            tasks: Arc::new(RwLock::new(Vec::new())),
            connection_id,
        }
    }

    /// Get the service registry for manual registration
    pub fn registry(&self) -> &ServiceRegistry {
        &self.registry
    }

    /// Subscribe to a service method
    async fn subscribe_to_method(&self, service_name: &str, method_name: &str) -> Result<Name, Status> {
        let method_full_name = self.build_method_name(service_name, method_name);

        // Subscribe to this method with optional connection ID for propagation
        tracing::info!("Subscribing to method: {} (connection_id: {:?})", method_full_name, self.connection_id);
        self.app
            .subscribe_async(Arc::new(method_full_name.clone()), self.connection_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to subscribe to method: {}", e)))?;

        Ok(method_full_name)
    }

    /// Start the server and listen for incoming RPC requests
    ///
    /// This method subscribes to all registered methods and starts listening
    /// for incoming sessions.
    pub async fn serve(&self) -> Result<(), Status> {
        // Subscribe to all registered methods
        let methods = self.registry.methods();
        for method_path in &methods {
            let parts: Vec<&str> = method_path.split('/').collect();
            if parts.len() == 2 {
                let service_name = parts[0];
                let method_name = parts[1];
                self.subscribe_to_method(service_name, method_name).await?;
            }
        }

        // Main server loop - listen for sessions
        loop {
            let session = self
                .app
                .listen_for_session_async(None)
                .await
                .map_err(|e| Status::internal(format!("Failed to listen for session: {}", e)))?;

            // Spawn a task to handle this session
            let server = self.clone();
            let handle = tokio::spawn(async move {
                if let Err(e) = server.handle_session(session.as_ref()).await {
                    tracing::error!("Error handling session: {}", e);
                }
            });

            self.tasks.write().push(handle);
        }
    }

    /// Handle an incoming session
    async fn handle_session(&self, session: &Session) -> Result<(), Status> {
        // Extract service and method from destination
        let destination = session.destination().map_err(|e| Status::internal(format!("Failed to get destination: {}", e)))?;
        let (service_name, method_name) = self.parse_method_from_destination(&destination)?;

        // Find the handler
        let method_path = format!("{}/{}", service_name, method_name);
        let (handler, handler_type) = self
            .registry
            .get_handler(&method_path)
            .ok_or_else(|| Status::unimplemented(format!("Method not found: {}", method_path)))?;

        // Create context
        let ctx = Context::from_session(&session)
            .map_err(|e| Status::internal(format!("Failed to create context: {}", e)))?;

        // Check deadline
        if ctx.is_deadline_exceeded() {
            return self.send_error(&session, Status::deadline_exceeded("Deadline exceeded")).await;
        }

        // Handle based on type
        match handler_type {
            HandlerType::UnaryUnary => {
                self.handle_unary_unary(&session, ctx, handler).await?;
            }
            HandlerType::UnaryStream => {
                self.handle_unary_stream(&session, ctx, handler).await?;
            }
            HandlerType::StreamUnary => {
                self.handle_stream_unary(&session, ctx, handler).await?;
            }
            HandlerType::StreamStream => {
                self.handle_stream_stream(&session, ctx, handler).await?;
            }
        }

        Ok(())
    }

    /// Handle unary-unary RPC
    async fn handle_unary_unary(
        &self,
        session: &Session,
        ctx: Context,
        handler: RpcHandler,
    ) -> Result<(), Status> {
        // Receive request
        let received = session
            .get_message_async(None)
            .await
            .map_err(|e| Status::internal(format!("Failed to receive request: {}", e)))?;

        // Call handler
        let response = handler(received.payload, ctx).await?;

        // Send response
        match response {
            HandlerResponse::Unary(response_bytes) => {
                let mut metadata = HashMap::new();
                metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                session
                    .publish_async(response_bytes, Some("msg".to_string()), Some(metadata))
                    .await
                    .map_err(|e| Status::internal(format!("Failed to send response: {}", e)))?;
            }
            _ => {
                return Err(Status::internal("Handler returned unexpected response type"));
            }
        }

        Ok(())
    }

    /// Handle unary-stream RPC
    async fn handle_unary_stream(
        &self,
        session: &Session,
        ctx: Context,
        handler: RpcHandler,
    ) -> Result<(), Status> {
        // Receive request
        let received = session
            .get_message_async(None)
            .await
            .map_err(|e| Status::internal(format!("Failed to receive request: {}", e)))?;

        // Call handler
        let response = handler(received.payload, ctx).await?;

        // Send streaming responses
        match response {
            HandlerResponse::Stream(mut stream) => {
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(response_bytes) => {
                            let mut metadata = HashMap::new();
                            metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                            session
                                .publish_async(response_bytes, Some("msg".to_string()), Some(metadata))
                                .await
                                .map_err(|e| Status::internal(format!("Failed to send response: {}", e)))?;
                        }
                        Err(e) => {
                            return self.send_error(session, e).await;
                        }
                    }
                }

                // Send end-of-stream marker
                let mut metadata = HashMap::new();
                metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                session
                    .publish_async(Vec::new(), Some("msg".to_string()), Some(metadata))
                    .await
                    .map_err(|e| Status::internal(format!("Failed to send end-of-stream: {}", e)))?;
            }
            _ => {
                return Err(Status::internal("Handler returned unexpected response type"));
            }
        }

        Ok(())
    }

    /// Handle stream-unary RPC
    async fn handle_stream_unary(
        &self,
        session: &Session,
        ctx: Context,
        handler: RpcHandler,
    ) -> Result<(), Status> {
        // This is a placeholder - proper implementation would require
        // restructuring to pass the request stream to the handler
        Err(Status::unimplemented("Stream-unary not yet fully implemented"))
    }

    /// Handle stream-stream RPC
    async fn handle_stream_stream(
        &self,
        session: &Session,
        ctx: Context,
        handler: RpcHandler,
    ) -> Result<(), Status> {
        // This is a placeholder - proper implementation would require
        // restructuring to pass the request stream to the handler
        Err(Status::unimplemented("Stream-stream not yet fully implemented"))
    }

    /// Send an error response
    async fn send_error(&self, session: &Session, status: Status) -> Result<(), Status> {
        let message = status.message().unwrap_or("").to_string();
        let mut metadata = HashMap::new();
        metadata.insert(STATUS_CODE_KEY.to_string(), status.code().as_i32().to_string());

        session
            .publish_async(message.into_bytes(), Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| Status::internal(format!("Failed to send error: {}", e)))?;

        Ok(())
    }

    /// Build the full method name from service and method
    fn build_method_name(&self, service_name: &str, method_name: &str) -> Name {
        let components = self.base_name.components();
        Name::new(
            components[0].clone(),
            components[1].clone(),
            format!("{}-{}-{}", components[2], service_name, method_name),
        )
    }

    /// Parse service and method name from destination
    fn parse_method_from_destination(&self, destination: &Name) -> Result<(String, String), Status> {
        let components = destination.components();
        if components.len() < 3 {
            return Err(Status::invalid_argument("Invalid destination name"));
        }

        let full_method = &components[2];
        let base_components = self.base_name.components();
        let base_app = &base_components[2];

        // Remove base prefix: "app-ServiceName-MethodName" -> "ServiceName-MethodName"
        if let Some(suffix) = full_method.strip_prefix(&format!("{}-", base_app)) {
            let parts: Vec<&str> = suffix.splitn(2, '-').collect();
            if parts.len() == 2 {
                return Ok((parts[0].to_string(), parts[1].to_string()));
            }
        }

        Err(Status::invalid_argument(format!(
            "Could not parse service/method from destination: {}",
            full_method
        )))
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            app: Arc::clone(&self.app),
            registry: Arc::clone(&self.registry),
            base_name: Arc::clone(&self.base_name),
            tasks: Arc::clone(&self.tasks),
            connection_id: self.connection_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_registry_new() {
        let registry = ServiceRegistry::new();
        assert_eq!(registry.methods().len(), 0);
    }

    #[test]
    fn test_build_method_name() {
        // Test name building logic without requiring full App setup
        let base_name = Name::new("org".to_string(), "namespace".to_string(), "app".to_string());
        let components = base_name.components();
        let expected = format!("{}-{}-{}", components[2], "MyService", "MyMethod");

        assert_eq!(expected, "app-MyService-MyMethod");
    }

    #[test]
    fn test_parse_method_from_destination() {
        // Test parsing logic
        let full_method = "app-MyService-MyMethod";
        let base_app = "app";

        if let Some(suffix) = full_method.strip_prefix(&format!("{}-", base_app)) {
            let parts: Vec<&str> = suffix.splitn(2, '-').collect();
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0], "MyService");
            assert_eq!(parts[1], "MyMethod");
        } else {
            panic!("Failed to parse method name");
        }
    }

    #[test]
    fn test_handler_type_equality() {
        assert_eq!(HandlerType::UnaryUnary, HandlerType::UnaryUnary);
        assert_ne!(HandlerType::UnaryUnary, HandlerType::UnaryStream);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling implementation
//!
//! Provides a Server type for handling incoming RPC requests and dispatching
//! them to registered service implementations.

use std::collections::HashMap;
use std::sync::Arc;

use async_stream::stream;
use drain;
use futures::StreamExt;
use futures::stream::Stream;
use futures_timer::Delay;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;
use slim_session::context::SessionContext;
use slim_session::errors::SessionError;
use slim_session::notification::Notification;

use crate::{
    Code, Context, STATUS_CODE_KEY, Session, Status,
    codec::{Decoder, Encoder},
};

/// Handler function type for RPC methods (unary input)
pub type RpcHandler = Arc<
    dyn Fn(
            Vec<u8>,
            Context,
        ) -> std::pin::Pin<
            Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
        > + Send
        + Sync,
>;

/// Handler function type for stream-input RPC methods
pub type StreamRpcHandler = Arc<
    dyn Fn(
            std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>,
            Context,
        ) -> std::pin::Pin<
            Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
        > + Send
        + Sync,
>;

/// Response from an RPC handler
pub enum HandlerResponse {
    /// Single response message
    Unary(Vec<u8>),
    /// Stream of response messages
    Stream(std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>),
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
    /// Map of method paths to handlers (for unary-input methods)
    handlers: Arc<RwLock<HashMap<String, (RpcHandler, HandlerType)>>>,
    /// Map of method paths to stream handlers (for stream-input methods)
    stream_handlers: Arc<RwLock<HashMap<String, (StreamRpcHandler, HandlerType)>>>,
}

impl ServiceRegistry {
    /// Create a new service registry
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            stream_handlers: Arc::new(RwLock::new(HashMap::new())),
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
                let response_bytes = response.encode()?;
                Ok(HandlerResponse::Unary(response_bytes))
            })
                as std::pin::Pin<
                    Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
                >
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
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
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
                let byte_stream = response_stream.map(|res| res.and_then(|r| r.encode()));
                Ok(HandlerResponse::Stream(Box::pin(byte_stream)))
            })
                as std::pin::Pin<
                    Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
                >
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
        F: Fn(std::pin::Pin<Box<dyn Stream<Item = Result<Req, Status>> + Send>>, Context) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(
            move |stream: std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>,
                  ctx: Context| {
                let handler = Arc::clone(&handler);
                Box::pin(async move {
                    let request_stream =
                        stream.map(|res| res.and_then(|bytes| Req::decode(&bytes)));
                    let response = handler(Box::pin(request_stream), ctx).await?;
                    let response_bytes = response.encode()?;
                    Ok(HandlerResponse::Unary(response_bytes))
                })
                    as std::pin::Pin<
                        Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
                    >
            },
        );

        self.stream_handlers
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
        F: Fn(std::pin::Pin<Box<dyn Stream<Item = Result<Req, Status>> + Send>>, Context) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(
            move |stream: std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>,
                  ctx: Context| {
                let handler = Arc::clone(&handler);
                Box::pin(async move {
                    let request_stream =
                        stream.map(|res| res.and_then(|bytes| Req::decode(&bytes)));
                    let response_stream = handler(Box::pin(request_stream), ctx).await?;
                    let byte_stream = response_stream.map(|res| res.and_then(|r| r.encode()));
                    Ok(HandlerResponse::Stream(Box::pin(byte_stream)))
                })
                    as std::pin::Pin<
                        Box<dyn futures::Future<Output = Result<HandlerResponse, Status>> + Send>,
                    >
            },
        );

        self.stream_handlers
            .write()
            .insert(method_path, (wrapper, HandlerType::StreamStream));
    }

    /// Get a handler by method path
    fn get_handler(&self, method_path: &str) -> Option<(RpcHandler, HandlerType)> {
        self.handlers.read().get(method_path).cloned()
    }

    fn get_stream_handler(&self, method_path: &str) -> Option<(StreamRpcHandler, HandlerType)> {
        self.stream_handlers.read().get(method_path).cloned()
    }

    /// Get all registered method paths
    pub fn methods(&self) -> Vec<String> {
        let mut methods: Vec<String> = self.handlers.read().keys().cloned().collect();
        methods.extend(self.stream_handlers.read().keys().cloned());
        methods
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal server state shared across clones
struct ServerInner {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Service registry containing all registered handlers
    registry: ServiceRegistry,
    /// Base service name
    base_name: Name,
    /// Running task handles
    tasks: RwLock<Vec<JoinHandle<()>>>,
    /// Optional connection ID for subscription propagation
    #[allow(dead_code)]
    connection_id: Option<u64>,
    /// Notification receiver for incoming sessions (either owned or shared)
    notification_rx: tokio::sync::Mutex<Option<NotificationReceiver>>,
    /// Cancellation token for shutdown
    cancellation_token: CancellationToken,
    /// Drain signal for graceful shutdown
    drain_signal: RwLock<Option<drain::Signal>>,
    /// Drain watch for session handlers
    drain_watch: RwLock<Option<drain::Watch>>,
}

/// Enum to hold either an owned or shared notification receiver
enum NotificationReceiver {
    Owned(mpsc::Receiver<Result<Notification, SessionError>>),
    Shared(Arc<tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>>),
}

/// RPC Server
///
/// Handles incoming RPC requests by creating sessions and dispatching
/// to registered service handlers.
pub struct Server {
    inner: Arc<ServerInner>,
}

impl Server {
    /// Create a new RPC server
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `notification_rx` - Receiver for session notifications
    pub fn new(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        Self::new_with_connection(app, base_name, None, notification_rx)
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    /// * `notification_rx` - Receiver for session notifications
    pub fn new_with_connection(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        Self {
            inner: Arc::new(ServerInner {
                app,
                registry: ServiceRegistry::new(),
                base_name,
                tasks: RwLock::new(Vec::new()),
                connection_id,
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Owned(notification_rx))),
                cancellation_token: CancellationToken::new(),
                drain_signal: RwLock::new(Some(drain_signal)),
                drain_watch: RwLock::new(Some(drain_watch)),
            }),
        }
    }

    /// Create a new RPC server with shared notification receiver
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `notification_rx` - Shared Arc to the notification receiver
    pub fn new_with_shared_rx(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        notification_rx: Arc<tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>>,
    ) -> Self {
        Self::new_with_shared_rx_and_connection(app, base_name, None, notification_rx)
    }

    /// Create a new RPC server with shared notification receiver and connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    /// * `notification_rx` - Shared Arc to the notification receiver
    pub fn new_with_shared_rx_and_connection(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: Arc<tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>>,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        Self {
            inner: Arc::new(ServerInner {
                app,
                registry: ServiceRegistry::new(),
                base_name,
                tasks: RwLock::new(Vec::new()),
                connection_id,
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Shared(notification_rx))),
                cancellation_token: CancellationToken::new(),
                drain_signal: RwLock::new(Some(drain_signal)),
                drain_watch: RwLock::new(Some(drain_watch)),
            }),
        }
    }

    /// Get the service registry for manual registration
    pub fn registry(&self) -> &ServiceRegistry {
        &self.inner.registry
    }

    /// Subscribe to a service method

    /// Start the server and listen for incoming RPC requests
    ///
    /// This method listens for incoming sessions. The service/method routing
    /// is determined by metadata in the session, not by subscriptions.
    pub async fn serve(&self) -> Result<(), Status> {
        tracing::info!(
            "SlimRPC server starting on base_name: {}",
            self.inner.base_name
        );

        // Main server loop - listen for sessions
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = self.inner.cancellation_token.cancelled() => {
                    tracing::info!("Server received shutdown signal");
                    return Ok(());
                }
                // Handle incoming sessions
                session_result = self.listen_for_session(None) => {
                    let session_ctx = session_result?;

                    // Spawn a task to handle this session
                    let server = self.clone();
                    let handle = tokio::spawn(async move {
                        let session = session_ctx.session.clone();

                        if let Err(e) = server.handle_session(session_ctx).await {
                            tracing::error!("Error handling session: {}", e);
                        }

                        // Delete the session
                        if let Some(session) = session.upgrade() {
                            if let Ok(handle) = server.inner.app.delete_session(session.as_ref()) {
                                let _ = handle.await;
                            }
                        }
                    });

                    self.inner.tasks.write().push(handle);
                }
            }
        }
    }

    /// Shutdown the server gracefully
    ///
    /// This signals all active session handlers to terminate and waits for them to drain.
    /// After shutdown completes, the server can be restarted by calling `serve()` again.
    pub async fn shutdown(&self) {
        tracing::info!("Shutting down SlimRPC server");

        // Signal cancellation to the main serve loop
        self.inner.cancellation_token.cancel();

        // Take the drain signal and watch
        let drain_signal = self.inner.drain_signal.write().take();
        let drain_watch = self.inner.drain_watch.write().take();

        // Drop the watch to complete the drain
        drop(drain_watch);

        // Signal all session handlers to terminate
        if let Some(signal) = drain_signal {
            tracing::debug!("Draining active sessions");
            signal.drain().await;
            tracing::info!("All sessions drained successfully");
        }

        // Recreate drain signal and watch so the server can be restarted
        let (new_signal, new_watch) = drain::channel();
        *self.inner.drain_signal.write() = Some(new_signal);
        *self.inner.drain_watch.write() = Some(new_watch);

        tracing::debug!("Server shutdown complete, ready to restart");
    }

    /// Listen for an incoming session
    async fn listen_for_session(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<SessionContext, Status> {
        let mut rx_guard = self.inner.notification_rx.lock().await;
        let rx_opt = rx_guard.take();
        
        let notification_opt = match rx_opt {
            Some(NotificationReceiver::Owned(mut rx)) => {
                let result = if let Some(dur) = timeout {
                    // Runtime-agnostic timeout using tokio::time
                    tokio::select! {
                        result = rx.recv() => result,
                        _ = tokio::time::sleep(dur) => {
                            *rx_guard = Some(NotificationReceiver::Owned(rx));
                            return Err(Status::deadline_exceeded("listen_for_session timed out"));
                        }
                    }
                } else {
                    rx.recv().await
                };
                
                // Put the receiver back
                *rx_guard = Some(NotificationReceiver::Owned(rx));
                result
            }
            Some(NotificationReceiver::Shared(rx_arc)) => {
                // For shared receiver, put it back immediately and work with the Arc
                *rx_guard = Some(NotificationReceiver::Shared(rx_arc.clone()));
                drop(rx_guard);
                
                // Now lock and receive from the shared receiver
                let result = if let Some(dur) = timeout {
                    // Runtime-agnostic timeout using tokio::time
                    let mut rx = rx_arc.write().await;
                    tokio::select! {
                        result = rx.recv() => result,
                        _ = tokio::time::sleep(dur) => {
                            return Err(Status::deadline_exceeded("listen_for_session timed out"));
                        }
                    }
                } else {
                    let mut rx = rx_arc.write().await;
                    rx.recv().await
                };
                
                result
            }
            None => {
                return Err(Status::internal("notification receiver not available"));
            }
        };

        if notification_opt.is_none() {
            return Err(Status::internal("notification channel closed"));
        }

        let notification = notification_opt
            .unwrap()
            .map_err(|e| Status::internal(format!("Session error: {}", e)))?;

        match notification {
            Notification::NewSession(session_ctx) => Ok(session_ctx),
            _ => Err(Status::internal("Unexpected notification type")),
        }
    }

    /// Handle an incoming session
    async fn handle_session(&self, session_ctx: SessionContext) -> Result<(), Status> {
        // Get a drain watch handle for this session
        let drain_watch = self
            .inner
            .drain_watch
            .read()
            .clone()
            .ok_or_else(|| Status::internal("drain watch not available"))?;

        // Create context from the session context
        let ctx = Context::from_session(&session_ctx);

        // Wrap the session context
        let session = Session::new(session_ctx);

        // Extract service and method from metadata
        let metadata = session.metadata().await;
        let service_name = metadata
            .get("slimrpc-service")
            .ok_or_else(|| Status::invalid_argument("Missing service name in metadata"))?
            .clone();
        let method_name = metadata
            .get("slimrpc-method")
            .ok_or_else(|| Status::invalid_argument("Missing method name in metadata"))?
            .clone();

        // Get the handler based on type
        let method_path = format!("{}/{}", service_name, method_name);

        // Try to get as a stream handler first (for stream-unary and stream-stream)
        if let Some((stream_handler, handler_type)) =
            self.inner.registry.get_stream_handler(&method_path)
        {
            return self
                .handle_stream_based_method(stream_handler, handler_type, &session, ctx)
                .await;
        }

        // Otherwise get as a regular handler (for unary-unary and unary-stream)
        let (handler, handler_type) = self
            .inner
            .registry
            .get_handler(&method_path)
            .ok_or_else(|| Status::unimplemented(format!("Method not found: {}", method_path)))?;

        // Check deadline
        if ctx.is_deadline_exceeded() {
            return self
                .send_error(&session, Status::deadline_exceeded("Deadline exceeded"))
                .await;
        }

        // Handle based on type (only unary-input handlers reach here)
        tokio::select! {
            result = async {
                match handler_type {
                    HandlerType::UnaryUnary => {
                        self.handle_unary_unary(handler, &session, ctx).await
                    }
                    HandlerType::UnaryStream => {
                        self.handle_unary_stream(handler, &session, ctx).await
                    }
                    _ => {
                        Err(Status::internal("Invalid handler type for unary-input method"))
                    }
                }
            } => result,
            _ = drain_watch.clone().signaled() => {
                tracing::debug!("Session handler terminated due to server shutdown");
                Err(Status::unavailable("Server is shutting down"))
            }
        }
    }

    /// Handle unary-unary RPC
    async fn handle_unary_unary(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
    ) -> Result<(), Status> {
        // Receive request
        let received = session
            .get_message(None)
            .await
            .map_err(|e| Status::internal(format!("Failed to receive request: {}", e)))?;

        // Call handler
        let response = match handler(received.payload, ctx).await {
            Ok(resp) => resp,
            Err(status) => {
                tracing::debug!("Handler returned error: {:?}", status);
                return self.send_error(session, status).await;
            }
        };

        // Send response
        match response {
            HandlerResponse::Unary(response_bytes) => {
                let mut metadata = HashMap::new();
                metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                let handle = session
                    .publish(response_bytes, Some("msg".to_string()), Some(metadata))
                    .await
                    .map_err(|e| Status::internal(format!("Failed to send response: {}", e)))?;
                handle.await
                    .map_err(|e| Status::internal(format!("Failed to receive response ack: {}", e)))?;
            }
            _ => {
                return Err(Status::internal(
                    "Handler returned unexpected response type",
                ));
            }
        }

        Ok(())
    }

    /// Handle unary-stream RPC
    async fn handle_unary_stream(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
    ) -> Result<(), Status> {
        // Receive request
        let received = session
            .get_message(None)
            .await
            .map_err(|e| Status::internal(format!("Failed to receive request: {}", e)))?;

        // Clone metadata before moving ctx into handler
        let end_metadata = ctx.metadata().as_map().clone();

        // Call handler
        let response = match handler(received.payload, ctx).await {
            Ok(resp) => resp,
            Err(status) => {
                tracing::debug!("Handler returned error: {:?}", status);
                return self.send_error(session, status).await;
            }
        };

        // Send streaming responses
        match response {
            HandlerResponse::Stream(mut stream) => {
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(response_bytes) => {
                            let mut metadata = HashMap::new();
                            metadata
                                .insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                            session
                                .publish(response_bytes, Some("msg".to_string()), Some(metadata))
                                .await
                                .map_err(|e| {
                                    Status::internal(format!("Failed to send response: {}", e))
                                })?;
                        }
                        Err(e) => {
                            return self.send_error(session, e).await;
                        }
                    }
                }

                // Send end-of-stream marker
                let mut end_metadata = end_metadata.clone();
                end_metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                session
                    .publish(Vec::new(), Some("msg".to_string()), Some(end_metadata))
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to send end-of-stream: {}", e))
                    })?;
            }
            _ => {
                return Err(Status::internal(
                    "Handler returned unexpected response type",
                ));
            }
        }

        Ok(())
    }

    /// Handle stream-based methods (stream-unary and stream-stream)
    async fn handle_stream_based_method(
        &self,
        handler: StreamRpcHandler,
        handler_type: HandlerType,
        session: &Session,
        ctx: Context,
    ) -> Result<(), Status> {
        // Get a drain watch handle for this session
        let drain_watch = self
            .inner
            .drain_watch
            .read()
            .clone()
            .ok_or_else(|| Status::internal("drain watch not available"))?;

        // Create a stream of incoming requests
        let session_clone = session.clone();
        let drain_for_stream = drain_watch.clone();

        let request_stream = stream! {
            loop {
                let received_result = tokio::select! {
                    result = session_clone.get_message(None) => result,
                    _ = drain_for_stream.clone().signaled() => {
                        tracing::debug!("Request stream terminated due to server shutdown");
                        break;
                    }
                };

                println!("Received message in stream-based method");

                let received = match received_result {
                    Ok(msg) => msg,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };

                // Check for end-of-stream marker
                let code = received.metadata.get(STATUS_CODE_KEY)
                    .and_then(|s| s.parse::<i32>().ok())
                    .and_then(Code::from_i32)
                    .unwrap_or(Code::Ok);

                if code == Code::Ok && received.payload.is_empty() {
                    break;
                }

                if code != Code::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    yield Err(Status::new(code, message));
                    break;
                }

                yield Ok(received.payload);
            }
        };

        // Pin the stream
        let boxed_stream: std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>> =
            Box::pin(request_stream);

        // Call the handler with drain signal awareness
        let handler_result = tokio::select! {
            result = handler(boxed_stream, ctx) => match result {
                Ok(resp) => resp,
                Err(status) => {
                    tracing::debug!("Handler returned error: {:?}", status);
                    return self.send_error(session, status).await;
                }
            },
            _ = drain_watch.clone().signaled() => {
                tracing::debug!("Stream handler terminated due to server shutdown");
                return Err(Status::unavailable("Server is shutting down"));
            }
        };

        match handler_type {
            HandlerType::StreamUnary => {
                // Send single response
                let response = match handler_result {
                    HandlerResponse::Unary(bytes) => bytes,
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                let mut metadata = HashMap::new();
                metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                session
                    .publish(response, Some("msg".to_string()), Some(metadata))
                    .await
                    .map_err(|e| Status::internal(format!("Failed to send response: {}", e)))?;
            }
            HandlerType::StreamStream => {
                // Send streaming responses
                let mut response_stream = match handler_result {
                    HandlerResponse::Stream(stream) => stream,
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                let end_metadata = HashMap::new();

                while let Some(result) = response_stream.next().await {
                    match result {
                        Ok(payload) => {
                            let mut metadata = end_metadata.clone();
                            metadata
                                .insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                            session
                                .publish(payload, Some("msg".to_string()), Some(metadata))
                                .await
                                .map_err(|e| {
                                    Status::internal(format!("Failed to send response: {}", e))
                                })?;
                        }
                        Err(status) => {
                            self.send_error(session, status).await?;
                            return Ok(());
                        }
                    }
                }

                // Send end-of-stream marker
                let mut end_metadata = end_metadata.clone();
                end_metadata.insert(STATUS_CODE_KEY.to_string(), Code::Ok.as_i32().to_string());
                session
                    .publish(Vec::new(), Some("msg".to_string()), Some(end_metadata))
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to send end-of-stream: {}", e))
                    })?;
            }
            _ => {
                return Err(Status::internal(
                    "Invalid handler type for stream-based method",
                ));
            }
        }

        Ok(())
    }

    /// Send an error response
    async fn send_error(&self, session: &Session, status: Status) -> Result<(), Status> {
        let message = status.message().unwrap_or("").to_string();
        let mut metadata = HashMap::new();
        metadata.insert(
            STATUS_CODE_KEY.to_string(),
            status.code().as_i32().to_string(),
        );

        session
            .publish(
                message.into_bytes(),
                Some("msg".to_string()),
                Some(metadata),
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to send error: {}", e)))?;

        Ok(())
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
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
        let base_name = Name::from_strings(["org", "namespace", "app"]);
        let components = base_name.components_strings();
        let expected = format!("{}-{}-{}", &components[2], "MyService", "MyMethod");

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

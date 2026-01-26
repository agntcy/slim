// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling implementation
//!
//! Provides a Server type for handling incoming RPC requests and dispatching
//! them to registered service implementations.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use async_stream::stream;
use drain;
use futures::StreamExt;
use futures::stream::Stream;

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

/// A wrapper that makes a pinned stream Unpin
struct UnpinStream<S> {
    inner: Pin<Box<S>>,
}

impl<S> UnpinStream<S> {
    fn new(stream: S) -> Self {
        Self {
            inner: Box::pin(stream),
        }
    }
}

impl<S: Stream> Stream for UnpinStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

// UnpinStream is always Unpin regardless of whether S is
impl<S> Unpin for UnpinStream<S> {}

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
            Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send + Unpin>,
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

/// Registry for RPC service methods (internal implementation detail)
struct ServiceRegistry {
    /// Map of method paths to handlers (for unary-input methods)
    handlers: Arc<RwLock<HashMap<String, (RpcHandler, HandlerType)>>>,
    /// Map of method paths to stream handlers (for stream-input methods)
    stream_handlers: Arc<RwLock<HashMap<String, (StreamRpcHandler, HandlerType)>>>,
}

impl ServiceRegistry {
    /// Create a new service registry
    fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            stream_handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a unary-unary handler
    fn register_unary_unary<F, Req, Res, Fut>(
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
                let request = Req::decode(bytes)?;
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
    fn register_unary_stream<F, Req, Res, S, Fut>(
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
                let request = Req::decode(bytes)?;
                let response_stream = handler(request, ctx).await?;
                let byte_stream = response_stream.map(|res| res.and_then(|r| r.encode()));
                Ok(HandlerResponse::Stream(Box::new(UnpinStream::new(
                    byte_stream,
                ))))
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
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut
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
            move |stream: Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send + Unpin>,
                  ctx: Context| {
                let handler = Arc::clone(&handler);
                Box::pin(async move {
                    let request_stream = stream.map(|res| res.and_then(|bytes| Req::decode(bytes)));
                    let response = handler(Box::new(request_stream), ctx).await?;
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
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut
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
            move |stream: Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send + Unpin>,
                  ctx: Context| {
                let handler = Arc::clone(&handler);
                Box::pin(async move {
                    let request_stream = stream.map(|res| res.and_then(|bytes| Req::decode(bytes)));
                    let response_stream = handler(Box::new(request_stream), ctx).await?;
                    let byte_stream = response_stream.map(|res| res.and_then(|r| r.encode()));
                    Ok(HandlerResponse::Stream(Box::new(UnpinStream::new(
                        byte_stream,
                    ))))
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
    fn methods(&self) -> Vec<String> {
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
///
/// # Example
///
/// ```no_run
/// # use slim_rpc::{Server, Context, Status};
/// # use slim_datapath::messages::Name;
/// # use slim_service::app::App;
/// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
/// # use std::sync::Arc;
/// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>, notification_rx: tokio::sync::mpsc::Receiver<std::result::Result<slim_session::notification::Notification, slim_session::errors::SessionError>>) -> std::result::Result<(), Status> {
/// # #[derive(Default)]
/// # struct Request {}
/// # impl slim_rpc::Decoder for Request {
/// #     fn decode(_buf: Vec<u8>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
/// # }
/// # #[derive(Default)]
/// # struct Response {}
/// # impl slim_rpc::Encoder for Response {
/// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
/// # }
/// let base_name = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
/// let server = Server::new(app, base_name, notification_rx);
///
/// // Register handlers
/// server.register_unary_unary(
///     "MyService",
///     "MyMethod",
///     |request: Request, _ctx: Context| async move {
///         Ok(Response::default())
///     }
/// );
///
/// // Start serving
/// server.serve().await?;
/// # Ok(())
/// # }
/// ```
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
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Server;
    /// # use slim_datapath::messages::Name;
    /// # use slim_service::app::App;
    /// # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    /// # use std::sync::Arc;
    /// # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>, notification_rx: tokio::sync::mpsc::Receiver<std::result::Result<slim_session::notification::Notification, slim_session::errors::SessionError>>) {
    /// let base_name = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
    /// let server = Server::new(app, base_name, notification_rx);
    /// # }
    /// ```
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
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Owned(
                    notification_rx,
                ))),
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
        notification_rx: Arc<
            tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>,
        >,
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
        notification_rx: Arc<
            tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>,
        >,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        Self {
            inner: Arc::new(ServerInner {
                app,
                registry: ServiceRegistry::new(),
                base_name,
                tasks: RwLock::new(Vec::new()),
                connection_id,
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Shared(
                    notification_rx,
                ))),
                cancellation_token: CancellationToken::new(),
                drain_signal: RwLock::new(Some(drain_signal)),
                drain_watch: RwLock::new(Some(drain_watch)),
            }),
        }
    }

    /// Register a unary-unary RPC handler
    ///
    /// Handles a single request and returns a single response.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `handler` - An async function that takes a request and context, returns a response
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Context, Status};
    /// # #[derive(Default)]
    /// # struct Request { name: String }
    /// # impl slim_rpc::Decoder for Request {
    /// #     fn decode(_buf: Vec<u8>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { greeting: String }
    /// # impl slim_rpc::Encoder for Response {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # fn example(server: Server) {
    /// server.register_unary_unary(
    ///     "GreeterService",
    ///     "SayHello",
    ///     |request: Request, _ctx: Context| async move {
    ///         Ok(Response {
    ///             greeting: format!("Hello, {}", request.name)
    ///         })
    ///     }
    /// );
    /// # }
    /// ```
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
        self.inner
            .registry
            .register_unary_unary(service_name, method_name, handler)
    }

    /// Register a unary-stream RPC handler
    ///
    /// Handles a single request and returns a stream of responses.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `handler` - An async function that takes a request and returns a stream of responses
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Context, Status};
    /// # use futures::stream;
    /// # #[derive(Default)]
    /// # struct Request { count: i32 }
    /// # impl slim_rpc::Decoder for Request {
    /// #     fn decode(_buf: Vec<u8>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { value: i32 }
    /// # impl slim_rpc::Encoder for Response {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # fn example(server: Server) {
    /// server.register_unary_stream(
    ///     "NumberService",
    ///     "GenerateNumbers",
    ///     |request: Request, _ctx: Context| async move {
    ///         let numbers: Vec<std::result::Result<Response, Status>> = (0..request.count)
    ///             .map(|i| Ok(Response { value: i }))
    ///             .collect();
    ///         Ok(stream::iter(numbers))
    ///     }
    /// );
    /// # }
    /// ```
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
        self.inner
            .registry
            .register_unary_stream(service_name, method_name, handler)
    }

    /// Register a stream-unary RPC handler
    ///
    /// Handles a stream of requests and returns a single response.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `handler` - An async function that takes a request stream and returns a response
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Context, Status, RequestStream};
    /// # use futures::StreamExt;
    /// # #[derive(Default)]
    /// # struct Request { value: i32 }
    /// # impl slim_rpc::Decoder for Request {
    /// #     fn decode(_buf: Vec<u8>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { sum: i32 }
    /// # impl slim_rpc::Encoder for Response {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # fn example(server: Server) {
    /// server.register_stream_unary(
    ///     "AggregateService",
    ///     "SumNumbers",
    ///     |mut request_stream: RequestStream<Request>, _ctx: Context| async move {
    ///         let mut sum = 0;
    ///         while let Some(result) = request_stream.next().await {
    ///             let request = result?;
    ///             sum += request.value;
    ///         }
    ///         Ok(Response { sum })
    ///     }
    /// );
    /// # }
    /// ```
    pub fn register_stream_unary<F, Req, Res, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.inner
            .registry
            .register_stream_unary(service_name, method_name, handler)
    }

    /// Register a stream-stream RPC handler
    ///
    /// Handles a stream of requests and returns a stream of responses.
    ///
    /// # Arguments
    /// * `service_name` - The name of the service
    /// * `method_name` - The name of the method
    /// * `handler` - An async function that takes a request stream and returns a response stream
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Context, Status, RequestStream};
    /// # use futures::{stream, StreamExt};
    /// # #[derive(Default)]
    /// # struct Request { message: String }
    /// # impl slim_rpc::Decoder for Request {
    /// #     fn decode(_buf: Vec<u8>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { reply: String }
    /// # impl slim_rpc::Encoder for Response {
    /// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # fn example(server: Server) {
    /// server.register_stream_stream(
    ///     "EchoService",
    ///     "Echo",
    ///     |mut request_stream: RequestStream<Request>, _ctx: Context| async move {
    ///         let responses = async_stream::stream! {
    ///             while let Some(result) = request_stream.next().await {
    ///                 match result {
    ///                     Ok(request) => {
    ///                         yield Ok(Response {
    ///                             reply: format!("Echo: {}", request.message)
    ///                         });
    ///                     }
    ///                     Err(e) => {
    ///                         yield Err(e);
    ///                         break;
    ///                     }
    ///                 }
    ///             }
    ///         };
    ///         Ok(responses)
    ///     }
    /// );
    /// # }
    /// ```
    pub fn register_stream_stream<F, Req, Res, S, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(Box<dyn Stream<Item = Result<Req, Status>> + Send + Unpin>, Context) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.inner
            .registry
            .register_stream_stream(service_name, method_name, handler)
    }

    /// Get all registered method paths
    ///
    /// Returns a list of all registered service/method paths in the format "Service/Method".
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Server;
    /// # fn example(server: Server) {
    /// let methods = server.methods();
    /// for method in methods {
    ///     println!("Registered: {}", method);
    /// }
    /// # }
    /// ```
    pub fn methods(&self) -> Vec<String> {
        self.inner.registry.methods()
    }

    /// Start the server and listen for incoming RPC requests
    ///
    /// This method listens for incoming sessions and dispatches them to registered handlers.
    /// The service/method routing is determined by metadata in the session.
    ///
    /// This method runs indefinitely until [`shutdown`](Self::shutdown) is called or an error occurs.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Status};
    /// # async fn example(server: Server) -> std::result::Result<(), Status> {
    /// // Register handlers first...
    ///
    /// // Start serving - this runs until shutdown
    /// server.serve().await?;
    /// # Ok(())
    /// # }
    /// ```
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

                    // Spawn a task to handle this session (processes multiple RPCs in a loop)
                    let server = self.clone();
                    let handle = tokio::spawn(async move {
                        let session_weak = session_ctx.session.clone();
                        let session = Session::new(session_ctx);

                        if let Err(e) = server.handle_session_with_wrapper(session).await {
                            tracing::error!("Error handling session: {}", e);
                        }

                        // Delete the session when done
                        if let Some(session) = session_weak.upgrade() {
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
    /// After shutdown completes, the server can be restarted by calling [`serve`](Self::serve) again.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::Server;
    /// # async fn example(server: Server) {
    /// // In another task or signal handler:
    /// server.shutdown().await;
    /// # }
    /// ```
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

    /// Handle an incoming session wrapper (processes multiple RPCs in a loop)
    async fn handle_session_with_wrapper(&self, session: Session) -> Result<(), Status> {
        // Get a drain watch handle for this session
        let drain_watch = self
            .inner
            .drain_watch
            .read()
            .clone()
            .ok_or_else(|| Status::internal("drain watch not available"))?;

        // Create initial context from session wrapper
        let initial_ctx = Context::from_session_wrapper(&session).await;

        // Loop to handle multiple RPCs on the same session
        loop {
            // Wait for the first message of the next RPC
            let first_message = tokio::select! {
                result = session.get_message(None) => {
                    match result {
                        Ok(msg) => msg,
                        Err(e) => {
                            tracing::debug!("Session closed or error receiving message: {}", e);
                            break;
                        }
                    }
                }
                _ = drain_watch.clone().signaled() => {
                    tracing::debug!("Session handler terminated due to server shutdown");
                    return Err(Status::unavailable("Server is shutting down"));
                }
            };

            // Extract service and method from message metadata
            let service_name = match first_message.metadata.get("slimrpc-service") {
                Some(name) => name.clone(),
                None => {
                    tracing::debug!("Missing service name in message metadata");
                    continue;
                }
            };
            let method_name = match first_message.metadata.get("slimrpc-method") {
                Some(name) => name.clone(),
                None => {
                    tracing::debug!("Missing method name in message metadata");
                    continue;
                }
            };

            tracing::debug!("Processing RPC: {}/{}", service_name, method_name);

            // Create context for this RPC by cloning initial context and adding message metadata
            let ctx = initial_ctx.clone().with_message_metadata(first_message.metadata.clone());

            // Get the handler based on type
            let method_path = format!("{}/{}", service_name, method_name);

            // Try to get as a stream handler first (for stream-unary and stream-stream)
            let result = if let Some((stream_handler, handler_type)) =
                self.inner.registry.get_stream_handler(&method_path)
            {
                self
                    .handle_stream_based_method(stream_handler, handler_type, &session, ctx, first_message, &drain_watch)
                    .await
            } else if let Some((handler, handler_type)) = self.inner.registry.get_handler(&method_path) {
                // Check deadline
                if ctx.is_deadline_exceeded() {
                    self
                        .send_error(&session, Status::deadline_exceeded("Deadline exceeded"))
                        .await
                } else {
                    // Handle based on type (only unary-input handlers reach here)
                    tokio::select! {
                        result = async {
                            match handler_type {
                                HandlerType::UnaryUnary => {
                                    self.handle_unary_unary(handler, &session, ctx, first_message).await
                                }
                                HandlerType::UnaryStream => {
                                    self.handle_unary_stream(handler, &session, ctx, first_message).await
                                }
                                _ => {
                                    Err(Status::internal("Invalid handler type for unary-input method"))
                                }
                            }
                        } => result,
                        _ = drain_watch.clone().signaled() => {
                            tracing::debug!("Session handler terminated due to server shutdown");
                            return Err(Status::unavailable("Server is shutting down"));
                        }
                    }
                }
            } else {
                self.send_error(&session, Status::unimplemented(format!("Method not found: {}", method_path))).await
            };

            if let Err(e) = result {
                tracing::error!("Error handling RPC {}: {}", method_path, e);
                // On error, break the loop and close the session
                break;
            }

            // Successfully handled one RPC, continue to handle next RPC on same session
            tracing::debug!("Completed RPC {}, ready for next RPC on same session", method_path);
        }

        Ok(())
    }

    /// Handle unary-unary RPC
    async fn handle_unary_unary(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
        first_message: crate::ReceivedMessage,
    ) -> Result<(), Status> {
        // Use the first message that was already received
        let received = first_message;

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
                handle.await.map_err(|e| {
                    Status::internal(format!("Failed to receive response ack: {}", e))
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

    /// Handle unary-stream RPC
    async fn handle_unary_stream(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
        first_message: crate::ReceivedMessage,
    ) -> Result<(), Status> {
        // Use the first message that was already received
        let received = first_message;

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
            HandlerResponse::Stream(stream) => {
                let mut stream = stream;
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
        first_message: crate::ReceivedMessage,
        drain_watch: &drain::Watch,
    ) -> Result<(), Status> {
        // Create a stream of incoming requests
        let session_clone = session.clone();
        let drain_for_stream = drain_watch.clone();

        let request_stream = stream! {
            // Check if the first message is an end-of-stream marker
            let first_code = first_message.metadata.get(STATUS_CODE_KEY)
                .and_then(|s| s.parse::<i32>().ok())
                .and_then(Code::from_i32)
                .unwrap_or(Code::Ok);
            
            // If first message is end-of-stream, don't yield it and exit immediately
            if first_code == Code::Ok && first_message.payload.is_empty() {
                // Empty stream - exit immediately
                return;
            }
            
            // Yield the first message
            yield Ok(first_message.payload);

            loop {
                let received_result = tokio::select! {
                    result = session_clone.get_message(None) => result,
                    _ = drain_for_stream.clone().signaled() => {
                        tracing::debug!("Request stream terminated due to server shutdown");
                        break;
                    }
                };

                tracing::debug!("Received message in stream-based method");

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

        // Wrap the stream to make it Unpin
        let boxed_stream: Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send + Unpin> =
            Box::new(UnpinStream::new(request_stream));

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
                let response_stream = match handler_result {
                    HandlerResponse::Stream(stream) => stream,
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                let end_metadata = HashMap::new();

                let mut response_stream = response_stream;
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

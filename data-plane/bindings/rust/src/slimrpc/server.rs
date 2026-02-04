// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling implementation
//!
//! Provides a Server type for handling incoming RPC requests and dispatching
//! them to registered service implementations.

use std::collections::HashMap;
use std::sync::Arc;

use async_stream::stream;
use futures::stream::Stream;
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;

use slim_session::errors::SessionError;
use slim_session::notification::Notification;

use super::{
    Code, Context, MAX_TIMEOUT, STATUS_CODE_KEY, Session, Status,
    codec::{Decoder, Encoder},
};

pub type Item = Vec<u8>;
pub type ItemStream = BoxStream<'static, Result<Vec<u8>, Status>>;
pub type ResponseStream = BoxFuture<'static, Result<HandlerResponse, Status>>;

/// Handler function type for RPC methods (unary input)
pub type RpcHandler = Arc<dyn Fn(Item, Context) -> ResponseStream + Send + Sync>;

/// Handler function type for stream-input RPC methods
pub type StreamRpcHandler = Arc<dyn Fn(ItemStream, Context) -> ResponseStream + Send + Sync>;

/// Response from an RPC handler
pub enum HandlerResponse {
    /// Single response message
    Unary(Item),
    /// Stream of response messages
    Stream(ItemStream),
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
    handlers: HashMap<String, (RpcHandler, HandlerType)>,
    /// Map of method paths to stream handlers (for stream-input methods)
    stream_handlers: HashMap<String, (StreamRpcHandler, HandlerType)>,
    /// Map of subscription names to method paths for routing
    subscription_to_method: HashMap<Name, String>,
}

impl ServiceRegistry {
    /// Create a new service registry
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            stream_handlers: HashMap::new(),
            subscription_to_method: HashMap::new(),
        }
    }

    /// Register a subscription name mapping
    fn register_subscription(&mut self, mut subscription_name: Name, method_path: String) {
        subscription_name.set_id(Name::NULL_COMPONENT);
        self.subscription_to_method
            .insert(subscription_name, method_path);
    }

    /// Get method path from subscription name
    fn get_method_from_subscription(&self, subscription_name: &mut Name) -> Option<String> {
        subscription_name.set_id(Name::NULL_COMPONENT);
        self.subscription_to_method.get(subscription_name).cloned()
    }

    /// Register a unary-unary handler
    fn register_unary_unary<F, Req, Res, Fut>(
        &mut self,
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
            async move {
                let request = Req::decode(bytes)?;
                let response = handler(request, ctx).await?;
                let response_bytes = response.encode()?;
                Ok(HandlerResponse::Unary(response_bytes))
            }
            .boxed()
        });

        self.handlers
            .insert(method_path, (wrapper, HandlerType::UnaryUnary));
    }

    /// Register a unary-stream handler
    pub fn register_unary_stream<F, Req, Res, S, Fut>(
        &mut self,
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
            async move {
                let request = Req::decode(bytes)?;
                let response_stream = handler(request, ctx).await?;
                let byte_mapped = response_stream
                    .map(|res| res.and_then(|r| r.encode()))
                    .boxed();
                Ok(HandlerResponse::Stream(byte_mapped))
            }
            .boxed()
        });

        self.handlers
            .insert(method_path, (wrapper, HandlerType::UnaryStream));
    }

    /// Register a stream-unary handler
    pub fn register_stream_unary<F, Req, Res, Fut>(
        &mut self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(super::RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(move |stream: ItemStream, ctx: Context| {
            let handler = Arc::clone(&handler);
            async move {
                let mapped = stream.map(|res| res.and_then(|bytes| Req::decode(bytes)));
                let boxed_stream = mapped.boxed();
                let response = handler(boxed_stream, ctx).await?;
                let response_bytes = response.encode()?;
                Ok(HandlerResponse::Unary(response_bytes))
            }
            .boxed()
        });

        self.stream_handlers
            .insert(method_path, (wrapper, HandlerType::StreamUnary));
    }

    /// Register a stream-stream handler
    pub fn register_stream_stream<F, Req, Res, S, Fut>(
        &mut self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(super::RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        let method_path = format!("{}/{}", service_name, method_name);
        let handler = Arc::new(handler);
        let wrapper = Arc::new(move |stream: ItemStream, ctx: Context| {
            let handler = Arc::clone(&handler);
            async move {
                let mapped = stream.map(|res| res.and_then(|bytes| Req::decode(bytes)));
                let boxed_stream = mapped.boxed();
                let response_stream = handler(boxed_stream, ctx).await?;
                let byte_mapped = response_stream.map(|res| res.and_then(|r| r.encode()));
                Ok(HandlerResponse::Stream(byte_mapped.boxed()))
            }
            .boxed()
        });

        self.stream_handlers
            .insert(method_path, (wrapper, HandlerType::StreamStream));
    }

    /// Get handler info (either stream or unary) in one lookup
    fn get_handler_info(&self, method_path: &str) -> Option<HandlerInfo> {
        if let Some((stream_handler, handler_type)) = self.stream_handlers.get(method_path).cloned()
        {
            Some(HandlerInfo::Stream(stream_handler, handler_type))
        } else if let Some((handler, handler_type)) = self.handlers.get(method_path).cloned() {
            Some(HandlerInfo::Unary(handler, handler_type))
        } else {
            None
        }
    }

    /// Get all registered method paths
    fn methods(&self) -> Vec<String> {
        let mut methods: Vec<String> = self.handlers.keys().cloned().collect();
        methods.extend(self.stream_handlers.keys().cloned());
        methods
    }
}

/// Handler information retrieved from registry
enum HandlerInfo {
    Stream(StreamRpcHandler, HandlerType),
    Unary(RpcHandler, HandlerType),
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
    registry: RwLock<ServiceRegistry>,
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
    /// Runtime handle for spawning tasks (resolved at construction)
    runtime: tokio::runtime::Handle,
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
/// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
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
/// // Start serving in background task
/// let server_handle = server.serve();
///
/// // Wait for server to complete
/// server_handle.await.unwrap()?;
/// # Ok(())
/// # }
/// ```
#[derive(uniffi::Object)]
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
    pub fn create_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        Self::create_with_connection_and_runtime(app, base_name, None, notification_rx, None)
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    /// * `notification_rx` - Receiver for session notifications
    /// * `runtime` - Optional tokio runtime handle for spawning tasks
    pub fn create_with_connection_and_runtime(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        // Resolve runtime handle: use provided or try to get current
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            inner: Arc::new(ServerInner {
                app,
                registry: RwLock::new(ServiceRegistry::new()),
                base_name,
                tasks: RwLock::new(Vec::new()),
                connection_id,
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Owned(
                    notification_rx,
                ))),
                cancellation_token: CancellationToken::new(),
                drain_signal: RwLock::new(Some(drain_signal)),
                drain_watch: RwLock::new(Some(drain_watch)),
                runtime,
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
        Self::new_with_shared_rx_and_connection(app, base_name, None, notification_rx, None)
    }

    /// Create a new RPC server with shared notification receiver and connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    /// * `notification_rx` - Shared Arc to the notification receiver
    /// * `runtime` - Optional tokio runtime handle for spawning tasks
    pub fn new_with_shared_rx_and_connection(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: Arc<
            tokio::sync::RwLock<mpsc::Receiver<Result<Notification, SessionError>>>,
        >,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        // Resolve runtime handle: use provided or try to get current
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            inner: Arc::new(ServerInner {
                app,
                registry: RwLock::new(ServiceRegistry::new()),
                base_name,
                tasks: RwLock::new(Vec::new()),
                connection_id,
                notification_rx: tokio::sync::Mutex::new(Some(NotificationReceiver::Shared(
                    notification_rx,
                ))),
                cancellation_token: CancellationToken::new(),
                drain_signal: RwLock::new(Some(drain_signal)),
                drain_watch: RwLock::new(Some(drain_watch)),
                runtime,
            }),
        }
    }

    /// Helper method to register subscription for a method
    fn register_method_mapping(&self, service_name: &str, method_name: &str) {
        // Build subscription name and register mapping
        let subscription_name =
            super::build_method_subscription_name(&self.inner.base_name, service_name, method_name);
        let method_path = format!("{}/{}", service_name, method_name);
        self.inner
            .registry
            .write()
            .register_subscription(subscription_name, method_path);
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
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
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
    pub fn register_unary_unary_internal<F, Req, Res, Fut>(
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
            .write()
            .register_unary_unary(service_name, method_name, handler);

        self.register_method_mapping(service_name, method_name);
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
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
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
    pub fn register_unary_stream_internal<F, Req, Res, S, Fut>(
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
            .write()
            .register_unary_stream(service_name, method_name, handler);

        self.register_method_mapping(service_name, method_name);
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
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
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
    pub fn register_stream_unary_internal<F, Req, Res, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(super::RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.inner
            .registry
            .write()
            .register_stream_unary(service_name, method_name, handler);

        self.register_method_mapping(service_name, method_name);
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
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
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
    pub fn register_stream_stream_internal<F, Req, Res, S, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(super::RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.inner
            .registry
            .write()
            .register_stream_stream(service_name, method_name, handler);

        self.register_method_mapping(service_name, method_name);
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
        self.inner.registry.read().methods()
    }

    /// Start the server and listen for incoming RPC requests in a separate task
    ///
    /// This method spawns a background task that listens for incoming sessions and
    /// dispatches them to registered handlers. The service/method routing is determined
    /// by metadata in the session.
    ///
    /// The spawned task runs indefinitely until [`shutdown`](Self::shutdown) is called
    /// or an error occurs.
    ///
    /// # Returns
    ///
    /// Returns a `JoinHandle` for the spawned server task. The task result is `Result<(), Status>`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_rpc::{Server, Status};
    /// # async fn example(server: Server) -> std::result::Result<(), Status> {
    /// // Register handlers first...
    ///
    /// // Start serving in background task
    /// let server_handle = server.serve();
    ///
    /// // Do other work...
    ///
    /// // Wait for server to complete (or handle shutdown)
    /// server_handle.await.unwrap()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn serve_handle(&self) -> JoinHandle<Result<(), Status>> {
        let server = self.clone();
        self.inner
            .runtime
            .spawn(async move { server.serve_internal().await })
    }

    /// Internal server loop implementation
    ///
    /// This method contains the actual server loop logic and is called by [`serve`](Self::serve)
    /// in a spawned task.
    async fn serve_internal(&self) -> Result<(), Status> {
        tracing::info!(
            base_name = %self.inner.base_name,
            "SlimRPC server starting"
        );

        // Subscribe to all registered methods
        let subscription_names: Vec<Name> = {
            let registry = self.inner.registry.read();
            registry.subscription_to_method.keys().cloned().collect()
        };

        for subscription_name in subscription_names {
            tracing::info!(%subscription_name, "Subscribing");
            self.inner
                .app
                .subscribe(&subscription_name, self.inner.connection_id)
                .await
                .map_err(|e| {
                    Status::internal(format!(
                        "Failed to subscribe to {}: {}",
                        subscription_name, e
                    ))
                })?;
        }

        // Main server loop - listen for sessions
        loop {
            tokio::select! {
                // Handle shutdown signal
                _ = self.inner.cancellation_token.cancelled() => {
                    tracing::info!("Server received shutdown signal");
                    return Ok(());
                }
                // Handle incoming sessions
                session_result = self.listen_for_session() => {
                    tracing::debug!("Received session notification");

                    let session_ctx = session_result?;

                    // Get the source (subscription name) from the session controller to determine which method to call
                    let mut subscription_name = session_ctx.session_arc()
                        .ok_or_else(|| Status::internal("Session controller not available"))?
                        .source()
                        .clone();

                    tracing::debug!(%subscription_name, "Processing session for subscription");

                    // Log the registry for debugging
                    {
                        let registry = self.inner.registry.read();
                        let methods = registry.methods();
                        tracing::debug!(
                            methods = ?methods,
                            "Registered methods in service registry"
                        );
                    }

                    // Look up the method path and handler info for this subscription
                    let lookup_result = {
                        let registry = self.inner.registry.read();
                        let method_path_opt = registry.get_method_from_subscription(&mut subscription_name);
                        method_path_opt.and_then(|method_path| {
                            registry.get_handler_info(&method_path).map(|handler_info| (method_path, handler_info))
                        })
                    };

                    // Spawn a task to handle this session
                    let server = self.clone();
                    let session = Session::new(session_ctx);
                    let handle = self.inner.runtime.spawn(async move {
                        // Check if method is registered and handle error in task
                        let Some((method_path, handler_info)) = lookup_result else {
                            tracing::error!(%subscription_name, "No method registered for subscription");
                            // Send error and wait for acknowledgment
                            let _ = server.send_error(&session, Status::internal("No method registered for subscription")).await;

                            // Delete the session when done
                            let _ = session.close(server.inner.app.as_ref()).await;
                            return;
                        };

                        tracing::debug!(%method_path, %subscription_name, "Received session for method");

                        if let Err(e) = server.handle_session(&session, &method_path, handler_info).await {
                            tracing::error!(%method_path, error = %e, "Error handling session");

                            // Send error to client before closing
                            let _ = server.send_error(&session, e).await;
                        }

                        // Close the session after handling (success or after sending error)
                        let _ = session.close(server.inner.app.as_ref()).await;
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
    pub async fn shutdown_internal(&self) {
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

    /// Create status code metadata
    fn create_status_metadata(code: Code) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        metadata.insert(STATUS_CODE_KEY.to_string(), code.as_i32().to_string());
        metadata
    }

    /// Send a message with status code
    async fn send_message(session: &Session, payload: Vec<u8>, code: Code) -> Result<(), Status> {
        let metadata = Self::create_status_metadata(code);
        let handle = session
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| Status::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| Status::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker
    async fn send_end_of_stream(session: &Session) -> Result<(), Status> {
        Self::send_message(session, Vec::new(), Code::Ok).await
    }

    /// Send all responses from a stream
    async fn send_response_stream(session: &Session, mut stream: ItemStream) -> Result<(), Status> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    Self::send_message(session, response_bytes, Code::Ok).await?;
                }
                Err(e) => return Err(e),
            }
        }
        Self::send_end_of_stream(session).await
    }

    /// Get timeout duration from context
    fn get_timeout_duration(ctx: &Context) -> std::time::Duration {
        ctx.remaining_time()
            .unwrap_or(std::time::Duration::from_secs(MAX_TIMEOUT))
    }

    /// Receive first message from session
    async fn receive_first_message(session: &Session) -> Result<super::ReceivedMessage, Status> {
        session.get_message(None).await.map_err(|e| {
            tracing::debug!(error = %e, "Session closed or error receiving message");
            Status::internal(format!("Failed to receive message: {}", e))
        })
    }

    /// Listen for an incoming session from the notification receiver
    async fn listen_for_session(&self) -> Result<slim_session::context::SessionContext, Status> {
        tracing::debug!("Waiting for incoming session notification");
        let mut rx_guard = self.inner.notification_rx.lock().await;
        tracing::debug!("Notification receiver lock acquired");
        let rx_opt = rx_guard.take();

        let notification_opt = match rx_opt {
            Some(NotificationReceiver::Owned(mut rx)) => {
                let result = rx.recv().await;
                // Put the receiver back
                *rx_guard = Some(NotificationReceiver::Owned(rx));
                result
            }
            Some(NotificationReceiver::Shared(rx_arc)) => {
                // For shared receiver, put it back immediately and work with the Arc
                *rx_guard = Some(NotificationReceiver::Shared(rx_arc.clone()));
                drop(rx_guard);

                tracing::debug!("Acquiring shared notification receiver");

                // Now lock and receive from the shared receiver
                let mut rx = rx_arc.write().await;

                tracing::debug!("Receiving from shared notification receiver");

                rx.recv().await
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

    /// Handle a session for a specific method, processing multiple RPCs until session closes
    async fn handle_session(
        &self,
        session: &Session,
        method_path: &str,
        handler_info: HandlerInfo,
    ) -> Result<(), Status> {
        // Get a drain watch handle for this session
        let drain_watch = self
            .inner
            .drain_watch
            .read()
            .clone()
            .ok_or_else(|| Status::internal("drain watch not available"))?;

        // Create context from session wrapper
        let ctx = Context::from_session_wrapper(session).await;

        tracing::debug!(%method_path, "Processing RPC");

        // Handle based on handler type (already looked up before calling this function)
        let result = match handler_info {
            HandlerInfo::Stream(stream_handler, handler_type) => {
                // Check deadline for stream-based methods
                if ctx.is_deadline_exceeded() {
                    Err(Status::deadline_exceeded("Deadline exceeded"))
                } else {
                    self.handle_stream_based_method(
                        stream_handler,
                        handler_type,
                        session,
                        ctx,
                        &drain_watch,
                    )
                    .await
                }
            }
            HandlerInfo::Unary(handler, handler_type) => {
                // Check deadline
                if ctx.is_deadline_exceeded() {
                    Err(Status::deadline_exceeded("Deadline exceeded"))
                } else {
                    // Handle based on type (only unary-input handlers reach here)
                    tokio::select! {
                        result = async {
                            match handler_type {
                                HandlerType::UnaryUnary => {
                                    self.handle_unary_unary(handler, session, ctx).await
                                }
                                HandlerType::UnaryStream => {
                                    self.handle_unary_stream(handler, session, ctx).await
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
            }
        };

        if let Err(e) = &result {
            tracing::error!(%method_path, error = %e, "Error handling RPC");
        } else {
            tracing::debug!(%method_path, "RPC completed successfully");
        }

        result
    }

    /// Handle unary-unary RPC
    async fn handle_unary_unary(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
    ) -> Result<(), Status> {
        // Get the first message from the session
        let received = Self::receive_first_message(session).await?;

        // Update context with message metadata to parse deadline
        let ctx = ctx.with_message_metadata(received.metadata);

        // Calculate deadline
        let timeout_duration = Self::get_timeout_duration(&ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;

        // Call handler and send response with deadline
        tokio::select! {
            result = async {
                // Call handler
                let response = handler(received.payload, ctx).await?;

                // Send response
                match response {
                    HandlerResponse::Unary(response_bytes) => {
                        Self::send_message(session, response_bytes, Code::Ok).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
        }
    }

    /// Handle unary-stream RPC
    async fn handle_unary_stream(
        &self,
        handler: RpcHandler,
        session: &Session,
        ctx: Context,
    ) -> Result<(), Status> {
        // Get the first message from the session
        let received = Self::receive_first_message(session).await?;

        // Calculate deadline and create sleep future
        let timeout_duration = Self::get_timeout_duration(&ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let sleep_fut = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep_fut);

        // Call handler and send streaming responses with deadline
        tokio::select! {
            result = async {
                // Call handler
                let response = handler(received.payload, ctx).await?;

                // Send streaming responses
                match response {
                    HandlerResponse::Stream(stream) => {
                        Self::send_response_stream(session, stream).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = &mut sleep_fut => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
        }
    }

    /// Handle stream-based methods (stream-unary and stream-stream)
    async fn handle_stream_based_method(
        &self,
        handler: StreamRpcHandler,
        handler_type: HandlerType,
        session: &Session,
        ctx: Context,
        drain_watch: &drain::Watch,
    ) -> Result<(), Status> {
        // Calculate deadline and create sleep future
        let timeout_duration = Self::get_timeout_duration(&ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let sleep_fut = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep_fut);

        // Create a oneshot channel to signal stream completion
        let (stream_done_tx, stream_done_rx) = tokio::sync::oneshot::channel::<()>();

        // Create a stream of incoming requests
        let session_clone = session.clone();
        let drain_for_stream = drain_watch.clone();

        // Wrap sender in Option so we can move it into the stream
        let mut stream_done_tx = Some(stream_done_tx);

        let request_stream = stream! {
            loop {
                // Get the next message from the session
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

            // Signal that the stream has completed
            if let Some(tx) = stream_done_tx.take() {
                let _ = tx.send(());
            }
        };

        // Box and pin the stream
        let boxed_stream = request_stream.boxed();

        // Call handler and send responses with deadline and drain signal awareness
        tokio::select! {
            result = async {
                // Call handler
                let handler_result = handler(boxed_stream, ctx).await?;

                // Send responses based on handler type
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

                        Self::send_message(session, response, Code::Ok).await?;
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

                        Self::send_response_stream(session, response_stream).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Invalid handler type for stream-based method",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = &mut sleep_fut => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
            _ = drain_watch.clone().signaled() => {
                tracing::debug!("Stream handler terminated due to server shutdown");
                Err(Status::unavailable("Server is shutting down"))
            }
        }?;

        // Wait for the request stream to complete before returning
        // This ensures all background tasks are finished before resource cleanup
        let _ = stream_done_rx.await;

        Ok(())
    }

    /// Send an error response
    async fn send_error(&self, session: &Session, status: Status) -> Result<(), Status> {
        let message = status.message().unwrap_or("").to_string();
        Self::send_message(session, message.into_bytes(), status.code())
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "Failed to send error response");
                e
            })
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

// UniFFI-compatible methods for foreign language bindings
#[uniffi::export]
impl Server {
    /// Create a new RPC server
    ///
    /// This is the primary constructor for creating an RPC server instance
    /// that can handle incoming RPC requests over SLIM.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance that provides the underlying
    ///   network transport and session management
    /// * `base_name` - The base name for this service (e.g., org.namespace.service).
    ///   This name is used to construct subscription names for RPC methods.
    ///
    /// # Returns
    /// A new RPC server instance wrapped in an Arc for shared ownership
    #[uniffi::constructor]
    pub fn new(app: &std::sync::Arc<crate::App>, base_name: std::sync::Arc<crate::Name>) -> std::sync::Arc<Self> {
        Self::new_with_connection(app, base_name, None)
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// The connection ID is used to set up routing before serving RPC requests,
    /// enabling multi-hop RPC calls through specific connections.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance that provides the underlying
    ///   network transport and session management
    /// * `base_name` - The base name for this service (e.g., org.namespace.service).
    ///   This name is used to construct subscription names for RPC methods.
    /// * `connection_id` - Optional connection ID for routing setup
    ///
    /// # Returns
    /// A new RPC server instance wrapped in an Arc for shared ownership
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: &std::sync::Arc<crate::App>,
        base_name: std::sync::Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> std::sync::Arc<Self> {
        let app_inner = app.inner_app().clone();
        let slim_name = base_name.as_slim_name().clone();
        let rx = app.notification_receiver();
        
        // Get the notification receiver from the app
        let notification_rx = crate::get_runtime().block_on(async {
            let mut lock = rx.write().await;
            // Take ownership of the receiver by replacing it with a dummy channel
            let (_, new_rx) = tokio::sync::mpsc::channel(1);
            std::mem::replace(&mut *lock, new_rx)
        });
        
        std::sync::Arc::new(Self::create_with_connection_and_runtime(
            app_inner,
            slim_name,
            connection_id,
            notification_rx,
            Some(crate::get_runtime().handle().clone()),
        ))
    }

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
        handler: std::sync::Arc<dyn super::UnaryUnaryHandler>,
    ) {
        let handler_clone = handler.clone();
        let service_clone = service_name.clone();
        let method_clone = method_name.clone();

        tracing::debug!(service = %service_clone, method = %method_clone, "Registering unary-unary handler");
        
        self.register_unary_unary_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: super::Context| {
                let handler = handler_clone.clone();
                let ctx = super::UniffiContext::new(context);

                tracing::debug!(service = %service_clone, method = %method_clone, "Handling unary-unary request");

                Box::pin(async move {
                    let result = handler.handle(request, std::sync::Arc::new(ctx)).await;
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
        handler: std::sync::Arc<dyn super::UnaryStreamHandler>,
    ) {
        let handler_clone = handler.clone();

        self.register_unary_stream_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: super::Context| {
                let handler = handler_clone.clone();
                let ctx = super::UniffiContext::new(context);

                Box::pin(async move {
                    let (sink, rx) = super::ResponseSink::receiver();
                    let sink_arc = std::sync::Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        crate::get_runtime().spawn(async move {
                            if let Err(e) = handler.handle(request, std::sync::Arc::new(ctx), sink.clone()).await {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };

                    // Detach the task - it will run independently
                    drop(handler_task);

                    // Convert the receiver to a stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
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
        handler: std::sync::Arc<dyn super::StreamUnaryHandler>,
    ) {
        let handler_clone = handler.clone();

        self.register_stream_unary_internal(
            &service_name,
            &method_name,
            move |stream: super::RequestStream<Vec<u8>>, context: super::Context| {
                let handler = handler_clone.clone();
                let ctx = super::UniffiContext::new(context);
                let request_stream = std::sync::Arc::new(super::UniffiRequestStream::new(stream));

                Box::pin(async move {
                    let result = handler.handle(request_stream, std::sync::Arc::new(ctx)).await;
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
        handler: std::sync::Arc<dyn super::StreamStreamHandler>,
    ) {
        let handler_clone = handler.clone();

        self.register_stream_stream_internal(
            &service_name,
            &method_name,
            move |stream: super::RequestStream<Vec<u8>>, context: super::Context| {
                let handler = handler_clone.clone();
                let ctx = super::UniffiContext::new(context);
                let request_stream = std::sync::Arc::new(super::UniffiRequestStream::new(stream));

                Box::pin(async move {
                    let (sink, rx) = super::ResponseSink::receiver();
                    let sink_arc = std::sync::Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        crate::get_runtime().spawn(async move {
                            if let Err(e) = handler.handle(request_stream, std::sync::Arc::new(ctx), sink.clone()).await {
                                let _ = sink.send_error_async(e).await;
                            }
                        })
                    };

                    // Detach the task - it will run independently
                    drop(handler_task);

                    // Convert the receiver to a stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
                    Ok(stream)
                })
            },
        );
    }

    /// Start serving RPC requests (blocking version)
    ///
    /// This is a blocking method that runs until the server is shut down.
    /// It listens for incoming RPC calls and dispatches them to registered handlers.
    pub fn serve(&self) -> Result<(), super::RpcError> {
        let handle = self.serve_handle();
        crate::get_runtime().block_on(async move {
            match handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(status)) => Err(status.into()),
                Err(e) => Err(super::RpcError::new(super::Code::Internal, e.to_string())),
            }
        })
    }

    /// Start serving RPC requests (async version)
    ///
    /// This is an async method that runs until the server is shut down.
    /// It listens for incoming RPC calls and dispatches them to registered handlers.
    pub async fn serve_async(&self) -> Result<(), super::RpcError> {
        let handle = self.serve_handle();
        match handle.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(status)) => Err(status.into()),
            Err(e) => Err(super::RpcError::new(super::Code::Internal, e.to_string())),
        }
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
        self.shutdown_internal().await
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

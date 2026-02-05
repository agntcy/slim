// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Server-side RPC handling implementation
//!
//! Provides a Server type for handling incoming RPC requests and dispatching
//! them to registered service implementations.

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::join_all;
use futures::stream::Stream;
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_datapath::messages::Name;
use slim_service::app::App as SlimApp;
use slim_session::errors::SessionError;
use slim_session::notification::Notification;

use super::{
    Code, Context, HandlerInfo, RequestStream, ResponseSink, RpcError, RpcSession, Status,
    StreamRpcSession, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler,
    UnaryUnaryHandler, UniffiRequestStream, build_method_subscription_name,
    codec::{Decoder, Encoder},
    send_error,
    session_wrapper::new_session,
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
#[derive(Clone)]
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
        F: Fn(RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
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
        F: Fn(RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
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

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
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
/// # use slim_bindings::{Server, Context, Status, Decoder, Encoder, App, Name};
/// # use std::sync::Arc;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # use slim_bindings::{IdentityProviderConfig, IdentityVerifierConfig};
/// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
/// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
/// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
/// # let app = App::new(app_name, provider, verifier)?;
/// # let core_app = app.inner();
/// # let notification_rx = app.notification_receiver();
/// # #[derive(Default)]
/// # struct Request {}
/// # impl Decoder for Request {
/// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Request::default()) }
/// # }
/// # #[derive(Default)]
/// # struct Response {}
/// # impl Encoder for Response {
/// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
/// # }
/// let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
/// let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
///
/// // Register handlers
/// server.register_unary_unary_internal(
///     "MyService",
///     "MyMethod",
///     |request: Request, _ctx: Context| async move {
///         Ok(Response::default())
///     }
/// );
/// # Ok(())
/// # }
/// ```
#[derive(uniffi::Object)]
pub struct Server {
    /// The SLIM app instance
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
    /// Service registry containing all registered handlers
    registry: RwLock<ServiceRegistry>,
    /// Base service name
    base_name: Name,
    /// Optional connection ID for subscription propagation
    connection_id: Option<u64>,
    /// Notification receiver for incoming sessions (either owned or shared)
    notification_rx: parking_lot::Mutex<Option<NotificationReceiver>>,
    /// Drain signal for graceful shutdown
    drain_signal: RwLock<Option<drain::Signal>>,
    /// Drain watch for session handlers
    drain_watch: RwLock<Option<drain::Watch>>,
    /// Runtime handle for spawning tasks (resolved at construction)
    runtime: tokio::runtime::Handle,
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
    /// # use slim_bindings::{Server, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        Self::construct_internal(
            app,
            base_name,
            None,
            NotificationReceiver::Owned(notification_rx),
            None,
        )
    }

    /// Create a new RPC server with optional connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server (e.g., "org/namespace/app")
    /// * `connection_id` - Optional connection ID for subscription propagation to next SLIM node
    /// * `notification_rx` - Receiver for session notifications
    /// * `runtime` - Optional tokio runtime handle for spawning tasks
    pub fn new_with_connection_and_runtime(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        Self::construct_internal(
            app,
            base_name,
            connection_id,
            NotificationReceiver::Owned(notification_rx),
            runtime,
        )
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
        Self::construct_internal(
            app,
            base_name,
            connection_id,
            NotificationReceiver::Shared(notification_rx),
            runtime,
        )
    }

    /// Internal constructor - single point of truth for Server construction
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server
    /// * `connection_id` - Optional connection ID for subscription propagation
    /// * `notification_rx` - Notification receiver (owned or shared)
    /// * `runtime` - Optional tokio runtime handle for spawning tasks
    fn construct_internal(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        base_name: Name,
        connection_id: Option<u64>,
        notification_rx: NotificationReceiver,
        runtime: Option<tokio::runtime::Handle>,
    ) -> Self {
        let (drain_signal, drain_watch) = drain::channel();

        // Resolve runtime handle: use provided or try to get current
        let runtime = runtime.unwrap_or_else(|| {
            tokio::runtime::Handle::try_current()
                .expect("No tokio runtime found. Either provide a runtime handle or call from within a tokio runtime context")
        });

        Self {
            app,
            registry: RwLock::new(ServiceRegistry::new()),
            base_name,
            connection_id,
            notification_rx: parking_lot::Mutex::new(Some(notification_rx)),
            drain_signal: RwLock::new(Some(drain_signal)),
            drain_watch: RwLock::new(Some(drain_watch)),
            runtime,
        }
    }

    /// Helper method to register subscription for a method
    fn register_method_mapping(&self, service_name: &str, method_name: &str) {
        // Build subscription name and register mapping
        let subscription_name =
            build_method_subscription_name(&self.base_name, service_name, method_name);
        let method_path = format!("{}/{}", service_name, method_name);
        self.registry
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
    /// # use slim_bindings::{Server, Context, Status, Decoder, Encoder, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// # #[derive(Default)]
    /// # struct Request { name: String }
    /// # impl Decoder for Request {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { greeting: String }
    /// # impl Encoder for Response {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// server.register_unary_unary_internal(
    ///     "GreeterService",
    ///     "SayHello",
    ///     |request: Request, _ctx: Context| async move {
    ///         Ok(Response {
    ///             greeting: format!("Hello, {}", request.name)
    ///         })
    ///     }
    /// );
    /// # Ok(())
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
        self.registry
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
    /// # use slim_bindings::{Server, Context, Status, Decoder, Encoder, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # use futures::stream;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// # #[derive(Default)]
    /// # struct Request { count: i32 }
    /// # impl Decoder for Request {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { value: i32 }
    /// # impl Encoder for Response {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// server.register_unary_stream_internal(
    ///     "NumberService",
    ///     "GenerateNumbers",
    ///     |request: Request, _ctx: Context| async move {
    ///         let numbers: Vec<Result<Response, Status>> = (0..request.count)
    ///             .map(|i| Ok(Response { value: i }))
    ///             .collect();
    ///         Ok(stream::iter(numbers))
    ///     }
    /// );
    /// # Ok(())
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
        self.registry
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
    /// # use slim_bindings::{Server, Context, Status, Decoder, Encoder, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # use futures::StreamExt;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// # #[derive(Default)]
    /// # struct Request { value: i32 }
    /// # impl Decoder for Request {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { sum: i32 }
    /// # impl Encoder for Response {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # use futures::stream::BoxStream;
    /// server.register_stream_unary_internal(
    ///     "AggregateService",
    ///     "SumNumbers",
    ///     |mut request_stream: BoxStream<'static, Result<Request, Status>>, _ctx: Context| async move {
    ///         let mut sum = 0;
    ///         while let Some(result) = request_stream.next().await {
    ///             let request = result?;
    ///             sum += request.value;
    ///         }
    ///         Ok(Response { sum })
    ///     }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_stream_unary_internal<F, Req, Res, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.registry
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
    /// # use slim_bindings::{Server, Context, Status, Decoder, Encoder, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # use futures::StreamExt;
    /// # use async_stream::stream;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// # #[derive(Default)]
    /// # struct Request { message: String }
    /// # impl Decoder for Request {
    /// #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, Status> { Ok(Request::default()) }
    /// # }
    /// # #[derive(Default)]
    /// # struct Response { reply: String }
    /// # impl Encoder for Response {
    /// #     fn encode(self) -> Result<Vec<u8>, Status> { Ok(vec![]) }
    /// # }
    /// # use futures::stream::BoxStream;
    /// server.register_stream_stream_internal(
    ///     "EchoService",
    ///     "Echo",
    ///     |mut request_stream: BoxStream<'static, Result<Request, Status>>, _ctx: Context| async move {
    ///         let responses = stream! {
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
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_stream_stream_internal<F, Req, Res, S, Fut>(
        &self,
        service_name: &str,
        method_name: &str,
        handler: F,
    ) where
        F: Fn(RequestStream<Req>, Context) -> Fut + Send + Sync + 'static,
        Fut: futures::Future<Output = Result<S, Status>> + Send + 'static,
        S: Stream<Item = Result<Res, Status>> + Send + 'static,
        Req: Decoder + Send + 'static,
        Res: Encoder + Send + 'static,
    {
        self.registry
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
    /// # use slim_bindings::{Server, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// let methods = server.methods();
    /// for method in methods {
    ///     println!("Registered: {}", method);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn methods(&self) -> Vec<String> {
        self.registry.read().methods()
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
    /// # use slim_bindings::{Server, Status, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// // Register handlers first...
    ///
    /// // Start serving in background task
    /// let _server_handle = server.serve_handle()?;
    ///
    /// // Do other work...
    /// # Ok(())
    /// # }
    /// ```
    fn serve_handle(
        &self,
    ) -> Result<JoinHandle<(NotificationReceiver, Result<(), Status>)>, RpcError> {
        let notification = self
            .notification_rx
            .lock()
            .take()
            .ok_or(Status::internal("server already running"))?;

        let registry = self.registry.read().clone();
        let base_name = self.base_name.clone();
        let app = self.app.clone();
        let connection_id = self.connection_id;
        let drain_watch = self
            .drain_watch
            .read()
            .clone()
            .ok_or_else(|| Status::internal("drain watch not available"))?;

        let ret = self.runtime.spawn(Server::serve_internal(
            notification,
            registry,
            connection_id,
            base_name,
            app,
            drain_watch,
        ));

        Ok(ret)
    }

    /// Internal server loop implementation
    ///
    /// This method contains the actual server loop logic and is called by [`serve`](Self::serve)
    /// in a spawned task.
    async fn serve_internal(
        mut rx: NotificationReceiver,
        registry: ServiceRegistry,
        connection_id: Option<u64>,
        base_name: Name,
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        drain_watch: drain::Watch,
    ) -> (NotificationReceiver, Result<(), Status>) {
        tracing::info!(
            %base_name,
            "SlimRPC server starting"
        );

        // Subscribe to all registered methods
        let subscription_names: Vec<Name> =
            registry.subscription_to_method.keys().cloned().collect();

        for subscription_name in subscription_names {
            tracing::info!(%subscription_name, "Subscribing");
            if let Err(e) = app.subscribe(&subscription_name, connection_id).await {
                let status = Status::internal(format!(
                    "Failed to subscribe to {}: {}",
                    subscription_name, e
                ));
                return (rx, Err(status));
            }
        }

        // Save spawned tasks
        let mut tasks = vec![];

        // Pin the drain watch for use in select (clone to avoid moving)
        let mut drain_signaled = std::pin::pin!(drain_watch.clone().signaled());

        // Main server loop - listen for sessions
        loop {
            tokio::select! {
                // Handle shutdown signal via drain
                _ = &mut drain_signaled => {
                    tracing::info!("Server received drain signal, waiting for {} tasks to complete", tasks.len());
                    break;
                }
                // Handle incoming sessions
                session_result = Server::listen_for_session(&mut rx) => {
                    tracing::debug!("Received session notification");

                    let session_ctx = match session_result {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            tracing::error!("Error receiving session: {}", e);
                            return (rx, Err(e));
                        }
                    };

                    // Get the source (subscription name) from the session controller to determine which method to call
                    let mut subscription_name = match session_ctx.session_arc() {
                        Some(session) => session.source().clone(),
                        None => {
                            let status = Status::internal("Session controller not available");
                            return (rx, Err(status));
                        }
                    };

                    tracing::debug!(%subscription_name, "Processing session for subscription");

                    // Look up the method path and handler info for this subscription
                    let lookup_result = {
                        let method_path_opt = registry.get_method_from_subscription(&mut subscription_name);
                        method_path_opt.and_then(|method_path| {
                            registry.get_handler_info(&method_path).map(|handler_info| (method_path, handler_info))
                        })
                    };

                    // Spawn a task to handle this session
                    let (session_tx, mut session_rx) = new_session(session_ctx);
                    let app_clone = app.clone();
                    let watch_clone = drain_watch.clone();
                    let handle = tokio::spawn(async move {
                        let watch = std::pin::pin!(watch_clone.signaled());

                        tokio::select! {
                            _ = watch => {
                                tracing::debug!(%subscription_name, "Session task terminated due to server shutdown");
                                // Send Cancelled error to client before closing
                                let _ = send_error(&session_tx, Status::cancelled("Server shutting down")).await;
                                let _ = session_tx.close(app_clone.as_ref()).await;
                            }
                            _ = async {
                                // Check if method is registered and handle error in task
                                let Some((method_path, handler_info)) = lookup_result else {
                                    tracing::error!(%subscription_name, "No method registered for subscription");
                                    // Send error and wait for acknowledgment
                                    let _ = send_error(&session_tx, Status::internal("No method registered for subscription")).await;

                                    // Delete the session when done
                                    let _ = session_tx.close(app_clone.as_ref()).await;
                                    return;
                                };

                                tracing::debug!(%method_path, %subscription_name, "Received session for method");

                                let result = match handler_info {
                                    HandlerInfo::Stream(stream_handler, handler_type) => {
                                        // For stream-based methods, create StreamRpcSession
                                        let stream_session = StreamRpcSession::new(&session_tx, session_rx, method_path.clone());
                                        stream_session.handle(stream_handler, handler_type).await
                                    }
                                    _ => {
                                        // For unary methods, create RpcSession
                                        let session = RpcSession::new(&session_tx, &mut session_rx, method_path.clone());
                                        session.handle(handler_info).await
                                    }
                                };

                                if let Err(e) = result {
                                    tracing::error!(%method_path, error = %e, "Error handling session");

                                    // Send error to client before closing
                                    let _ = send_error(&session_tx, e).await;
                                }

                                // Close the session after handling (success or after sending error)
                                let _ = session_tx.close(app_clone.as_ref()).await;
                            } => {}
                        }
                    });

                    tasks.push(handle);
                }
            }
        }

        // Wait for all tasks to finish
        tracing::debug!("Waiting for {} session tasks to complete", tasks.len());
        let results = join_all(tasks).await;

        // Log any panicked tasks
        let mut panicked_count = 0;
        for (idx, result) in results.iter().enumerate() {
            if let Err(e) = result {
                tracing::error!(task_index = idx, error = %e, "Session task panicked");
                panicked_count += 1;
            }
        }

        if panicked_count > 0 {
            tracing::warn!("{} session tasks panicked during shutdown", panicked_count);
        } else {
            tracing::info!("All session tasks completed successfully");
        }

        (rx, Ok(()))
    }

    /// Shutdown the server gracefully
    ///
    /// This signals all active session handlers to terminate and waits for them to drain.
    /// After shutdown completes, the server can be restarted by calling [`serve`](Self::serve) again.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use slim_bindings::{Server, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
    /// # use std::sync::Arc;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
    /// # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
    /// # let app = App::new(app_name, provider, verifier)?;
    /// # let core_app = app.inner();
    /// # let notification_rx = app.notification_receiver();
    /// # let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
    /// # let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
    /// // In another task or signal handler:
    /// # slim_bindings::get_runtime().block_on(async {
    /// server.shutdown_internal().await;
    /// # });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown_internal(&self) {
        tracing::info!("Shutting down SlimRPC server");

        // Take the drain signal and watch
        let drain_signal = self.drain_signal.write().take();
        let drain_watch = self.drain_watch.write().take();

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
        *self.drain_signal.write() = Some(new_signal);
        *self.drain_watch.write() = Some(new_watch);

        tracing::debug!("Server shutdown complete, ready to restart");
    }

    /// Listen for an incoming session from the notification receiver
    async fn listen_for_session(
        notification_rx: &mut NotificationReceiver,
    ) -> Result<slim_session::context::SessionContext, Status> {
        tracing::debug!("Waiting for incoming session notification");
        let notification_opt = match notification_rx {
            NotificationReceiver::Owned(rx) => rx.recv().await,
            NotificationReceiver::Shared(rx_arc) => {
                // For shared receiver, put it back immediately and work with the Arc
                tracing::debug!("Acquiring shared notification receiver");

                // Now lock and receive from the shared receiver
                let mut rx = rx_arc.write().await;
                tracing::debug!("Receiving from shared notification receiver");
                rx.recv().await
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
    pub fn new(app: &Arc<crate::App>, base_name: Arc<crate::Name>) -> Self {
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
        app: &Arc<crate::App>,
        base_name: Arc<crate::Name>,
        connection_id: Option<u64>,
    ) -> Self {
        let app_inner = app.inner();
        let rx = app.notification_receiver();

        Self::new_with_shared_rx_and_connection(
            app_inner,
            base_name.as_ref().into(),
            connection_id,
            rx,
            Some(crate::get_runtime().handle().clone()),
        )
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
        handler: Arc<dyn UnaryUnaryHandler>,
    ) {
        let service_clone = service_name.clone();
        let method_clone = method_name.clone();

        tracing::debug!(service = %service_clone, method = %method_clone, "Registering unary-unary handler");

        self.register_unary_unary_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: Context| {
                let handler = handler.clone();
                tracing::debug!(service = %service_clone, method = %method_clone, "Handling unary-unary request");

                Box::pin(async move {
                    handler.handle(request, Arc::new(context)).await.map_err(|e| e.into())
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
        self.register_unary_stream_internal(
            &service_name,
            &method_name,
            move |request: Vec<u8>, context: Context| {
                let handler = handler.clone();

                Box::pin(async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        crate::get_runtime().spawn(async move {
                            if let Err(e) = handler
                                .handle(request, Arc::new(context), sink.clone())
                                .await
                            {
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
        handler: Arc<dyn StreamUnaryHandler>,
    ) {
        self.register_stream_unary_internal(
            &service_name,
            &method_name,
            move |stream: RequestStream<Vec<u8>>, context: Context| {
                let handler = handler.clone();
                let request_stream = Arc::new(UniffiRequestStream::new(stream));

                Box::pin(async move {
                    handler
                        .handle(request_stream, Arc::new(context))
                        .await
                        .map_err(|e| e.into())
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
        self.register_stream_stream_internal(
            &service_name,
            &method_name,
            move |stream: RequestStream<Vec<u8>>, context: Context| {
                let handler = handler.clone();
                let request_stream = Arc::new(UniffiRequestStream::new(stream));

                Box::pin(async move {
                    let (sink, rx) = ResponseSink::receiver();
                    let sink_arc = Arc::new(sink);

                    // Spawn a task to run the handler
                    let handler_task = {
                        let sink = sink_arc.clone();
                        crate::get_runtime().spawn(async move {
                            if let Err(e) = handler
                                .handle(request_stream, Arc::new(context), sink.clone())
                                .await
                            {
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
    pub fn serve(&self) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.serve_async())
    }

    /// Start serving RPC requests (async version)
    ///
    /// This is an async method that runs until the server is shut down.
    /// It listens for incoming RPC calls and dispatches them to registered handlers.
    pub async fn serve_async(&self) -> Result<(), RpcError> {
        let handle = self.serve_handle()?;

        // Wait for the server task to complete and restore the NotificationReceiver
        let (notification_rx, result) = handle
            .await
            .map_err(|e| RpcError::new(Code::Internal, format!("Server task panicked: {}", e)))?;

        // Restore the NotificationReceiver back to the server
        *self.notification_rx.lock() = Some(notification_rx);

        // Return the result from the server task
        result.map_err(|e| RpcError::new(Code::Internal, e.to_string()))
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

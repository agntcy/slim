// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SlimRPC - gRPC-like RPC framework over SLIM
//!
//! SlimRPC provides a gRPC-compatible RPC framework built on top of the SLIM messaging protocol.
//! It supports all standard gRPC interaction patterns:
//! - Unary-Unary: Single request, single response
//! - Unary-Stream: Single request, streaming responses
//! - Stream-Unary: Streaming requests, single response
//! - Stream-Stream: Streaming requests, streaming responses
//!
//! ## Architecture
//!
//! SlimRPC uses SLIM sessions as the underlying transport mechanism. Each RPC call creates
//! a new session, exchanges messages, and closes the session upon completion.
//!
//! ## Core Types
//!
//! This crate works directly with core SLIM types:
//! - `slim_service::app::App` - The SLIM application instance
//! - `slim_session::context::SessionContext` - Session context for RPC calls
//! - `slim_datapath::messages::Name` - SLIM names for service addressing
//!
//! ## Client Example
//!
//! ```no_run
//! # use slim_rpc::{Channel, Context, Status};
//! # use slim_datapath::messages::Name;
//! # use slim_service::app::App;
//! # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
//! # use std::sync::Arc;
//! # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) -> std::result::Result<(), Status> {
//! # #[derive(Default)]
//! # struct Request {}
//! # impl slim_rpc::Encoder for Request {
//! #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
//! # }
//! # #[derive(Default)]
//! # struct Response {}
//! # impl slim_rpc::Decoder for Response {
//! #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Response::default()) }
//! # }
//! # let request = Request::default();
//! // Create a channel
//! let remote = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
//! let channel = Channel::new(app.clone(), remote);
//!
//! // Make an RPC call (typically through generated code)
//! let response: Response = channel.unary("MyService", "MyMethod", request, None, None).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Server Example
//!
//! ```no_run
//! # use slim_rpc::{Server, Context, Status};
//! # use slim_datapath::messages::Name;
//! # use slim_service::app::App;
//! # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
//! # use std::sync::Arc;
//! # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>, notification_rx: tokio::sync::mpsc::Receiver<std::result::Result<slim_session::notification::Notification, slim_session::errors::SessionError>>) -> std::result::Result<(), Status> {
//! # #[derive(Default)]
//! # struct Request {}
//! # impl slim_rpc::Decoder for Request {
//! #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(Request::default()) }
//! # }
//! # #[derive(Default)]
//! # struct Response {}
//! # impl slim_rpc::Encoder for Response {
//! #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
//! # }
//! // Register and run server
//! let base_name = Name::from_strings(["org".to_string(), "namespace".to_string(), "service".to_string()]);
//! let server = Server::new(app, base_name, notification_rx);
//!
//! // Register a handler
//! server.register_unary_unary(
//!     "MyService",
//!     "MyMethod",
//!     |request: Request, _ctx: Context| async move {
//!         Ok(Response::default())
//!     }
//! );
//!
//! // Start serving in background task
//! let server_handle = server.serve();
//!
//! // Wait for server to complete
//! server_handle.await.unwrap()?;
//! # Ok(())
//! # }
//! ```

use slim_datapath::messages::Name;

/// Build a method-specific subscription name (base-service-method)
///
/// This creates a subscription name in the format: `org/namespace/app-service-method`
/// matching the Python implementation's `handler_name_to_pyname`.
///
/// # Arguments
/// * `base_name` - The base name (e.g., "org/namespace/app")
/// * `service_name` - The service name (e.g., "MyService")
/// * `method_name` - The method name (e.g., "MyMethod")
///
/// # Panics
/// Panics if base_name doesn't have at least 3 components
pub fn build_method_subscription_name(
    base_name: &Name,
    service_name: &str,
    method_name: &str,
) -> Name {
    let components_strings = base_name.components_strings();
    if components_strings.len() < 3 {
        panic!("Base name must have at least 3 components");
    }

    // Create subscription name: org/namespace/app-service-method
    let app_with_method = format!(
        "{}-{}-{}",
        &components_strings[2], service_name, method_name
    );

    Name::from_strings([
        components_strings[0].clone(),
        components_strings[1].clone(),
        app_with_method,
    ])
}

mod channel;
mod codec;
mod context;
mod metadata;
mod server;
mod session_wrapper;
mod status;

// UniFFI-specific modules
mod handler_traits;
mod stream_types;

pub use channel::Channel;
pub use codec::{Codec, Decoder, Encoder};
pub use context::{Context, MessageContext, SessionContext};
pub use metadata::Metadata;
pub use server::{HandlerResponse, HandlerType, Server};
pub use session_wrapper::{ReceivedMessage, Session};
pub use status::{Code, Status, StatusError, RpcError};

// UniFFI handler traits and stream types
pub use handler_traits::{
    UnaryUnaryHandler, UnaryStreamHandler, StreamUnaryHandler, StreamStreamHandler,
    UniffiContext,
};
pub use stream_types::{
    RequestStream as UniffiRequestStream, ResponseSink, StreamMessage,
    ResponseStreamReader, RequestStreamWriter, BidiStreamHandler,
};

/// Key used in metadata for RPC deadline/timeout
pub const DEADLINE_KEY: &str = "slimrpc-timeout";

/// Key used in metadata for RPC status code
pub const STATUS_CODE_KEY: &str = "slimrpc-code";

/// Maximum timeout in seconds (10 hours)
pub const MAX_TIMEOUT: u64 = 36000;

/// Result type for SlimRPC operations
pub type Result<T> = std::result::Result<T, Status>;

/// Type alias for request streams in stream-based RPC handlers
///
/// This represents a pinned, boxed stream of requests that can be used
/// in stream-unary and stream-stream RPC handlers.
///
/// # Example
///
/// ```no_run
/// # use slim_rpc::{RequestStream, Status};
/// # use futures::StreamExt;
/// # #[derive(Default)]
/// # struct MyRequest {}
/// # impl slim_rpc::Decoder for MyRequest {
/// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(MyRequest::default()) }
/// # }
/// # #[derive(Default)]
/// # struct MyResponse {}
/// # impl slim_rpc::Encoder for MyResponse {
/// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
/// # }
/// async fn handler(mut stream: RequestStream<MyRequest>) -> std::result::Result<MyResponse, Status> {
///     while let Some(request) = stream.next().await {
///         let req = request?;
///         // Process request
///     }
///     Ok(MyResponse::default())
/// }
/// ```
pub type RequestStream<T> = futures::stream::BoxStream<'static, Result<T>>;

/// Type alias for response streams in stream-based RPC handlers
///
/// This represents a stream of responses that can be returned from
/// unary-stream and stream-stream RPC handlers.
///
/// Note: While this type can represent the return value, handlers typically
/// return concrete stream types (like those from `futures::stream::iter`) which
/// are then automatically converted.
///
/// # Example
///
/// ```no_run
/// # use slim_rpc::{Status, Result};
/// # use futures::stream::{self, Stream};
/// # #[derive(Default, Clone)]
/// # struct MyRequest {}
/// # impl slim_rpc::Decoder for MyRequest {
/// #     fn decode(_buf: impl Into<Vec<u8>>) -> std::result::Result<Self, Status> { Ok(MyRequest::default()) }
/// # }
/// # #[derive(Default, Clone)]
/// # struct MyResponse {}
/// # impl slim_rpc::Encoder for MyResponse {
/// #     fn encode(self) -> std::result::Result<Vec<u8>, Status> { Ok(vec![]) }
/// # }
/// async fn handler(request: MyRequest) -> std::result::Result<impl Stream<Item = Result<MyResponse>>, Status> {
///     let responses = vec![MyResponse::default(), MyResponse::default(), MyResponse::default()];
///     Ok(stream::iter(responses.into_iter().map(Ok)))
/// }
/// ```
pub type ResponseStream<T> = futures::stream::BoxStream<'static, Result<T>>;

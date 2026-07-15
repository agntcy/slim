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
//! SlimRPC uses SLIM sessions as the underlying transport mechanism. A persistent session
//! is maintained per remote peer and shared across concurrent RPC calls. Each call is
//! identified by a unique `rpc-id` in message metadata, which allows the server to
//! demultiplex concurrent calls over the shared session.
//!
//! ## Core Types
//!
//! This crate works directly with core SLIM types:
//! - `slim_service::app::App` - The SLIM application instance
//! - `slim_session::context::SessionContext` - Session context for RPC calls
//! - `slim_datapath::api::ProtoName` - SLIM names for service addressing
//!
//! ## Client Example
//!
//! ```no_run
//! # use slim_bindings::{Channel, Context, RpcError, Encoder, Decoder};
//! # use slim_bindings::Name;
//! # use slim_service::app::App;
//! # use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
//! # use std::sync::Arc;
//! # async fn example(app: Arc<App<AuthProvider, AuthVerifier>>) -> Result<(), RpcError> {
//! # #[derive(Default)]
//! # struct Request {}
//! # impl Encoder for Request {
//! #     fn encode(self) -> Result<Vec<u8>, RpcError> { Ok(vec![]) }
//! # }
//! # #[derive(Default)]
//! # struct Response {}
//! # impl Decoder for Response {
//! #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> { Ok(Response::default()) }
//! # }
//! # let request = Request::default();
//! // Create a channel
//! let remote = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
//! let channel = Channel::new_with_members_internal(app.clone(), vec![remote.as_slim_name().clone()], false, None).unwrap();
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
//! # use slim_bindings::{Server, Context, RpcError, Encoder, Decoder, App, Name, IdentityProviderConfig, IdentityVerifierConfig};
//! # use std::sync::Arc;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let app_name = Arc::new(Name::new("test".to_string(), "app".to_string(), "v1".to_string()));
//! # let provider = IdentityProviderConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
//! # let verifier = IdentityVerifierConfig::SharedSecret { id: "test".to_string(), data: "secret".to_string() };
//! # let app = App::new(app_name, provider, verifier)?;
//! # let core_app = app.inner();
//! # let notification_rx = app.notification_receiver();
//! # #[derive(Default)]
//! # struct Request {}
//! # impl Decoder for Request {
//! #     fn decode(_buf: impl Into<Vec<u8>>) -> Result<Self, RpcError> { Ok(Request::default()) }
//! # }
//! # #[derive(Default)]
//! # struct Response {}
//! # impl Encoder for Response {
//! #     fn encode(self) -> Result<Vec<u8>, RpcError> { Ok(vec![]) }
//! # }
//! // Register and run server
//! let base_name = Name::new("org".to_string(), "namespace".to_string(), "service".to_string());
//! let server = Server::new_with_shared_rx_and_connection(core_app, base_name.as_slim_name(), None, notification_rx, None);
//!
//! // Register a handler
//! server.register_unary_unary_internal(
//!     "MyService",
//!     "MyMethod",
//!     |request: Request, _ctx: Context| async move {
//!         Ok(Response::default())
//!     }
//! );
//! # Ok(())
//! # }
//! ```

// FFI wrapper modules: thin UniFFI-facing newtype wrappers over the
// corresponding `slim_rpc` types. All the RPC logic lives in the
// `agntcy-slim-rpc` crate; these modules only adapt the public UniFFI API
// (FFI `crate::App` / `crate::Name` parameters).
mod channel;
mod server;
mod stream_types;

pub use channel::Channel;
pub use server::Server;

// Core RPC types re-exported from the `slim_rpc` crate (single source of truth).
pub use slim_rpc::{
    Codec, Context, Decoder, Encoder, HandlerInfo, HandlerType, InvalidRpcCode, MessageContext,
    Metadata, MulticastItem, ReceivedMessage, RpcCode, RpcError, RpcHandler, RpcSession,
    SessionContext, SessionRx, SessionTx, StreamRpcHandler, StreamStreamHandler,
    StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler, build_method_subscription_name,
    calculate_deadline, calculate_timeout_duration, new_session, send_eos, send_error,
    send_error_for_rpc, send_response_stream,
};

// Metadata keys, timeout constant and RPC type aliases re-exported from `slim_rpc`.
pub use slim_rpc::{
    DEADLINE_KEY, MAX_TIMEOUT, METHOD_KEY, RPC_DIR_KEY, RPC_DIR_REQ, RPC_ID_KEY, RequestStream,
    ResponseStream, Result, SERVICE_KEY, STATUS_CODE_KEY,
};

// UniFFI stream types. Non-multicast types come straight from `slim_rpc`; the
// multicast types are re-exported from the local `stream_types` module because
// their FFI-facing versions use `crate::Name` (see `stream_types.rs`).
pub use stream_types::{
    BidiStreamHandler, DecodedStream, MulticastBidiStreamHandler, MulticastResponseReader,
    MulticastStreamMessage, RawStream, RequestStream as UniffiRequestStream, RequestStreamWriter,
    ResponseSink, ResponseStreamReader, RpcMessageContext, RpcMulticastItem, StreamMessage,
    StreamSource,
};

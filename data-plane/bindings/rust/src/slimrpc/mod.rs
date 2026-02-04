// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SlimRPC UniFFI Bindings
//!
//! This module provides UniFFI-compatible wrappers for the SlimRPC library.
//! Since UniFFI doesn't support Rust async streams directly, we provide:
//!
//! - Callback trait interfaces for handler implementations
//! - Stream wrapper objects for request/response streaming
//! - Synchronous and async-compatible APIs
//!
//! ## Architecture
//!
//! Applications implement the `RpcHandler` trait to handle RPC calls.
//! For streaming cases, the handler receives wrapper objects:
//! - `RequestStream`: Pull messages from client stream
//! - `ResponseSink`: Push messages to client stream
//!
//! ## Example
//!
//! ```rust,ignore
//! // Implement the handler trait
//! struct MyHandler;
//!
//! impl RpcHandler for MyHandler {
//!     fn handle_unary_unary(&self, request: Vec<u8>, context: RpcContext) -> Result<Vec<u8>, RpcError> {
//!         // Process unary request, return unary response
//!         Ok(response_bytes)
//!     }
//!
//!     fn handle_unary_stream(&self, request: Vec<u8>, context: RpcContext, sink: Arc<ResponseSink>) -> Result<(), RpcError> {
//!         // Process request, push multiple responses to sink
//!         sink.send(response1)?;
//!         sink.send(response2)?;
//!         sink.close()?;
//!         Ok(())
//!     }
//!
//!     fn handle_stream_unary(&self, stream: Arc<RequestStream>, context: RpcContext) -> Result<Vec<u8>, RpcError> {
//!         // Pull messages from stream, return single response
//!         while let Some(msg) = stream.next()? {
//!             // Process message
//!         }
//!         Ok(final_response)
//!     }
//!
//!     fn handle_stream_stream(&self, stream: Arc<RequestStream>, context: RpcContext, sink: Arc<ResponseSink>) -> Result<(), RpcError> {
//!         // Pull from stream, push to sink
//!         while let Some(msg) = stream.next()? {
//!             sink.send(process(msg))?;
//!         }
//!         sink.close()?;
//!         Ok(())
//!     }
//! }
//! ```

mod channel;
mod context;
mod error;
mod handler;
mod server;
mod types;

// Re-export core slimrpc types that don't need wrapping
pub use crate::slimrpc_core::{
    Code, Codec, DEADLINE_KEY, Decoder, Encoder, MAX_TIMEOUT, Metadata, STATUS_CODE_KEY,
};

// Re-export our wrapper types
pub use channel::{Channel as RpcChannel, ResponseStreamReader};
pub use context::{Context as RpcContext, RpcMessageContext, SessionContext as RpcSessionContext};
pub use error::{RpcCode, RpcError, Status, StatusError};
pub use handler::{
    HandlerResponse, HandlerType, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler,
    UnaryUnaryHandler,
};
pub use server::Server as RpcServer;
pub use types::{RequestStream, ResponseSink, StreamMessage};

// Legacy compatibility re-exports
pub use channel::Channel;
pub use context::Context;
pub use crate::slimrpc_core::Server;

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
//! ```rust,ignore
//! use slimrpc::{Channel, Context};
//! use slim_datapath::messages::Name;
//! use std::sync::Arc;
//!
//! // Create a channel
//! let remote = Name::from_strings(["org".into(), "namespace".into(), "service".into());
//! let channel = Channel::new(app.clone(), remote);
//!
//! // Make an RPC call (typically through generated code)
//! let response = channel.unary("MyService", "MyMethod", request, None, None).await?;
//! ```
//!
//! ## Server Example
//!
//! ```rust,ignore
//! use slimrpc::{Server, Context, Status};
//! use async_trait::async_trait;
//!
//! // Implement service trait (typically generated from protobuf)
//! struct MyServiceImpl;
//!
//! #[async_trait]
//! impl MyService for MyServiceImpl {
//!     async fn my_method(&self, request: Request, context: Context) -> Result<Response, Status> {
//!         Ok(Response::default())
//!     }
//! }
//!
//! // Register and run server
//! let base_name = Name::from_strings(["org".into(), "namespace".into(), "service".into());
//! let mut server = Server::new(app, base_name, None);
//! server.registry().register_unary_unary("MyService", "MyMethod", handler);
//! server.serve().await?;
//! ```

mod channel;
mod codec;
mod context;
mod metadata;
mod server;
mod session_wrapper;
mod status;

#[cfg(test)]
mod e2e_tests;

pub use channel::Channel;
pub use codec::{Codec, Decoder, Encoder};
pub use context::{Context, MessageContext, SessionContext};
pub use metadata::Metadata;
pub use server::{HandlerResponse, HandlerType, Server, ServiceRegistry};
pub use session_wrapper::{ReceivedMessage, Session};
pub use status::{Code, Status, StatusError};

/// Key used in metadata for RPC deadline/timeout
pub const DEADLINE_KEY: &str = "slimrpc-deadline";

/// Key used in metadata for RPC status code
pub const STATUS_CODE_KEY: &str = "slimrpc-code";

/// Maximum timeout in seconds (10 hours)
pub const MAX_TIMEOUT: u64 = 36000;

/// Result type for SlimRPC operations
pub type Result<T> = std::result::Result<T, Status>;
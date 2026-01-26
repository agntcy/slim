// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SlimRPC Bindings - UniFFI Language Bindings
//!
//! This module provides language-agnostic FFI bindings for SlimRPC using UniFFI.
//! It enables integration with Go, Python, Kotlin, Swift, and other languages.
//!
//! ## Architecture
//!
//! The module wraps the core slimrpc types to be UniFFI-compatible:
//!
//! - **`RpcChannel`**: Client-side channel for making RPC calls
//! - **`RpcServer`**: Server-side RPC server for handling requests
//! - **`RpcContext`**: Context information for RPC handlers
//! - **`Status`**: RPC status codes and error information
//! - **`Metadata`**: Key-value metadata for RPC calls
//!
//! ## Usage
//!
//! ### Client Example
//!
//! ```rust,ignore
//! use slim_bindings::{App, RpcChannel};
//!
//! // Create app
//! let app = App::new(app_name, provider_config, verifier_config, false)?;
//!
//! // Create RPC channel
//! let remote = Name { components: vec!["org".into(), "ns".into(), "service".into()], id: None };
//! let channel = RpcChannel::new(Arc::new(app), remote);
//!
//! // Make RPC call (typically through generated code)
//! let response = channel.unary("MyService", "MyMethod", request, None, None).await?;
//! ```

mod channel;
mod codec;
mod context;
mod error;
mod metadata;
mod rpc;
mod server;

pub use channel::{RequestSender, RpcChannel, VectorRequestSender};
pub use codec::{Codec, Decoder, Encoder};
pub use context::{RpcContext, RpcMessageContext, RpcSessionContext};
pub use error::{Code, RpcError, Status, StatusError};
pub use metadata::Metadata;
pub use rpc::{
    RequestStream, ResponseStream, StreamResult, StreamStreamHandler, StreamUnaryHandler,
    UnaryStreamHandler, UnaryUnaryHandler,
};
pub use server::{HandlerResponse, HandlerType, RpcHandler, RpcResponseStream, RpcServer};

// Re-export core types with RPC prefix for clarity
pub use agntcy_slimrpc::{
    Channel, Context, DEADLINE_KEY, MAX_TIMEOUT, MessageContext as CoreMessageContext,
    STATUS_CODE_KEY, Server, SessionContext as CoreSessionContext,
};

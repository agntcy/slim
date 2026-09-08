// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Handler trait interface for SlimRPC UniFFI bindings
//!
//! Defines the callback traits that foreign language applications implement to handle RPC calls.
//! UniFFI will generate foreign language interfaces for these traits.

use std::sync::Arc;

use super::{Context, RpcError, stream_types::{RequestStream, ResponseSink}};

#[cfg(feature = "uniffi")]
use super::stream_types::UniffiPeerResponseStream;

/// Unary-to-Unary RPC handler trait
///
/// Implement this trait to handle unary-to-unary RPC calls.
/// The handler receives a single request and returns a single response.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait UnaryUnaryHandler: Send + Sync {
    /// Handle a unary-to-unary RPC call
    ///
    /// # Arguments
    /// * `request` - The request message bytes
    /// * `context` - RPC context with metadata and session information
    ///
    /// # Returns
    /// The response message bytes or an error
    async fn handle(&self, request: Vec<u8>, context: Arc<Context>) -> Result<Vec<u8>, RpcError>;
}

/// Unary-to-Stream RPC handler trait
///
/// Implement this trait to handle unary-to-stream RPC calls.
/// The handler receives a single request and sends multiple responses via the sink.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait UnaryStreamHandler: Send + Sync {
    /// Handle a unary-to-stream RPC call
    ///
    /// # Arguments
    /// * `request` - The request message bytes
    /// * `context` - RPC context with metadata and session information
    /// * `sink` - Response sink to send streaming responses
    ///
    /// # Returns
    /// Ok(()) if handling succeeded, or an error
    ///
    /// # Note
    /// You must call `sink.close()` or `sink.send_error()` when done.
    async fn handle(
        &self,
        request: Vec<u8>,
        context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError>;
}

/// Stream-to-Unary RPC handler trait
///
/// Implement this trait to handle stream-to-unary RPC calls.
/// The handler receives multiple requests via the stream and returns a single response.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait StreamUnaryHandler: Send + Sync {
    /// Handle a stream-to-unary RPC call
    ///
    /// # Arguments
    /// * `stream` - Request stream to pull messages from
    /// * `context` - RPC context with metadata and session information
    ///
    /// # Returns
    /// The response message bytes or an error
    async fn handle(
        &self,
        stream: Arc<RequestStream>,
        context: Arc<Context>,
    ) -> Result<Vec<u8>, RpcError>;
}

/// Stream-to-Stream RPC handler trait
///
/// Implement this trait to handle stream-to-stream RPC calls.
/// The handler receives multiple requests via the stream and sends multiple responses via the sink.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait StreamStreamHandler: Send + Sync {
    /// Handle a stream-to-stream RPC call
    ///
    /// # Arguments
    /// * `stream` - Request stream to pull messages from
    /// * `context` - RPC context with metadata and session information
    /// * `sink` - Response sink to send streaming responses
    ///
    /// # Returns
    /// Ok(()) if handling succeeded, or an error
    ///
    /// # Note
    /// You must call `sink.close()` or `sink.send_error()` when done.
    async fn handle(
        &self,
        stream: Arc<RequestStream>,
        context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError>;
}

// ── Shared-responses handler traits ────────────────────────────────────────

/// Unary-to-Unary shared-responses RPC handler trait
///
/// Like [`UnaryUnaryHandler`] but also receives a stream of response messages
/// from peer servers in the multicast GROUP.
#[cfg(feature = "uniffi")]
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait UnaryUnarySharedHandler: Send + Sync {
    async fn handle(
        &self,
        request: Vec<u8>,
        context: Arc<Context>,
        peer_stream: Arc<UniffiPeerResponseStream>,
    ) -> Result<Vec<u8>, RpcError>;
}

/// Unary-to-Stream shared-responses RPC handler trait
///
/// Like [`UnaryStreamHandler`] but also receives peer response messages.
#[cfg(feature = "uniffi")]
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait UnaryStreamSharedHandler: Send + Sync {
    async fn handle(
        &self,
        request: Vec<u8>,
        context: Arc<Context>,
        sink: Arc<ResponseSink>,
        peer_stream: Arc<UniffiPeerResponseStream>,
    ) -> Result<(), RpcError>;
}

/// Stream-to-Unary shared-responses RPC handler trait
///
/// Like [`StreamUnaryHandler`] but also receives peer response messages.
#[cfg(feature = "uniffi")]
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait StreamUnarySharedHandler: Send + Sync {
    async fn handle(
        &self,
        stream: Arc<RequestStream>,
        context: Arc<Context>,
        peer_stream: Arc<UniffiPeerResponseStream>,
    ) -> Result<Vec<u8>, RpcError>;
}

/// Stream-to-Stream shared-responses RPC handler trait
///
/// Like [`StreamStreamHandler`] but also receives peer response messages.
#[cfg(feature = "uniffi")]
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[async_trait::async_trait]
pub trait StreamStreamSharedHandler: Send + Sync {
    async fn handle(
        &self,
        stream: Arc<RequestStream>,
        context: Arc<Context>,
        sink: Arc<ResponseSink>,
        peer_stream: Arc<UniffiPeerResponseStream>,
    ) -> Result<(), RpcError>;
}

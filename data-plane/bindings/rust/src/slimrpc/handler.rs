// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Handler trait interface for SlimRPC UniFFI bindings
//!
//! Defines the callback trait that applications implement to handle RPC calls.
//! UniFFI will generate foreign language interfaces for this trait.

use std::sync::Arc;

use crate::slimrpc::context::Context;
use crate::slimrpc::error::RpcError;
use crate::slimrpc::types::{RequestStream, ResponseSink};

// Re-export handler types from core
pub use crate::slimrpc_core::HandlerType;

/// Response from an RPC handler
///
/// UniFFI-compatible version of the handler response.
#[derive(uniffi::Enum)]
pub enum HandlerResponse {
    /// Single response message
    Unary(Vec<u8>),
    /// Streaming response (handled via ResponseSink in callbacks)
    Stream,
}

/// Unary-to-Unary RPC handler trait
///
/// Implement this trait to handle unary-to-unary RPC calls.
/// The handler receives a single request and returns a single response.
#[uniffi::export(with_foreign)]
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
#[uniffi::export(with_foreign)]
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
#[uniffi::export(with_foreign)]
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
#[uniffi::export(with_foreign)]
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

#[cfg(test)]
mod tests {
    use super::*;

    // Test handler implementations
    struct TestUnaryUnaryHandler;

    #[async_trait::async_trait]
    impl UnaryUnaryHandler for TestUnaryUnaryHandler {
        async fn handle(
            &self,
            request: Vec<u8>,
            _context: Arc<Context>,
        ) -> Result<Vec<u8>, RpcError> {
            // Echo the request
            Ok(request)
        }
    }

    struct TestUnaryStreamHandler;

    #[async_trait::async_trait]
    impl UnaryStreamHandler for TestUnaryStreamHandler {
        async fn handle(
            &self,
            request: Vec<u8>,
            _context: Arc<Context>,
            sink: Arc<ResponseSink>,
        ) -> Result<(), RpcError> {
            // Send the request back 3 times
            for _ in 0..3 {
                sink.send_async(request.clone()).await?;
            }
            sink.close_async().await?;
            Ok(())
        }
    }

    struct TestStreamUnaryHandler;

    #[async_trait::async_trait]
    impl StreamUnaryHandler for TestStreamUnaryHandler {
        async fn handle(
            &self,
            stream: Arc<RequestStream>,
            _context: Arc<Context>,
        ) -> Result<Vec<u8>, RpcError> {
            let mut combined = Vec::new();
            loop {
                match stream.next_async().await {
                    crate::slimrpc::types::StreamMessage::Data(data) => {
                        combined.extend_from_slice(&data);
                    }
                    crate::slimrpc::types::StreamMessage::Error(e) => {
                        return Err(e);
                    }
                    crate::slimrpc::types::StreamMessage::End => {
                        break;
                    }
                }
            }
            Ok(combined)
        }
    }

    struct TestStreamStreamHandler;

    #[async_trait::async_trait]
    impl StreamStreamHandler for TestStreamStreamHandler {
        async fn handle(
            &self,
            stream: Arc<RequestStream>,
            _context: Arc<Context>,
            sink: Arc<ResponseSink>,
        ) -> Result<(), RpcError> {
            loop {
                match stream.next_async().await {
                    crate::slimrpc::types::StreamMessage::Data(data) => {
                        // Echo each message
                        sink.send_async(data).await?;
                    }
                    crate::slimrpc::types::StreamMessage::Error(e) => {
                        return Err(e);
                    }
                    crate::slimrpc::types::StreamMessage::End => {
                        break;
                    }
                }
            }
            sink.close_async().await?;
            Ok(())
        }
    }

    #[test]
    fn test_handler_traits_compile() {
        // This test just ensures the traits compile correctly
        let _h1 = TestUnaryUnaryHandler;
        let _h2 = TestUnaryStreamHandler;
        let _h3 = TestStreamUnaryHandler;
        let _h4 = TestStreamStreamHandler;
    }
}

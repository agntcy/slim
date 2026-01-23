// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Handler and Stream Traits for UniFFI
//!
//! This module defines the async trait interfaces that foreign language code
//! can implement to handle RPC requests. It supports all four gRPC interaction
//! patterns with proper streaming support.

use std::pin::Pin;
use std::sync::Arc;

use futures::stream::{Stream, StreamExt};

use super::context::RpcContext;
use super::error::Status;

/// Result from a stream operation
#[derive(Debug, Clone, uniffi::Enum)]
pub enum StreamResult {
    /// Successfully received/sent data
    Data { value: Vec<u8> },
    /// Stream ended normally
    End,
    /// Error occurred
    Error { status: Status },
}

/// Async trait for request streams that can be awaited by foreign language handlers
///
/// Foreign language implementations receive this trait object to read
/// incoming requests in stream-based RPC methods.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait RequestStream: Send + Sync {
    /// Get the next request from the stream
    ///
    /// Returns:
    /// - `StreamResult::Data` with the next request bytes
    /// - `StreamResult::End` when the stream is exhausted
    /// - `StreamResult::Error` if an error occurs
    async fn next(&self) -> StreamResult;
}

/// Async trait for response streams that Rust can use to send responses
///
/// Foreign language implementations use this trait object to send
/// responses back to the client in stream-based RPC methods.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait ResponseStream: Send + Sync {
    /// Send a response to the stream
    ///
    /// # Arguments
    /// * `response` - The serialized response bytes to send
    ///
    /// # Returns
    /// Error status if sending fails
    async fn send(&self, response: Vec<u8>) -> Result<(), Status>;
    
    /// Close the stream
    ///
    /// Should be called when done sending responses to properly
    /// close the stream and signal completion to the client.
    async fn close(&self);
}

/// Async trait for unary-unary RPC handlers
///
/// Implement this trait in your language to handle unary-unary RPC calls.
/// This is the simplest pattern: single request → single response.
///
/// # Example (Rust)
///
/// ```rust,ignore
/// struct EchoHandler;
///
/// #[async_trait::async_trait]
/// impl UnaryUnaryHandler for EchoHandler {
///     async fn handle(&self, request: Vec<u8>, _context: Arc<RpcContext>) -> Result<Vec<u8>, Status> {
///         Ok(request) // Echo back the request
///     }
/// }
/// ```
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait UnaryUnaryHandler: Send + Sync {
    /// Handle a unary-unary RPC call
    ///
    /// # Arguments
    /// * `request` - The serialized request bytes
    /// * `context` - The RPC context containing metadata and session info
    ///
    /// # Returns
    /// The serialized response bytes, or an error status
    async fn handle(&self, request: Vec<u8>, context: Arc<RpcContext>) -> Result<Vec<u8>, Status>;
}

/// Async trait for unary-stream RPC handlers
///
/// Implement this trait in your language to handle unary-stream RPC calls.
/// Pattern: single request → stream of responses.
///
/// # Example (Rust)
///
/// ```rust,ignore
/// struct ReplicateHandler;
///
/// #[async_trait::async_trait]
/// impl UnaryStreamHandler for ReplicateHandler {
///     async fn handle(
///         &self,
///         request: Vec<u8>,
///         _context: Arc<RpcContext>,
///         response_stream: Arc<dyn ResponseStream>,
///     ) -> Result<(), Status> {
///         // Send the request 3 times
///         for _ in 0..3 {
///             response_stream.send(request.clone()).await?;
///         }
///         response_stream.close().await;
///         Ok(())
///     }
/// }
/// ```
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait UnaryStreamHandler: Send + Sync {
    /// Handle a unary-stream RPC call
    ///
    /// # Arguments
    /// * `request` - The serialized request bytes
    /// * `context` - The RPC context containing metadata and session info
    /// * `response_stream` - Stream to send responses to
    ///
    /// # Returns
    /// Error status if the handler fails
    async fn handle(
        &self,
        request: Vec<u8>,
        context: Arc<RpcContext>,
        response_stream: Arc<dyn ResponseStream>,
    ) -> Result<(), Status>;
}

/// Async trait for stream-unary RPC handlers
///
/// Implement this trait in your language to handle stream-unary RPC calls.
/// Pattern: stream of requests → single response.
///
/// # Example (Rust)
///
/// ```rust,ignore
/// struct AggregateHandler;
///
/// #[async_trait::async_trait]
/// impl StreamUnaryHandler for AggregateHandler {
///     async fn handle(
///         &self,
///         request_stream: Arc<dyn RequestStream>,
///         _context: Arc<RpcContext>,
///     ) -> Result<Vec<u8>, Status> {
///         let mut result = Vec::new();
///         loop {
///             match request_stream.next().await {
///                 StreamResult::Data { value } => result.extend(value),
///                 StreamResult::End => break,
///                 StreamResult::Error { status } => return Err(status),
///             }
///         }
///         Ok(result)
///     }
/// }
/// ```
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait StreamUnaryHandler: Send + Sync {
    /// Handle a stream-unary RPC call
    ///
    /// # Arguments
    /// * `request_stream` - Stream to read requests from
    /// * `context` - The RPC context containing metadata and session info
    ///
    /// # Returns
    /// The serialized response bytes, or an error status
    async fn handle(
        &self,
        request_stream: Arc<dyn RequestStream>,
        context: Arc<RpcContext>,
    ) -> Result<Vec<u8>, Status>;
}

/// Async trait for stream-stream RPC handlers
///
/// Implement this trait in your language to handle stream-stream RPC calls.
/// Pattern: stream of requests → stream of responses.
///
/// # Example (Rust)
///
/// ```rust,ignore
/// struct TransformHandler;
///
/// #[async_trait::async_trait]
/// impl StreamStreamHandler for TransformHandler {
///     async fn handle(
///         &self,
///         request_stream: Arc<dyn RequestStream>,
///         _context: Arc<RpcContext>,
///         response_stream: Arc<dyn ResponseStream>,
///     ) -> Result<(), Status> {
///         loop {
///             match request_stream.next().await {
///                 StreamResult::Data { value } => {
///                     let mut transformed = value;
///                     transformed.reverse();
///                     response_stream.send(transformed).await?;
///                 }
///                 StreamResult::End => break,
///                 StreamResult::Error { status } => return Err(status),
///             }
///         }
///         response_stream.close().await;
///         Ok(())
///     }
/// }
/// ```
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait StreamStreamHandler: Send + Sync {
    /// Handle a stream-stream RPC call
    ///
    /// # Arguments
    /// * `request_stream` - Stream to read requests from
    /// * `context` - The RPC context containing metadata and session info
    /// * `response_stream` - Stream to send responses to
    ///
    /// # Returns
    /// Error status if the handler fails
    async fn handle(
        &self,
        request_stream: Arc<dyn RequestStream>,
        context: Arc<RpcContext>,
        response_stream: Arc<dyn ResponseStream>,
    ) -> Result<(), Status>;
}

/// Internal wrapper to convert a Rust stream into a RequestStream trait object
pub(crate) struct RequestStreamWrapper {
    pub(crate) stream: Arc<tokio::sync::Mutex<Pin<Box<dyn Stream<Item = Result<Vec<u8>, agntcy_slimrpc::Status>> + Send>>>>,
}

#[async_trait::async_trait]
impl RequestStream for RequestStreamWrapper {
    async fn next(&self) -> StreamResult {
        let mut stream = self.stream.lock().await;
        match stream.next().await {
            Some(Ok(data)) => StreamResult::Data { value: data },
            Some(Err(e)) => StreamResult::Error { status: Status::from_core(e) },
            None => StreamResult::End,
        }
    }
}

/// Internal wrapper to convert a ResponseStream trait object into a Rust stream
pub(crate) struct ResponseStreamSender {
    pub(crate) tx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, agntcy_slimrpc::Status>>>>,
}

#[async_trait::async_trait]
impl ResponseStream for ResponseStreamSender {
    async fn send(&self, response: Vec<u8>) -> Result<(), Status> {
        let tx = self.tx.lock().await;
        tx.send(Ok(response))
            .map_err(|_| Status::new(super::error::Code::Internal, "Failed to send response"))
    }
    
    async fn close(&self) {
        // Dropping the sender will close the stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_result_enum() {
        let data = StreamResult::Data { value: vec![1, 2, 3] };
        match data {
            StreamResult::Data { value } => assert_eq!(value, vec![1, 2, 3]),
            _ => panic!("Expected Data variant"),
        }

        let end = StreamResult::End;
        match end {
            StreamResult::End => {},
            _ => panic!("Expected End variant"),
        }

        let error = StreamResult::Error {
            status: Status::with_code(super::super::error::Code::Internal),
        };
        match error {
            StreamResult::Error { status } => {
                assert_eq!(status.code, super::super::error::Code::Internal);
            },
            _ => panic!("Expected Error variant"),
        }
    }
}
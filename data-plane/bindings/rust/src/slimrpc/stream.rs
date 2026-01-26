// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Stream utilities for SlimRPC UniFFI bindings
//!
//! Provides utility types and traits for working with streaming RPC operations.

use std::sync::Arc;

use crate::slimrpc::error::RpcError;

/// Result type for stream operations
pub type StreamResult<T> = Result<T, RpcError>;

/// Request sender trait for stream-to-unary and stream-to-stream client calls
///
/// This trait allows applications to send multiple request messages in a streaming fashion.
/// Implementations handle the underlying stream mechanics while providing a simple send interface.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait RequestSender: Send + Sync {
    /// Send a request message to the stream
    ///
    /// # Arguments
    /// * `data` - The request message bytes
    ///
    /// # Returns
    /// Ok(()) if sent successfully, or an error
    async fn send(&self, data: Vec<u8>) -> Result<(), RpcError>;

    /// Close the request stream
    ///
    /// Signals that no more messages will be sent.
    /// This must be called to complete the RPC call.
    async fn close(&self) -> Result<(), RpcError>;
}

/// Vector-based request sender for collecting messages
///
/// A simple implementation of RequestSender that collects all messages
/// into a vector. Useful for testing or simple buffering scenarios.
#[derive(uniffi::Object)]
pub struct VectorRequestSender {
    /// Collected messages
    messages: Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>,
    /// Closed flag
    closed: Arc<tokio::sync::Mutex<bool>>,
}

impl VectorRequestSender {
    /// Create a new vector request sender
    pub fn new() -> Self {
        Self {
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            closed: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    /// Get all collected messages
    pub async fn messages(&self) -> Vec<Vec<u8>> {
        self.messages.lock().await.clone()
    }

    /// Check if the sender is closed
    pub async fn is_closed(&self) -> bool {
        *self.closed.lock().await
    }
}

#[uniffi::export]
impl VectorRequestSender {
    /// Create a new vector request sender
    #[uniffi::constructor]
    pub fn create() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[uniffi::export]
#[async_trait::async_trait]
impl RequestSender for VectorRequestSender {
    async fn send(&self, data: Vec<u8>) -> Result<(), RpcError> {
        let closed = self.closed.lock().await;
        if *closed {
            return Err(RpcError::new(
                crate::slimrpc::error::RpcCode::FailedPrecondition,
                "Request sender is closed".to_string(),
            ));
        }
        drop(closed);

        let mut messages = self.messages.lock().await;
        messages.push(data);
        Ok(())
    }

    async fn close(&self) -> Result<(), RpcError> {
        let mut closed = self.closed.lock().await;
        if *closed {
            return Ok(()); // Already closed, idempotent
        }
        *closed = true;
        Ok(())
    }
}

/// Request stream type alias
///
/// Represents a stream of request messages for stream-based RPC handlers.
pub type RequestStream = crate::slimrpc::types::RequestStream;

/// Response stream type alias
///
/// Represents a stream of response messages.
pub type ResponseStream = crate::slimrpc::types::ResponseStream;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_vector_request_sender() {
        let sender = VectorRequestSender::new();

        sender.send(vec![1, 2, 3]).await.unwrap();
        sender.send(vec![4, 5, 6]).await.unwrap();
        sender.send(vec![7, 8, 9]).await.unwrap();

        let messages = sender.messages().await;
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], vec![1, 2, 3]);
        assert_eq!(messages[1], vec![4, 5, 6]);
        assert_eq!(messages[2], vec![7, 8, 9]);

        sender.close().await.unwrap();
        assert!(sender.is_closed().await);

        // Sending after close should fail
        let result = sender.send(vec![10, 11, 12]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_close_idempotent() {
        let sender = VectorRequestSender::new();

        sender.close().await.unwrap();
        sender.close().await.unwrap(); // Should not error

        assert!(sender.is_closed().await);
    }

    #[tokio::test]
    async fn test_send_after_close_fails() {
        let sender = VectorRequestSender::new();
        sender.close().await.unwrap();

        let result = sender.send(vec![1, 2, 3]).await;
        assert!(result.is_err());
        
        if let Err(e) = result {
            assert_eq!(e.code(), crate::slimrpc::error::RpcCode::FailedPrecondition);
        }
    }
}
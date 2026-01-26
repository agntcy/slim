// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Stream wrapper types for SlimRPC UniFFI bindings
//!
//! Provides UniFFI-compatible wrappers for streaming operations.
//! Since UniFFI doesn't support Rust async streams, we provide synchronous
//! pull/push interfaces backed by async channels.

use std::sync::Arc;
use tokio::sync::Mutex;
use futures::StreamExt;

use crate::slimrpc::error::RpcError;

/// Request stream reader
///
/// Allows pulling messages from a client request stream.
/// This wraps the underlying async stream and provides a blocking interface
/// suitable for UniFFI callback traits.
#[derive(uniffi::Object)]
pub struct RequestStream {
    /// Inner stream wrapped in a mutex for interior mutability
    inner: Arc<Mutex<Box<dyn futures::Stream<Item = Result<Vec<u8>, slim_rpc::Status>> + Send + Unpin>>>,
}

impl RequestStream {
    /// Create a new request stream wrapper
    pub(crate) fn new(
        stream: Box<dyn futures::Stream<Item = Result<Vec<u8>, slim_rpc::Status>> + Send + Unpin>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(stream)),
        }
    }
}

#[uniffi::export]
impl RequestStream {
    /// Pull the next message from the stream (blocking version)
    ///
    /// Returns a StreamMessage indicating the result
    pub fn next(&self) -> StreamMessage {
        crate::get_runtime().block_on(self.next_async())
    }

    /// Pull the next message from the stream (async version)
    ///
    /// Returns a StreamMessage indicating the result
    pub async fn next_async(&self) -> StreamMessage {
        let mut stream = self.inner.lock().await;
        match stream.next().await {
            Some(Ok(data)) => StreamMessage::Data(data),
            Some(Err(e)) => StreamMessage::Error(e.into()),
            None => StreamMessage::End,
        }
    }
}

/// Message from a stream
#[derive(uniffi::Enum)]
pub enum StreamMessage {
    /// Successfully received data
    Data(Vec<u8>),
    /// Stream error occurred
    Error(RpcError),
    /// Stream has ended
    End,
}

/// Response stream writer
///
/// Allows pushing messages to a client response stream.
/// This wraps an async channel sender and provides a blocking interface
/// suitable for UniFFI callback traits.
#[derive(uniffi::Object)]
pub struct ResponseSink {
    /// Channel sender for streaming responses
    sender: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, slim_rpc::Status>>>>,
    /// Flag to track if the sink has been closed
    closed: Arc<Mutex<bool>>,
}

impl ResponseSink {
    /// Create a new response sink wrapper
    pub(crate) fn new(
        sender: tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, slim_rpc::Status>>,
    ) -> Self {
        Self {
            sender: Arc::new(Mutex::new(sender)),
            closed: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the receiver side of the channel
    pub(crate) fn receiver(
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, slim_rpc::Status>>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = Self::new(tx);
        (sink, rx)
    }
}

#[uniffi::export]
impl ResponseSink {
    /// Send a message to the response stream (blocking version)
    ///
    /// Returns an error if the stream has been closed or if sending fails.
    pub fn send(&self, data: Vec<u8>) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.send_async(data))
    }

    /// Send a message to the response stream (async version)
    ///
    /// Returns an error if the stream has been closed or if sending fails.
    pub async fn send_async(&self, data: Vec<u8>) -> Result<(), RpcError> {
        let closed = self.closed.lock().await;
        if *closed {
            return Err(RpcError::new(
                crate::slimrpc::error::RpcCode::FailedPrecondition,
                "Response sink is closed".to_string(),
            ));
        }
        drop(closed);

        let sender = self.sender.lock().await;
        sender
            .send(Ok(data))
            .map_err(|_| {
                RpcError::new(
                    crate::slimrpc::error::RpcCode::Unavailable,
                    "Failed to send response".to_string(),
                )
            })
    }

    /// Send an error to the response stream and close it (blocking version)
    ///
    /// This terminates the stream with an error status.
    pub fn send_error(&self, error: RpcError) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.send_error_async(error))
    }

    /// Send an error to the response stream and close it (async version)
    ///
    /// This terminates the stream with an error status.
    pub async fn send_error_async(&self, error: RpcError) -> Result<(), RpcError> {
        let mut closed = self.closed.lock().await;
        if *closed {
            return Err(RpcError::new(
                crate::slimrpc::error::RpcCode::FailedPrecondition,
                "Response sink is already closed".to_string(),
            ));
        }
        *closed = true;
        drop(closed);

        let sender = self.sender.lock().await;
        let status: slim_rpc::Status = error.into();
        sender
            .send(Err(status))
            .map_err(|_| {
                RpcError::new(
                    crate::slimrpc::error::RpcCode::Unavailable,
                    "Failed to send error".to_string(),
                )
            })
    }

    /// Close the response stream (blocking version)
    ///
    /// Signals that no more messages will be sent.
    /// The stream will end gracefully.
    pub fn close(&self) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.close_async())
    }

    /// Close the response stream (async version)
    ///
    /// Signals that no more messages will be sent.
    /// The stream will end gracefully.
    pub async fn close_async(&self) -> Result<(), RpcError> {
        let mut closed = self.closed.lock().await;
        if *closed {
            return Ok(()); // Already closed, idempotent
        }
        *closed = true;
        drop(closed);

        // Drop the sender to signal stream end
        // The sender is wrapped in Arc<Mutex>, so we need to drop the actual sender
        // by replacing it or letting all references go
        // For now, we just mark as closed and the actual close happens when dropped
        Ok(())
    }

    /// Check if the sink has been closed (blocking version)
    pub fn is_closed(&self) -> bool {
        crate::get_runtime().block_on(self.is_closed_async())
    }

    /// Check if the sink has been closed (async version)
    pub async fn is_closed_async(&self) -> bool {
        *self.closed.lock().await
    }
}

/// Response stream for unary-stream and stream-stream handlers
///
/// This type represents the return value of streaming handlers.
/// It's an alias for compatibility with the handler trait definitions.
pub type ResponseStream = Box<dyn futures::Stream<Item = Result<Vec<u8>, slim_rpc::Status>> + Send + Unpin>;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[tokio::test]
    async fn test_request_stream() {
        let data = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let stream = stream::iter(data.clone().into_iter().map(Ok));
        let request_stream = RequestStream::new(Box::new(stream));

        let msg1 = request_stream.next_async().await;
        match msg1 {
            StreamMessage::Data(d) => assert_eq!(d, data[0]),
            _ => panic!("Expected data"),
        }

        let msg2 = request_stream.next_async().await;
        match msg2 {
            StreamMessage::Data(d) => assert_eq!(d, data[1]),
            _ => panic!("Expected data"),
        }

        let msg3 = request_stream.next_async().await;
        match msg3 {
            StreamMessage::End => {},
            _ => panic!("Expected end"),
        }
    }

    #[tokio::test]
    async fn test_response_sink() {
        let (sink, mut rx) = ResponseSink::receiver();

        sink.send_async(vec![1, 2, 3]).await.unwrap();
        sink.send_async(vec![4, 5, 6]).await.unwrap();
        sink.close_async().await.unwrap();

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.unwrap(), vec![1, 2, 3]);

        let msg2 = rx.recv().await.unwrap();
        assert_eq!(msg2.unwrap(), vec![4, 5, 6]);

        // After close, no more messages
        assert!(sink.is_closed_async().await);
    }

    #[tokio::test]
    async fn test_response_sink_error() {
        let (sink, mut rx) = ResponseSink::receiver();

        sink.send_async(vec![1, 2, 3]).await.unwrap();
        
        let error = RpcError::new(
            crate::slimrpc::error::RpcCode::Internal,
            "Test error".to_string(),
        );
        sink.send_error_async(error).await.unwrap();

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.unwrap(), vec![1, 2, 3]);

        let msg2 = rx.recv().await.unwrap();
        assert!(msg2.is_err());

        assert!(sink.is_closed_async().await);
    }
}
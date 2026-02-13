// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Stream wrapper types for SlimRPC UniFFI bindings
//!
//! Provides UniFFI-compatible wrappers for streaming operations.
//! Since UniFFI doesn't support Rust async streams, we provide synchronous
//! pull/push interfaces backed by async channels.

use futures::StreamExt;
use parking_lot::Mutex;
use tokio::sync::Mutex as TokioMutex;

use super::{Channel, RpcCode, RpcError};

/// Request stream reader
///
/// Allows pulling messages from a client request stream.
/// This wraps the underlying async stream and provides a blocking interface
/// suitable for UniFFI callback traits.
#[derive(uniffi::Object)]
pub struct RequestStream {
    /// Inner stream wrapped in a mutex for interior mutability
    inner: TokioMutex<super::RequestStream<Vec<u8>>>,
}

impl RequestStream {
    /// Create a new request stream wrapper
    pub fn new(stream: super::RequestStream<Vec<u8>>) -> Self {
        Self {
            inner: TokioMutex::new(stream),
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
            Some(Err(e)) => StreamMessage::Error(e),
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
    /// Channel sender for streaming responses (None when closed)
    sender: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, RpcError>>>>,
}

impl ResponseSink {
    /// Create a new response sink wrapper
    pub fn new(sender: tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, RpcError>>) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
        }
    }

    /// Get the receiver side of the channel
    pub fn receiver() -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>,
    ) {
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
        let sender = self.sender.lock();
        match sender.as_ref() {
            Some(s) => s.send(Ok(data)).map_err(|_| {
                RpcError::new(RpcCode::Unavailable, "Failed to send response".to_string())
            }),
            None => Err(RpcError::new(
                RpcCode::FailedPrecondition,
                "Response sink is closed".to_string(),
            )),
        }
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
        let mut sender_guard = self.sender.lock();
        match sender_guard.take() {
            Some(sender) => sender.send(Err(error)).map_err(|_| {
                RpcError::new(RpcCode::Unavailable, "Failed to send error".to_string())
            }),
            None => Err(RpcError::new(
                RpcCode::FailedPrecondition,
                "Response sink is already closed".to_string(),
            )),
        }
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
        let mut sender = self.sender.lock();
        sender.take(); // Drop the sender to signal stream end
        Ok(())
    }

    /// Check if the sink has been closed (blocking version)
    pub fn is_closed(&self) -> bool {
        crate::get_runtime().block_on(self.is_closed_async())
    }

    /// Check if the sink has been closed (async version)
    pub async fn is_closed_async(&self) -> bool {
        self.sender.lock().is_none()
    }
}

/// Response stream reader for unary-to-stream RPC calls
///
/// Allows pulling messages from a server response stream one at a time.
#[derive(uniffi::Object)]
pub struct ResponseStreamReader {
    /// Inner receiver channel for stream messages
    inner: TokioMutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>>,
}

impl ResponseStreamReader {
    /// Create a new response stream reader
    pub fn new(rx: tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>) -> Self {
        Self {
            inner: TokioMutex::new(rx),
        }
    }
}

#[uniffi::export]
impl ResponseStreamReader {
    /// Pull the next message from the response stream (blocking version)
    ///
    /// Returns a StreamMessage indicating the result
    pub fn next(&self) -> StreamMessage {
        crate::get_runtime().block_on(self.next_async())
    }

    /// Pull the next message from the response stream (async version)
    ///
    /// Returns a StreamMessage indicating the result
    pub async fn next_async(&self) -> StreamMessage {
        let mut rx = self.inner.lock().await;
        match rx.recv().await {
            Some(Ok(data)) => StreamMessage::Data(data),
            Some(Err(e)) => StreamMessage::Error(e),
            None => StreamMessage::End,
        }
    }
}

/// Request stream writer for stream-to-unary RPC calls
///
/// Allows sending multiple request messages and getting a final response.
#[derive(uniffi::Object)]
pub struct RequestStreamWriter {
    sender: TokioMutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
    response: TokioMutex<Option<tokio::task::JoinHandle<Result<Vec<u8>, RpcError>>>>,
}

impl RequestStreamWriter {
    pub fn new(
        channel: Channel,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let channel_clone = channel;
        let service_name_clone = service_name;
        let method_name_clone = method_name;

        // Spawn task to handle the stream_unary call
        let response_handle = crate::get_runtime().spawn(async move {
            use async_stream::stream;

            let stream = stream! {
                while let Some(data) = rx.recv().await {
                    yield data;
                }
            };

            channel_clone
                .stream_unary::<Vec<u8>, Vec<u8>>(
                    &service_name_clone,
                    &method_name_clone,
                    stream,
                    timeout,
                    metadata,
                )
                .await
        });

        Self {
            sender: TokioMutex::new(Some(tx)),
            response: TokioMutex::new(Some(response_handle)),
        }
    }
}

#[uniffi::export]
impl RequestStreamWriter {
    /// Send a request message to the stream (blocking version)
    pub fn send(&self, data: Vec<u8>) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.send_async(data))
    }

    /// Send a request message to the stream (async version)
    pub async fn send_async(&self, data: Vec<u8>) -> Result<(), RpcError> {
        let sender = self.sender.lock().await;
        if let Some(tx) = sender.as_ref() {
            tx.send(data)
                .map_err(|_| RpcError::new(RpcCode::Internal, "Stream closed".to_string()))
        } else {
            Err(RpcError::new(
                RpcCode::Internal,
                "Stream already finalized".to_string(),
            ))
        }
    }

    /// Finalize the stream and get the response (blocking version)
    pub fn finalize(&self) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.finalize_async())
    }

    /// Finalize the stream and get the response (async version)
    pub async fn finalize_async(&self) -> Result<Vec<u8>, RpcError> {
        // Drop the sender to signal end of stream
        {
            let mut sender = self.sender.lock().await;
            *sender = None;
        }

        // Wait for response
        let mut response_guard = self.response.lock().await;
        if let Some(handle) = response_guard.take() {
            handle
                .await
                .map_err(|e| RpcError::new(RpcCode::Internal, format!("Task failed: {}", e)))?
        } else {
            Err(RpcError::new(
                RpcCode::Internal,
                "Stream already finalized".to_string(),
            ))
        }
    }
}

/// Bidirectional stream handler for stream-to-stream RPC calls
///
/// Allows sending and receiving messages concurrently.
#[derive(uniffi::Object)]
pub struct BidiStreamHandler {
    sender: TokioMutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
    receiver: TokioMutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>>,
}

impl BidiStreamHandler {
    pub fn new(
        channel: Channel,
        service_name: String,
        method_name: String,
        timeout: Option<std::time::Duration>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Self {
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn task to handle the stream_stream call
        crate::get_runtime().spawn(async move {
            use async_stream::stream;

            let request_stream = stream! {
                while let Some(data) = req_rx.recv().await {
                    yield data;
                }
            };

            let response_stream = channel.stream_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request_stream,
                timeout,
                metadata,
            );

            futures::pin_mut!(response_stream);
            while let Some(item) = futures::StreamExt::next(&mut response_stream).await {
                if resp_tx.send(item).is_err() {
                    break;
                }
            }
        });

        Self {
            sender: TokioMutex::new(Some(req_tx)),
            receiver: TokioMutex::new(resp_rx),
        }
    }
}

#[uniffi::export]
impl BidiStreamHandler {
    /// Send a request message to the stream (blocking version)
    pub fn send(&self, data: Vec<u8>) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.send_async(data))
    }

    /// Send a request message to the stream (async version)
    pub async fn send_async(&self, data: Vec<u8>) -> Result<(), RpcError> {
        let sender = self.sender.lock().await;
        if let Some(tx) = sender.as_ref() {
            tx.send(data)
                .map_err(|_| RpcError::new(RpcCode::Internal, "Stream closed".to_string()))
        } else {
            Err(RpcError::new(
                RpcCode::Internal,
                "Stream already closed".to_string(),
            ))
        }
    }

    /// Close the request stream (no more messages will be sent)
    pub fn close_send(&self) -> Result<(), RpcError> {
        crate::get_runtime().block_on(self.close_send_async())
    }

    /// Close the request stream (async version)
    pub async fn close_send_async(&self) -> Result<(), RpcError> {
        let mut sender = self.sender.lock().await;
        *sender = None;
        Ok(())
    }

    /// Receive the next response message (blocking version)
    pub fn recv(&self) -> StreamMessage {
        crate::get_runtime().block_on(self.recv_async())
    }

    /// Receive the next response message (async version)
    pub async fn recv_async(&self) -> StreamMessage {
        let mut rx = self.receiver.lock().await;
        match rx.recv().await {
            Some(Ok(data)) => StreamMessage::Data(data),
            Some(Err(e)) => StreamMessage::Error(e),
            None => StreamMessage::End,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[tokio::test]
    async fn test_request_stream() {
        let data = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let stream = stream::iter(data.clone().into_iter().map(Ok));
        let request_stream = RequestStream::new(stream.boxed());

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
            StreamMessage::End => {}
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

        let error = RpcError::new(RpcCode::Internal, "Test error".to_string());
        sink.send_error_async(error).await.unwrap();

        let msg1 = rx.recv().await.unwrap();
        assert_eq!(msg1.unwrap(), vec![1, 2, 3]);

        let msg2 = rx.recv().await.unwrap();
        assert!(msg2.is_err());

        assert!(sink.is_closed_async().await);
    }
}

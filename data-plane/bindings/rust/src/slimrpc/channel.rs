// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC Channel bindings for UniFFI
//!
//! Provides a UniFFI-compatible wrapper around the core SlimRPC Channel type.

use std::sync::Arc;
use std::time::Duration;

use crate::errors::SlimError;
use crate::{get_runtime, App, Name};

use agntcy_slimrpc::Channel as CoreChannel;

use super::metadata::Metadata;
use super::rpc::{ResponseStream, StreamResult};

/// Client-side RPC channel for making RPC calls
///
/// A UniFFI-compatible wrapper around the core SlimRPC Channel that provides
/// methods for making RPC calls with different streaming patterns.
#[derive(uniffi::Object)]
pub struct RpcChannel {
    /// The underlying core channel
    channel: CoreChannel,
}

#[uniffi::export]
impl RpcChannel {
    /// Create a new RPC channel
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    #[uniffi::constructor]
    pub fn new(app: Arc<App>, remote: Arc<Name>) -> Arc<Self> {
        let slim_name = remote.as_slim_name();
        let core_channel = CoreChannel::new(app.inner_app().clone(), slim_name);

        Arc::new(Self {
            channel: core_channel,
        })
    }

    /// Create a new RPC channel with connection ID
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    /// * `connection_id` - Optional connection ID for session propagation
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: Arc<App>,
        remote: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Arc<Self> {
        let slim_name = remote.as_slim_name();
        let core_channel = CoreChannel::new_with_connection(app.inner_app().clone(), slim_name, connection_id);

        Arc::new(Self {
            channel: core_channel,
        })
    }

    /// Make a unary-unary RPC call (blocking)
    ///
    /// Sends a single request and receives a single response.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The serialized request bytes
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// The serialized response bytes
    pub fn unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Vec<u8>, SlimError> {
        let runtime = get_runtime();
        runtime.block_on(self.unary_async(service_name, method_name, request, timeout_secs, metadata))
    }

    /// Make a unary-unary RPC call (async)
    ///
    /// Sends a single request and receives a single response.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The serialized request bytes
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// The serialized response bytes
    pub async fn unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Vec<u8>, SlimError> {
        let timeout = timeout_secs.map(Duration::from_secs);
        let core_metadata = metadata.map(|m| m.to_core());

        let response = self
            .channel
            .unary(
                &service_name,
                &method_name,
                request,
                timeout,
                core_metadata,
            )
            .await
            .map_err(|e| SlimError::RpcError {
                message: format!("RPC call failed: {}", e),
            })?;

        Ok(response)
    }

    /// Get the remote service name
    pub fn remote(&self) -> Arc<Name> {
        Arc::new(Name::from_slim_name(self.channel.remote().clone()))
    }

    /// Get the connection ID
    pub fn connection_id(&self) -> Option<u64> {
        self.channel.connection_id()
    }

    /// Make a unary-stream RPC call (blocking)
    ///
    /// Sends a single request and receives multiple responses via a stream.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The serialized request bytes
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// A ResponseReceiver for reading the stream of responses
    pub fn unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Arc<dyn ResponseStream>, SlimError> {
        let runtime = get_runtime();
        runtime.block_on(self.unary_stream_async(service_name, method_name, request, timeout_secs, metadata))
    }

    /// Make a unary-stream RPC call (async)
    ///
    /// Sends a single request and receives multiple responses via a stream.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The serialized request bytes
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// A ResponseReceiver for reading the stream of responses
    pub async fn unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Arc<dyn ResponseStream>, SlimError> {
        use futures::StreamExt;
        
        let timeout = timeout_secs.map(Duration::from_secs);
        let core_metadata = metadata.map(|m| m.to_core());
        
        // Clone channel for use in spawned task
        let channel = self.channel.clone();

        // Spawn a task to collect the stream into a channel
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let stream = channel.unary_stream(
                &service_name,
                &method_name,
                request,
                timeout,
                core_metadata,
            );
            
            futures::pin_mut!(stream);
            while let Some(result) = stream.next().await {
                let send_result = match result {
                    Ok(data) => tx.send(Ok(data)),
                    Err(e) => tx.send(Err(e)),
                };
                if send_result.is_err() {
                    break; // Receiver dropped
                }
            }
        });

        // Create a receiver that reads from the channel
        Ok(Arc::new(ResponseReceiver {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }))
    }

    /// Make a stream-unary RPC call (blocking)
    ///
    /// Sends multiple requests via a sender and receives a single response.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request_sender` - A RequestSender for sending requests
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// The serialized response bytes
    pub fn stream_unary(
        &self,
        service_name: String,
        method_name: String,
        request_sender: Arc<dyn RequestSender>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Vec<u8>, SlimError> {
        let runtime = get_runtime();
        runtime.block_on(self.stream_unary_async(service_name, method_name, request_sender, timeout_secs, metadata))
    }

    /// Make a stream-unary RPC call (async)
    ///
    /// Sends multiple requests via a sender and receives a single response.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request_sender` - A RequestSender for sending requests
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// The serialized response bytes
    pub async fn stream_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request_sender: Arc<dyn RequestSender>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Vec<u8>, SlimError> {
        let timeout = timeout_secs.map(Duration::from_secs);
        let core_metadata = metadata.map(|m| m.to_core());

        // Create a stream from the sender
        let request_stream = RequestSenderStream { sender: request_sender };

        let response = self
            .channel
            .stream_unary(
                &service_name,
                &method_name,
                request_stream,
                timeout,
                core_metadata,
            )
            .await
            .map_err(|e| SlimError::RpcError {
                message: format!("RPC call failed: {}", e),
            })?;

        Ok(response)
    }

    /// Make a stream-stream RPC call (blocking)
    ///
    /// Sends multiple requests via a sender and receives multiple responses via a receiver.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request_sender` - A RequestSender for sending requests
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// A ResponseReceiver for reading the stream of responses
    pub fn stream_stream(
        &self,
        service_name: String,
        method_name: String,
        request_sender: Arc<dyn RequestSender>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Arc<dyn ResponseStream>, SlimError> {
        let runtime = get_runtime();
        runtime.block_on(self.stream_stream_async(service_name, method_name, request_sender, timeout_secs, metadata))
    }

    /// Make a stream-stream RPC call (async)
    ///
    /// Sends multiple requests via a sender and receives multiple responses via a receiver.
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request_sender` - A RequestSender for sending requests
    /// * `timeout_secs` - Optional timeout in seconds
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// A ResponseReceiver for reading the stream of responses
    pub async fn stream_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request_sender: Arc<dyn RequestSender>,
        timeout_secs: Option<u64>,
        metadata: Option<Arc<Metadata>>,
    ) -> Result<Arc<dyn ResponseStream>, SlimError> {
        use futures::StreamExt;
        
        let timeout = timeout_secs.map(Duration::from_secs);
        let core_metadata = metadata.map(|m| m.to_core());
        
        // Clone channel for use in spawned task
        let channel = self.channel.clone();

        // Create a stream from the sender
        let request_stream = RequestSenderStream { sender: request_sender };

        // Spawn a task to collect the stream into a channel
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let stream = channel.stream_stream(
                &service_name,
                &method_name,
                request_stream,
                timeout,
                core_metadata,
            );
            
            futures::pin_mut!(stream);
            while let Some(result) = stream.next().await {
                let send_result = match result {
                    Ok(data) => tx.send(Ok(data)),
                    Err(e) => tx.send(Err(e)),
                };
                if send_result.is_err() {
                    break; // Receiver dropped
                }
            }
        });

        // Create a receiver that reads from the channel
        Ok(Arc::new(ResponseReceiver {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }))
    }
}

// Internal helper methods
impl RpcChannel {
    /// Get the underlying core channel
    pub(crate) fn core_channel(&self) -> &CoreChannel {
        &self.channel
    }
}

// Note: Codec implementations for Vec<u8> are provided in the core slimrpc crate

/// Async trait for sending requests from client side
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait RequestSender: Send + Sync {
    /// Get the next request to send
    /// Returns End when done sending, Error on failure
    async fn next(&self) -> StreamResult;
}

/// Helper implementation of RequestSender that sends from a vector
/// 
/// This is useful for testing and simple use cases where you have
/// all requests ready upfront.
pub struct VectorRequestSender {
    requests: Arc<tokio::sync::Mutex<std::vec::IntoIter<Vec<u8>>>>,
}

impl VectorRequestSender {
    /// Create a new VectorRequestSender from a vector of requests
    pub fn new(requests: Vec<Vec<u8>>) -> Self {
        Self {
            requests: Arc::new(tokio::sync::Mutex::new(requests.into_iter())),
        }
    }
}

#[async_trait::async_trait]
impl RequestSender for VectorRequestSender {
    async fn next(&self) -> StreamResult {
        let mut iter = self.requests.lock().await;
        match iter.next() {
            Some(data) => StreamResult::Data { value: data },
            None => StreamResult::End,
        }
    }
}

/// Internal adapter to convert RequestSender into a Stream
struct RequestSenderStream {
    sender: Arc<dyn RequestSender>,
}

impl futures::Stream for RequestSenderStream {
    type Item = Vec<u8>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let sender = Arc::clone(&self.sender);
        let fut = async move { sender.next().await };
        
        futures::pin_mut!(fut);
        match fut.poll(cx) {
            std::task::Poll::Ready(result) => match result {
                StreamResult::Data { value } => std::task::Poll::Ready(Some(value)),
                StreamResult::End => std::task::Poll::Ready(None),
                StreamResult::Error { .. } => std::task::Poll::Ready(None),
            },
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// Internal response receiver that reads from a channel
struct ResponseReceiver {
    rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, agntcy_slimrpc::Status>>>>,
}

#[async_trait::async_trait]
impl ResponseStream for ResponseReceiver {
    async fn send(&self, _response: Vec<u8>) -> Result<(), super::error::Status> {
        Err(super::error::Status::new(
            super::error::Code::Unimplemented,
            "ResponseReceiver does not support send - use for reading only",
        ))
    }

    async fn close(&self) {
        // No-op for receiver
    }
}

// Implement RequestStream for ResponseReceiver to allow reading
#[async_trait::async_trait]
impl super::rpc::RequestStream for ResponseReceiver {
    async fn next(&self) -> StreamResult {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(Ok(data)) => StreamResult::Data { value: data },
            Some(Err(e)) => StreamResult::Error {
                status: super::error::Status::from_core(e),
            },
            None => StreamResult::End,
        }
    }
}



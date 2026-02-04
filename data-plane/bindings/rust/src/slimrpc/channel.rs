// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel wrapper for SlimRPC UniFFI bindings
//!
//! Provides a UniFFI-compatible client channel for making RPC calls.

use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
use futures::StreamExt;
use crate::slimrpc_core::Channel as CoreChannel;

use crate::slimrpc::error::RpcError;
use crate::slimrpc::types::StreamMessage;
use crate::{App, Name};

/// RPC Channel for making client calls
///
/// Wraps the core SlimRPC channel to provide UniFFI-compatible client operations.
/// A channel represents a connection to a remote service and can be used to make
/// multiple RPC calls.
#[derive(Clone, uniffi::Object)]
pub struct Channel {
    /// Wrapped core channel
    inner: CoreChannel,
}

#[uniffi::export]
impl Channel {
    /// Create a new RPC channel
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance
    /// * `remote` - The remote service name to connect to
    ///
    /// # Returns
    /// A new channel instance
    #[uniffi::constructor]
    pub fn new(app: Arc<App>, remote: Arc<Name>) -> Arc<Self> {
        Self::new_with_connection(app, remote, None)
    }

    /// Create a new RPC channel with optional connection ID
    ///
    /// The connection ID is used to set up routing before making RPC calls,
    /// enabling multi-hop RPC calls through specific connections.
    ///
    /// # Arguments
    /// * `app` - The SLIM application instance
    /// * `remote` - The remote service name to connect to
    /// * `connection_id` - Optional connection ID for routing setup
    ///
    /// # Returns
    /// A new channel instance
    #[uniffi::constructor]
    pub fn new_with_connection(
        app: Arc<App>,
        remote: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Arc<Self> {
        let slim_name = remote.as_ref().clone().into();
        let runtime = crate::get_runtime().handle().clone();

        // If connection_id is None, get the first connection from the service
        let resolved_connection_id = connection_id.or_else(|| {
            app.service()
                .get_all_connections()
                .first()
                .map(|conn| conn.id)
        });

        let inner = CoreChannel::new_with_connection(
            app.inner(),
            slim_name,
            resolved_connection_id,
            Some(runtime),
        );

        Arc::new(Self { inner })
    }

    /// Make a unary-to-unary RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// The response message bytes or an error
    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout,
        ))
    }

    /// Make a unary-to-unary RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// The response message bytes or an error
    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>, RpcError> {
        Ok(self
            .inner
            .unary(&service_name, &method_name, request, timeout, None)
            .await?)
    }

    /// Make a unary-to-stream RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A stream receiver for pulling response messages
    ///
    /// # Note
    /// This returns a ResponseStreamReader that can be used to pull messages
    /// one at a time from the response stream.
    pub fn call_unary_stream(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout,
        ))
    }

    /// Make a unary-to-stream RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A stream receiver for pulling response messages
    ///
    /// # Note
    /// This returns a ResponseStreamReader that can be used to pull messages
    /// one at a time from the response stream.
    pub async fn call_unary_stream_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        let channel = self.inner.clone();

        // Create a channel to transfer stream items
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn a task to consume the stream and forward to the channel
        crate::get_runtime().spawn(async move {
            let stream = channel.unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                None,
            );
            let mut stream = std::pin::pin!(stream);

            while let Some(item) = stream.next().await {
                if tx.send(item).is_err() {
                    // Receiver dropped, stop consuming
                    break;
                }
            }
        });

        Ok(Arc::new(ResponseStreamReader::new(rx)))
    }

    /// Make a stream-to-unary RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A RequestStreamWriter for sending request messages and getting the final response
    ///
    /// # Note
    /// This returns a RequestStreamWriter that can be used to send multiple request
    /// messages and then finalize to get the single response.
    pub fn call_stream_unary(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<Duration>,
    ) -> Arc<RequestStreamWriter> {
        Arc::new(RequestStreamWriter::new(
            self.inner.clone(),
            service_name,
            method_name,
            timeout,
        ))
    }

    /// Make a stream-to-stream RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `timeout` - Optional timeout duration
    ///
    /// # Returns
    /// A BidiStreamHandler for sending and receiving messages
    ///
    /// # Note
    /// This returns a BidiStreamHandler that can be used to send request messages
    /// and read response messages concurrently.
    pub fn call_stream_stream(
        &self,
        service_name: String,
        method_name: String,
        timeout: Option<Duration>,
    ) -> Arc<BidiStreamHandler> {
        Arc::new(BidiStreamHandler::new(
            self.inner.clone(),
            service_name,
            method_name,
            timeout,
        ))
    }
}

/// Response stream reader for unary-to-stream RPC calls
///
/// Allows pulling messages from a server response stream one at a time.
#[derive(uniffi::Object)]
pub struct ResponseStreamReader {
    /// Inner receiver channel for stream messages
    inner:
        tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, crate::slimrpc_core::Status>>>,
}

impl ResponseStreamReader {
    /// Create a new response stream reader
    pub(crate) fn new(
        rx: tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, crate::slimrpc_core::Status>>,
    ) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(rx),
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
            Some(Err(e)) => StreamMessage::Error(e.into()),
            None => StreamMessage::End,
        }
    }
}

/// Request stream writer for stream-to-unary RPC calls
///
/// Allows sending multiple request messages and then finalizing to get a single response.
#[derive(uniffi::Object)]
pub struct RequestStreamWriter {
    sender: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
    response:
        tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<Vec<u8>, crate::slimrpc_core::Status>>>>,
}

impl RequestStreamWriter {
    fn new(
        channel: CoreChannel,
        service_name: String,
        method_name: String,
        timeout: Option<Duration>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let channel_clone = channel.clone();
        let service_name_clone = service_name.clone();
        let method_name_clone = method_name.clone();

        // Spawn task to handle the stream_unary call
        let response_handle = crate::get_runtime().spawn(async move {
            let stream = stream! {
                let mut receiver = rx;
                while let Some(data) = receiver.recv().await {
                    yield data;
                }
            };

            channel_clone
                .stream_unary::<Vec<u8>, Vec<u8>>(
                    &service_name_clone,
                    &method_name_clone,
                    stream,
                    timeout,
                    None,
                )
                .await
        });

        Self {
            sender: tokio::sync::Mutex::new(Some(tx)),
            response: tokio::sync::Mutex::new(Some(response_handle)),
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
            tx.send(data).map_err(|_| {
                RpcError::new(
                    crate::slimrpc::error::RpcCode::Internal,
                    "Stream closed".to_string(),
                )
            })
        } else {
            Err(RpcError::new(
                crate::slimrpc::error::RpcCode::Internal,
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
            let result = handle.await.map_err(|e| {
                RpcError::new(
                    crate::slimrpc::error::RpcCode::Internal,
                    format!("Task failed: {}", e),
                )
            })?;
            Ok(result?)
        } else {
            Err(RpcError::new(
                crate::slimrpc::error::RpcCode::Internal,
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
    sender: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
    receiver:
        tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, crate::slimrpc_core::Status>>>,
}

impl BidiStreamHandler {
    fn new(
        channel: CoreChannel,
        service_name: String,
        method_name: String,
        timeout: Option<Duration>,
    ) -> Self {
        let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn task to handle the stream_stream call
        crate::get_runtime().spawn(async move {
            let request_stream = stream! {
                let mut receiver = req_rx;
                while let Some(data) = receiver.recv().await {
                    yield data;
                }
            };

            let response_stream = channel.stream_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request_stream,
                timeout,
                None,
            );

            let mut response_stream = std::pin::pin!(response_stream);
            while let Some(item) = response_stream.next().await {
                if resp_tx.send(item).is_err() {
                    break;
                }
            }
        });

        Self {
            sender: tokio::sync::Mutex::new(Some(req_tx)),
            receiver: tokio::sync::Mutex::new(resp_rx),
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
            tx.send(data).map_err(|_| {
                RpcError::new(
                    crate::slimrpc::error::RpcCode::Internal,
                    "Stream closed".to_string(),
                )
            })
        } else {
            Err(RpcError::new(
                crate::slimrpc::error::RpcCode::Internal,
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
            Some(Err(e)) => StreamMessage::Error(e.into()),
            None => StreamMessage::End,
        }
    }
}

#[cfg(test)]
mod tests {
    // Basic compilation tests
    #[test]
    fn test_channel_type_compiles() {
        // This test ensures the Channel type compiles correctly with UniFFI attributes
        // Actual functionality tests require a full SLIM app setup
    }
}

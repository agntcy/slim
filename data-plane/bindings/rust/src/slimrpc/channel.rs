// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel wrapper for SlimRPC UniFFI bindings
//!
//! Provides a UniFFI-compatible client channel for making RPC calls.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use slim_rpc::Channel as CoreChannel;
use tokio_stream::wrappers::UnboundedReceiverStream;

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
        let slim_name = remote.as_ref().clone().into();
        let inner = CoreChannel::new(app.inner().clone(), slim_name);

        Arc::new(Self { inner })
    }

    /// Make a unary-to-unary RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Returns
    /// The response message bytes or an error
    pub fn call_unary(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_ms: Option<u64>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_async(
            service_name,
            method_name,
            request,
            timeout_ms,
        ))
    }

    /// Make a unary-to-unary RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name (e.g., "MyService")
    /// * `method_name` - The method name (e.g., "GetUser")
    /// * `request` - The request message bytes
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Returns
    /// The response message bytes or an error
    pub async fn call_unary_async(
        &self,
        service_name: String,
        method_name: String,
        request: Vec<u8>,
        timeout_ms: Option<u64>,
    ) -> Result<Vec<u8>, RpcError> {
        let timeout = timeout_ms.map(Duration::from_millis);

        self.inner
            .unary(&service_name, &method_name, request, timeout, None)
            .await
            .map_err(|e| RpcError::new(e.code().into(), e.message().unwrap_or("").to_string()))
    }

    /// Make a unary-to-stream RPC call (blocking version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout_ms` - Optional timeout in milliseconds
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
        timeout_ms: Option<u64>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        crate::get_runtime().block_on(self.call_unary_stream_async(
            service_name,
            method_name,
            request,
            timeout_ms,
        ))
    }

    /// Make a unary-to-stream RPC call (async version)
    ///
    /// # Arguments
    /// * `service_name` - The service name
    /// * `method_name` - The method name
    /// * `request` - The request message bytes
    /// * `timeout_ms` - Optional timeout in milliseconds
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
        timeout_ms: Option<u64>,
    ) -> Result<Arc<ResponseStreamReader>, RpcError> {
        let timeout = timeout_ms.map(Duration::from_millis);
        let channel = self.inner.clone();

        // Create a channel to transfer stream items
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn a task to consume the stream and forward to the channel
        tokio::spawn(async move {
            let stream = channel.unary_stream::<Vec<u8>, Vec<u8>>(
                &service_name,
                &method_name,
                request,
                timeout,
                None,
            );
            let mut stream = Box::pin(stream);

            while let Some(item) = stream.next().await {
                if tx.send(item).is_err() {
                    // Receiver dropped, stop consuming
                    break;
                }
            }
        });

        // Convert receiver to a BoxStream
        let receiver_stream = UnboundedReceiverStream::new(rx);
        let boxed_stream = Box::pin(receiver_stream);

        Ok(Arc::new(ResponseStreamReader::new(boxed_stream)))
    }
}

impl Channel {
    /// Get reference to inner channel (for internal use)
    pub(crate) fn inner(&self) -> &CoreChannel {
        &self.inner
    }
}

/// Response stream reader for unary-to-stream RPC calls
///
/// Allows pulling messages from a server response stream one at a time.
#[derive(uniffi::Object)]
pub struct ResponseStreamReader {
    /// Inner stream wrapped for async access
    inner: Arc<
        tokio::sync::Mutex<futures::stream::BoxStream<'static, Result<Vec<u8>, slim_rpc::Status>>>,
    >,
}

impl ResponseStreamReader {
    /// Create a new response stream reader
    pub(crate) fn new(
        stream: futures::stream::BoxStream<'static, Result<Vec<u8>, slim_rpc::Status>>,
    ) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(stream)),
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
        let mut stream = self.inner.lock().await;
        match stream.next().await {
            Some(Ok(data)) => StreamMessage::Data(data),
            Some(Err(e)) => StreamMessage::Error(e.into()),
            None => StreamMessage::End,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic compilation tests
    #[test]
    fn test_channel_type_compiles() {
        // This test ensures the Channel type compiles correctly with UniFFI attributes
        // Actual functionality tests require a full SLIM app setup
    }
}

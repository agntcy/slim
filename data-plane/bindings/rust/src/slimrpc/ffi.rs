// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! FFI-compatible wrapper types for SlimRPC
//!
//! This module provides UniFFI-compatible types that wrap the core SlimRPC functionality
//! for use from other languages (Go, Python, Swift, Kotlin, etc.)

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::slimrpc::{Channel, Code, Context, Server, Status};
use crate::{App, Name};

/// FFI-compatible RPC channel for making client calls
#[derive(uniffi::Object)]
pub struct RpcChannel {
    inner: Channel,
}

#[uniffi::export]
impl RpcChannel {
    /// Create a new RPC channel
    #[uniffi::constructor]
    pub fn new(app: Arc<App>, remote: Arc<Name>) -> Self {
        let channel = Channel::new(app, remote.as_ref().clone());
        Self { inner: channel }
    }

    /// Create a new RPC channel with connection ID for session propagation
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `remote` - The base name of the remote service
    /// * `connection_id` - Optional connection ID to propagate sessions to next SLIM node
    #[uniffi::constructor]
    pub fn new_with_connection(app: Arc<App>, remote: Arc<Name>, connection_id: Option<u64>) -> Self {
        let channel = Channel::new_with_connection(app, remote.as_ref().clone(), connection_id);
        Self { inner: channel }
    }

    /// Make a unary RPC call (single request, single response)
    pub fn unary_call(
        &self,
        service_name: String,
        method_name: String,
        request_bytes: Vec<u8>,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<u8>, RpcError> {
        crate::config::get_runtime().block_on(async {
            self.unary_call_async(service_name, method_name, request_bytes, timeout_secs)
                .await
        })
    }

    /// Make an async unary RPC call
    pub async fn unary_call_async(
        &self,
        service_name: String,
        method_name: String,
        request_bytes: Vec<u8>,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<u8>, RpcError> {
        let timeout = timeout_secs.map(Duration::from_secs);

        tracing::info!("ok");

        // Use the codec-agnostic API with byte slices
        let response = self
            .inner
            .unary::<BytesMessage, BytesMessage>(
                &service_name,
                &method_name,
                BytesMessage(request_bytes),
                timeout,
                None,
            )
            .await
            .map_err(RpcError::from_status)?;

        Ok(response.0)
    }

    /// Make a unary request with streaming responses
    pub fn unary_stream_call(
        &self,
        service_name: String,
        method_name: String,
        request_bytes: Vec<u8>,
        timeout_secs: Option<u64>,
    ) -> Arc<RpcResponseStream> {
        let timeout = timeout_secs.map(Duration::from_secs);
        let channel = self.inner.clone();

        // Create response channel
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn task to handle streaming
        tokio::spawn(async move {
            let stream = channel.unary_stream::<BytesMessage, BytesMessage>(
                &service_name,
                &method_name,
                BytesMessage(request_bytes),
                timeout,
                None,
            );

            futures::pin_mut!(stream);

            while let Some(result) = stream.next().await {
                match result {
                    Ok(msg) => {
                        if tx.send(Ok(msg.0)).is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        let _ = tx.send(Err(RpcError::from_status(status)));
                        break;
                    }
                }
            }
        });

        Arc::new(RpcResponseStream {
            receiver: Mutex::new(rx),
        })
    }
}

/// FFI-compatible streaming response
#[derive(uniffi::Object)]
pub struct RpcResponseStream {
    receiver: Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, RpcError>>>,
}

#[uniffi::export]
impl RpcResponseStream {
    /// Get the next response from the stream (blocking)
    pub fn next(&self) -> Result<Option<Vec<u8>>, RpcError> {
        crate::config::get_runtime().block_on(async { self.next_async().await })
    }

    /// Get the next response from the stream (async)
    pub async fn next_async(&self) -> Result<Option<Vec<u8>>, RpcError> {
        let mut receiver = self.receiver.lock().await;
        match receiver.recv().await {
            Some(Ok(bytes)) => Ok(Some(bytes)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }
}

/// FFI-compatible RPC server
#[derive(uniffi::Object)]
pub struct RpcServer {
    inner: Server,
    handlers: Arc<tokio::sync::Mutex<HashMap<String, Arc<dyn RpcHandler>>>>,
}

#[uniffi::export]
impl RpcServer {
    /// Create a new RPC server
    #[uniffi::constructor]
    pub fn new(app: Arc<App>, base_name: Arc<Name>) -> Self {
        let server = Server::new(app, base_name.as_ref().clone());
        Self {
            inner: server,
            handlers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new RPC server with connection ID for subscription propagation
    ///
    /// # Arguments
    /// * `app` - The SLIM app to use for communication
    /// * `base_name` - The base name for this server
    /// * `connection_id` - Optional connection ID to propagate subscriptions to next SLIM node
    #[uniffi::constructor]
    pub fn new_with_connection(app: Arc<App>, base_name: Arc<Name>, connection_id: Option<u64>) -> Self {
        let server = Server::new_with_connection(app, base_name.as_ref().clone(), connection_id);
        Self {
            inner: server,
            handlers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a unary-unary handler
    pub fn register_unary_handler(
        &self,
        service_name: String,
        method_name: String,
        handler: Arc<dyn RpcHandler>,
    ) {
        let method_path = format!("{}/{}", service_name, method_name);
        crate::config::get_runtime().block_on(async {
            self.handlers.lock().await.insert(method_path.clone(), handler.clone());
        });

        let service_name_owned = service_name.clone();
        let method_name_owned = method_name.clone();
        let handler_clone = handler.clone();
        self.inner.registry().register_unary_unary(
            &service_name_owned,
            &method_name_owned,
            move |request: BytesMessage, ctx: Context| {
                let handler = handler_clone.clone();
                async move {
                    let request_bytes = request.0;
                    let rpc_ctx = RpcContext::from_context(ctx);
                    let response_bytes = handler
                        .handle(request_bytes, rpc_ctx)
                        .map_err(|e| e.to_status())?;
                    Ok(BytesMessage(response_bytes))
                }
            },
        );
    }

    /// Start serving requests (blocking)
    pub fn serve(&self) -> Result<(), RpcError> {
        crate::config::get_runtime()
            .block_on(async { self.serve_async().await })
    }

    /// Start serving requests (async)
    pub async fn serve_async(&self) -> Result<(), RpcError> {
        self.inner.serve().await.map_err(RpcError::from_status)
    }
}

/// Trait for RPC handlers (implemented in foreign languages)
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait RpcHandler: Send + Sync {
    /// Handle an RPC request
    fn handle(&self, request: Vec<u8>, context: RpcContext) -> Result<Vec<u8>, RpcError>;
}

/// FFI-compatible RPC context
#[derive(Clone, uniffi::Record)]
pub struct RpcContext {
    /// Session ID
    pub session_id: String,
    /// Source name components
    pub source_org: String,
    pub source_namespace: String,
    pub source_app: String,
    /// Destination name components
    pub dest_org: String,
    pub dest_namespace: String,
    pub dest_app: String,
    /// Request metadata
    pub metadata: HashMap<String, String>,
}

impl RpcContext {
    fn from_context(ctx: Context) -> Self {
        let session = ctx.session();
        let source = session.source();
        let dest = session.destination();
        let source_components = source.components();
        let dest_components = dest.components();

        Self {
            session_id: session.session_id().to_string(),
            source_org: source_components[0].clone(),
            source_namespace: source_components[1].clone(),
            source_app: source_components[2].clone(),
            dest_org: dest_components[0].clone(),
            dest_namespace: dest_components[1].clone(),
            dest_app: dest_components[2].clone(),
            metadata: ctx.metadata().as_map().clone(),
        }
    }
}

/// FFI-compatible RPC error
#[derive(Debug, Clone, uniffi::Error)]
pub enum RpcError {
    RpcFailed { code: i32, message: String },
}

impl RpcError {
    fn from_status(status: Status) -> Self {
        RpcError::RpcFailed {
            code: status.code().as_i32(),
            message: status.message().unwrap_or("").to_string(),
        }
    }

    fn to_status(&self) -> Status {
        match self {
            RpcError::RpcFailed { code, message } => {
                let code = Code::from_i32(*code).unwrap_or(Code::Unknown);
                Status::new(code, message.clone())
            }
        }
    }

    /// Create an internal error
    pub fn internal(message: String) -> Self {
        RpcError::RpcFailed {
            code: Code::Internal.as_i32(),
            message,
        }
    }

    /// Create a not found error
    pub fn not_found(message: String) -> Self {
        RpcError::RpcFailed {
            code: Code::NotFound.as_i32(),
            message,
        }
    }

    /// Create an invalid argument error
    pub fn invalid_argument(message: String) -> Self {
        RpcError::RpcFailed {
            code: Code::InvalidArgument.as_i32(),
            message,
        }
    }

    /// Create an unimplemented error
    pub fn unimplemented(message: String) -> Self {
        RpcError::RpcFailed {
            code: Code::Unimplemented.as_i32(),
            message,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::RpcFailed { code, message } => {
                write!(f, "RPC failed with code {}: {}", code, message)
            }
        }
    }
}

impl std::error::Error for RpcError {}

// Internal helper type for codec-agnostic message passing
struct BytesMessage(Vec<u8>);

impl crate::slimrpc::Encoder for BytesMessage {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status> {
        buf.extend_from_slice(&self.0);
        Ok(())
    }

    fn encoded_len(&self) -> usize {
        self.0.len()
    }
}

impl crate::slimrpc::Decoder for BytesMessage {
    fn decode(buf: &[u8]) -> Result<Self, Status> {
        Ok(BytesMessage(buf.to_vec()))
    }

    fn merge(&mut self, buf: &[u8]) -> Result<(), Status> {
        self.0.extend_from_slice(buf);
        Ok(())
    }
}

impl Default for BytesMessage {
    fn default() -> Self {
        BytesMessage(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_error_creation() {
        let err = RpcError::internal("test error".to_string());
        match err {
            RpcError::RpcFailed { code, message } => {
                assert_eq!(code, Code::Internal.as_i32());
                assert_eq!(message, "test error");
            }
        }
    }

    #[test]
    fn test_rpc_error_conversion() {
        let status = Status::not_found("resource not found");
        let err = RpcError::from_status(status);
        match err {
            RpcError::RpcFailed { code, ref message } => {
                assert_eq!(code, Code::NotFound.as_i32());
                assert_eq!(message, "resource not found");
            }
        }

        let status2 = err.to_status();
        assert_eq!(status2.code(), Code::NotFound);
    }
}

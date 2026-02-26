// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC session handling implementation
//!
//! This module contains the logic for handling individual RPC sessions,
//! including message sending/receiving, timeout handling, and stream processing.

use std::collections::HashMap;
use std::sync::Arc;

use async_stream::stream;
use futures::StreamExt;

use super::{
    Context, HandlerResponse, HandlerType, ItemStream, ReceivedMessage, RpcCode, RpcError,
    RpcHandler, STATUS_CODE_KEY, SessionRx, SessionTx, StreamRpcHandler,
};

/// Handler information retrieved from registry
pub enum HandlerInfo {
    Stream(StreamRpcHandler, HandlerType),
    Unary(RpcHandler, HandlerType),
}

/// RPC session handler for unary methods
///
/// This struct holds the session state and provides instance methods for handling
/// RPC sessions with unary input patterns (unary-unary, unary-stream).
pub struct RpcSession<'a> {
    session_tx: &'a SessionTx,
    session_rx: &'a mut SessionRx,
    method_path: String,
    ctx: Context,
}

impl<'a> RpcSession<'a> {
    /// Create a new RPC session handler for unary methods
    pub fn new(
        session_tx: &'a SessionTx,
        session_rx: &'a mut SessionRx,
        method_path: String,
    ) -> Self {
        let ctx = Context::from_session_tx(session_tx);
        Self {
            session_tx,
            session_rx,
            method_path,
            ctx,
        }
    }

    /// Handle a session for a specific method
    pub async fn handle(mut self, handler_info: HandlerInfo) -> Result<(), RpcError> {
        tracing::debug!(method_path = %self.method_path, "Processing RPC");

        // Check deadline before starting
        if self.ctx.is_deadline_exceeded() {
            return Err(RpcError::deadline_exceeded("Deadline exceeded"));
        }

        // Get deadline from context for the entire handler execution
        let deadline = tokio::time::Instant::now() + self.ctx.remaining_time();
        let sleep_fut = std::pin::pin!(tokio::time::sleep_until(deadline));

        // Handle based on handler type with deadline enforcement
        let result = tokio::select! {
            result = async {
                match handler_info {
                    HandlerInfo::Unary(handler, handler_type) => {
                        // Handle based on type (only unary-input handlers)
                        match handler_type {
                            HandlerType::UnaryUnary => self.handle_unary_unary(handler).await,
                            HandlerType::UnaryStream => self.handle_unary_stream(handler).await,
                            _ => Err(RpcError::internal(
                                "Invalid handler type for unary-input method",
                            )),
                        }
                    }
                    HandlerInfo::Stream(_, _) => {
                        // This shouldn't happen as stream methods are handled separately
                        Err(RpcError::internal(
                            "Stream methods should be handled separately",
                        ))
                    }
                }
            } => result,
            _ = sleep_fut => {
                tracing::debug!("RPC handler execution exceeded deadline");
                Err(RpcError::deadline_exceeded("Deadline exceeded during RPC execution"))
            }
        };

        if let Err(e) = &result {
            tracing::error!(method_path = %self.method_path, error = %e, "Error handling RPC");
        } else {
            tracing::debug!(method_path = %self.method_path, "RPC completed successfully");
        }

        result
    }

    /// Handle unary-unary RPC
    async fn handle_unary_unary(&mut self, handler: RpcHandler) -> Result<(), RpcError> {
        // Get the first message from the session
        let received = self.receive_first_message().await?;

        // Update context with message metadata to parse deadline
        self.ctx = self.ctx.clone().with_message_metadata(received.metadata);

        // Call handler and send response
        let response = handler(received.payload, self.ctx.clone()).await?;

        // Send response
        match response {
            HandlerResponse::Unary(response_bytes) => {
                self.send_message(response_bytes, RpcCode::Ok).await?;
            }
            _ => {
                return Err(RpcError::internal(
                    "Handler returned unexpected response type",
                ));
            }
        }

        Ok(())
    }

    /// Handle unary-stream RPC
    async fn handle_unary_stream(&mut self, handler: RpcHandler) -> Result<(), RpcError> {
        // Get the first message from the session
        let received = self.receive_first_message().await?;

        // Update context with message metadata to parse deadline
        self.ctx = self.ctx.clone().with_message_metadata(received.metadata);

        // Call handler and send streaming responses
        let response = handler(received.payload, self.ctx.clone()).await?;

        // Send streaming responses
        match response {
            HandlerResponse::Stream(stream) => {
                self.send_response_stream(stream).await?;
            }
            _ => {
                return Err(RpcError::internal(
                    "Handler returned unexpected response type",
                ));
            }
        }

        Ok(())
    }

    /// Send a message with status code
    async fn send_message(&self, payload: Vec<u8>, code: RpcCode) -> Result<(), RpcError> {
        let metadata = Self::create_status_metadata(code);
        let handle = self
            .session_tx
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| RpcError::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| RpcError::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker
    async fn send_end_of_stream(&self) -> Result<(), RpcError> {
        self.send_message(Vec::new(), RpcCode::Ok).await
    }

    /// Send all responses from a stream
    async fn send_response_stream(&self, mut stream: ItemStream) -> Result<(), RpcError> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    self.send_message(response_bytes, RpcCode::Ok).await?;
                }
                Err(e) => return Err(e),
            }
        }
        self.send_end_of_stream().await
    }

    /// Receive first message from session
    async fn receive_first_message(&mut self) -> Result<ReceivedMessage, RpcError> {
        self.session_rx.get_message(None).await.map_err(|e| {
            tracing::debug!(error = %e, "Session closed or error receiving message");
            RpcError::internal(format!("Failed to receive message: {}", e))
        })
    }

    /// Create status code metadata
    fn create_status_metadata(code: RpcCode) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        let code_i32: i32 = code.into();
        metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
        metadata
    }
}

/// RPC session handler for stream-based methods
///
/// This struct holds the session state for handling RPC sessions with
/// streaming input patterns (stream-unary, stream-stream).
pub struct StreamRpcSession<'a> {
    session_tx: &'a SessionTx,
    session_rx: SessionRx,
    method_path: String,
    ctx: Context,
}

impl<'a> StreamRpcSession<'a> {
    /// Create a new RPC session handler for stream-based methods
    pub fn new(session_tx: &'a SessionTx, session_rx: SessionRx, method_path: String) -> Self {
        let ctx = Context::from_session_tx(session_tx);
        Self {
            session_tx,
            session_rx,
            method_path,
            ctx,
        }
    }

    /// Handle a stream-based session
    pub async fn handle(
        self,
        stream_handler: StreamRpcHandler,
        handler_type: HandlerType,
    ) -> Result<(), RpcError> {
        let method_path = self.method_path.clone();

        tracing::debug!(%method_path, "Processing stream-based RPC");

        // Check deadline for stream-based methods
        if self.ctx.is_deadline_exceeded() {
            return Err(RpcError::deadline_exceeded("Deadline exceeded"));
        }

        // Get deadline from context for the entire handler execution
        let deadline = tokio::time::Instant::now() + self.ctx.remaining_time();
        let sleep_fut = std::pin::pin!(tokio::time::sleep_until(deadline));

        // Handle with deadline enforcement
        let result = tokio::select! {
            result = self.handle_stream_based_method(stream_handler, handler_type) => result,
            _ = sleep_fut => {
                tracing::debug!("Stream RPC handler execution exceeded deadline");
                Err(RpcError::deadline_exceeded("Deadline exceeded during RPC execution"))
            }
        };

        if let Err(e) = &result {
            tracing::error!(%method_path, error = %e, "Error handling RPC");
        } else {
            tracing::debug!(%method_path, "RPC completed successfully");
        }

        result
    }

    /// Handle stream-based methods (stream-unary and stream-stream)
    async fn handle_stream_based_method(
        self,
        handler: StreamRpcHandler,
        handler_type: HandlerType,
    ) -> Result<(), RpcError> {
        // Extract fields to consume self properly
        let session_tx = self.session_tx;
        let session_rx = self.session_rx;
        let ctx = self.ctx;

        // Create a stream of incoming requests
        // Wrap session_rx in Arc<Mutex> to allow capturing in stream! macro
        let session_rx = Arc::new(tokio::sync::Mutex::new(session_rx));
        let session_rx_for_stream = session_rx.clone();

        let request_stream = stream! {
            loop {
                let received_result = {
                    let mut rx = session_rx_for_stream.lock().await;
                    rx.get_message(None).await
                };

                tracing::debug!("Received message in stream-based method");

                let received = match received_result {
                    Ok(msg) => msg,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };

                // Check for end-of-stream marker
                let code = received.metadata.get(STATUS_CODE_KEY)
                    .and_then(|s| s.parse::<i32>().ok())
                    .and_then(|code| RpcCode::try_from(code).ok())
                    .unwrap_or(RpcCode::Ok);

                if code == RpcCode::Ok && received.payload.is_empty() {
                    break;
                }

                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    yield Err(RpcError::new(code, message));
                    break;
                }

                yield Ok(received.payload);
            }
        };

        // Box and pin the stream
        let boxed_stream = request_stream.boxed();

        // Call handler
        let handler_result = handler(boxed_stream, ctx.clone()).await?;

        // Send responses based on handler type
        match handler_type {
            HandlerType::StreamUnary => {
                // Send single response
                let response = match handler_result {
                    HandlerResponse::Unary(bytes) => bytes,
                    _ => {
                        return Err(RpcError::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                Self::send_message_static(session_tx, response, RpcCode::Ok).await?;
            }
            HandlerType::StreamStream => {
                // Send streaming responses
                let response_stream = match handler_result {
                    HandlerResponse::Stream(stream) => stream,
                    _ => {
                        return Err(RpcError::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                Self::send_response_stream_static(session_tx, response_stream).await?;
            }
            _ => {
                return Err(RpcError::internal(
                    "Invalid handler type for stream-based method",
                ));
            }
        }

        Ok(())
    }

    /// Send a message with status code (static version for after self is consumed)
    async fn send_message_static(
        session_tx: &SessionTx,
        payload: Vec<u8>,
        code: RpcCode,
    ) -> Result<(), RpcError> {
        let metadata = Self::create_status_metadata(code);
        let handle = session_tx
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| RpcError::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| RpcError::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker (static version for after self is consumed)
    async fn send_end_of_stream_static(session_tx: &SessionTx) -> Result<(), RpcError> {
        Self::send_message_static(session_tx, Vec::new(), RpcCode::Ok).await
    }

    /// Send all responses from a stream (static version for after self is consumed)
    async fn send_response_stream_static(
        session_tx: &SessionTx,
        mut stream: ItemStream,
    ) -> Result<(), RpcError> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    Self::send_message_static(session_tx, response_bytes, RpcCode::Ok).await?;
                }
                Err(e) => return Err(e),
            }
        }
        Self::send_end_of_stream_static(session_tx).await
    }

    /// Create status code metadata
    fn create_status_metadata(code: RpcCode) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        let code_i32: i32 = code.into();
        metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
        metadata
    }
}

/// Helper function to send an error response
pub async fn send_error(session: &SessionTx, error: RpcError) -> Result<(), RpcError> {
    let message = error.message().to_string();
    let metadata = create_status_metadata(error.code());
    let handle = session
        .publish(
            message.into_bytes(),
            Some("msg".to_string()),
            Some(metadata),
        )
        .await
        .map_err(|e| RpcError::internal(format!("Failed to send error: {}", e)))?;
    handle.await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to send error response");
        RpcError::internal(format!("Failed to complete error send: {}", e))
    })
}

/// Helper function to create status code metadata
fn create_status_metadata(code: RpcCode) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    let code_i32: i32 = code.into();
    metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
    metadata
}

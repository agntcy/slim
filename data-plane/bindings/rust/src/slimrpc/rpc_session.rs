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
    Code, Context, HandlerResponse, HandlerType, ItemStream, ReceivedMessage, RpcHandler,
    STATUS_CODE_KEY, SessionRx, SessionTx, Status, StreamRpcHandler,
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
    pub async fn handle(mut self, handler_info: HandlerInfo) -> Result<(), Status> {
        tracing::debug!(method_path = %self.method_path, "Processing RPC");

        // Handle based on handler type
        let result = match handler_info {
            HandlerInfo::Unary(handler, handler_type) => {
                // Check deadline
                if self.ctx.is_deadline_exceeded() {
                    Err(Status::deadline_exceeded("Deadline exceeded"))
                } else {
                    // Handle based on type (only unary-input handlers)
                    match handler_type {
                        HandlerType::UnaryUnary => self.handle_unary_unary(handler).await,
                        HandlerType::UnaryStream => self.handle_unary_stream(handler).await,
                        _ => Err(Status::internal(
                            "Invalid handler type for unary-input method",
                        )),
                    }
                }
            }
            HandlerInfo::Stream(_, _) => {
                // This shouldn't happen as stream methods are handled separately
                return Err(Status::internal(
                    "Stream methods should be handled separately",
                ));
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
    async fn handle_unary_unary(&mut self, handler: RpcHandler) -> Result<(), Status> {
        // Get the first message from the session
        let received = self.receive_first_message().await?;

        // Update context with message metadata to parse deadline
        self.ctx = self.ctx.clone().with_message_metadata(received.metadata);

        // Calculate deadline
        let timeout_duration = Self::get_timeout_duration(&self.ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;

        // Call handler and send response with deadline
        tokio::select! {
            result = async {
                // Call handler
                let response = handler(received.payload, self.ctx.clone()).await?;

                // Send response
                match response {
                    HandlerResponse::Unary(response_bytes) => {
                        self.send_message(response_bytes, Code::Ok).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
        }
    }

    /// Handle unary-stream RPC
    async fn handle_unary_stream(&mut self, handler: RpcHandler) -> Result<(), Status> {
        // Get the first message from the session
        let received = self.receive_first_message().await?;

        // Calculate deadline and create sleep future
        let timeout_duration = Self::get_timeout_duration(&self.ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let sleep_fut = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep_fut);

        // Call handler and send streaming responses with deadline
        tokio::select! {
            result = async {
                // Call handler
                let response = handler(received.payload, self.ctx.clone()).await?;

                // Send streaming responses
                match response {
                    HandlerResponse::Stream(stream) => {
                        self.send_response_stream(stream).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = &mut sleep_fut => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
        }
    }

    /// Send a message with status code
    async fn send_message(&self, payload: Vec<u8>, code: Code) -> Result<(), Status> {
        let metadata = Self::create_status_metadata(code);
        let handle = self
            .session_tx
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| Status::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| Status::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker
    async fn send_end_of_stream(&self) -> Result<(), Status> {
        self.send_message(Vec::new(), Code::Ok).await
    }

    /// Send all responses from a stream
    async fn send_response_stream(&self, mut stream: ItemStream) -> Result<(), Status> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    self.send_message(response_bytes, Code::Ok).await?;
                }
                Err(e) => return Err(e),
            }
        }
        self.send_end_of_stream().await
    }

    /// Receive first message from session
    async fn receive_first_message(&mut self) -> Result<ReceivedMessage, Status> {
        self.session_rx.get_message(None).await.map_err(|e| {
            tracing::debug!(error = %e, "Session closed or error receiving message");
            Status::internal(format!("Failed to receive message: {}", e))
        })
    }

    /// Create status code metadata
    fn create_status_metadata(code: Code) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        metadata.insert(STATUS_CODE_KEY.to_string(), code.as_i32().to_string());
        metadata
    }

    /// Get timeout duration from context
    ///
    /// Returns the remaining time from context, or a minimum grace period if deadline passed
    fn get_timeout_duration(ctx: &Context) -> std::time::Duration {
        let remaining = ctx.remaining_time();
        // If deadline has passed (remaining is ZERO), use a small grace period
        // Otherwise use the remaining time
        if remaining.is_zero() {
            std::time::Duration::from_secs(1)
        } else {
            remaining
        }
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
    ) -> Result<(), Status> {
        let method_path = self.method_path.clone();

        tracing::debug!(%method_path, "Processing stream-based RPC");

        // Check deadline for stream-based methods
        if self.ctx.is_deadline_exceeded() {
            return Err(Status::deadline_exceeded("Deadline exceeded"));
        }

        let result = self
            .handle_stream_based_method(stream_handler, handler_type)
            .await;

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
    ) -> Result<(), Status> {
        // Calculate deadline and create sleep future
        let timeout_duration = Self::get_timeout_duration(&self.ctx);
        let deadline = tokio::time::Instant::now() + timeout_duration;
        let sleep_fut = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep_fut);

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
                    .and_then(Code::from_i32)
                    .unwrap_or(Code::Ok);

                if code == Code::Ok && received.payload.is_empty() {
                    break;
                }

                if code != Code::Ok {
                    let message = String::from_utf8_lossy(&received.payload).to_string();
                    yield Err(Status::new(code, message));
                    break;
                }

                yield Ok(received.payload);
            }
        };

        // Box and pin the stream
        let boxed_stream = request_stream.boxed();

        // Call handler and send responses with deadline
        let result = tokio::select! {
            result = async {
                // Call handler
                let handler_result = handler(boxed_stream, ctx.clone()).await?;

                // Send responses based on handler type
                match handler_type {
                    HandlerType::StreamUnary => {
                        // Send single response
                        let response = match handler_result {
                            HandlerResponse::Unary(bytes) => bytes,
                            _ => {
                                return Err(Status::internal(
                                    "Handler returned unexpected response type",
                                ));
                            }
                        };

                        Self::send_message_static(session_tx, response, Code::Ok).await?;
                    }
                    HandlerType::StreamStream => {
                        // Send streaming responses
                        let response_stream = match handler_result {
                            HandlerResponse::Stream(stream) => stream,
                            _ => {
                                return Err(Status::internal(
                                    "Handler returned unexpected response type",
                                ));
                            }
                        };

                        Self::send_response_stream_static(session_tx, response_stream).await?;
                    }
                    _ => {
                        return Err(Status::internal(
                            "Invalid handler type for stream-based method",
                        ));
                    }
                }

                Ok(())
            } => result,
            _ = &mut sleep_fut => {
                tracing::debug!("Handler execution exceeded deadline");
                Err(Status::deadline_exceeded("Handler execution exceeded deadline"))
            }
        };

        result
    }

    /// Send a message with status code (static version for after self is consumed)
    async fn send_message_static(
        session_tx: &SessionTx,
        payload: Vec<u8>,
        code: Code,
    ) -> Result<(), Status> {
        let metadata = Self::create_status_metadata(code);
        let handle = session_tx
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| Status::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| Status::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker (static version for after self is consumed)
    async fn send_end_of_stream_static(session_tx: &SessionTx) -> Result<(), Status> {
        Self::send_message_static(session_tx, Vec::new(), Code::Ok).await
    }

    /// Send all responses from a stream (static version for after self is consumed)
    async fn send_response_stream_static(
        session_tx: &SessionTx,
        mut stream: ItemStream,
    ) -> Result<(), Status> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    Self::send_message_static(session_tx, response_bytes, Code::Ok).await?;
                }
                Err(e) => return Err(e),
            }
        }
        Self::send_end_of_stream_static(session_tx).await
    }

    /// Create status code metadata
    fn create_status_metadata(code: Code) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        metadata.insert(STATUS_CODE_KEY.to_string(), code.as_i32().to_string());
        metadata
    }

    /// Get timeout duration from context
    ///
    /// Returns the remaining time from context, or a minimum grace period if deadline passed
    fn get_timeout_duration(ctx: &Context) -> std::time::Duration {
        let remaining = ctx.remaining_time();
        // If deadline has passed (remaining is ZERO), use a small grace period
        // Otherwise use the remaining time
        if remaining.is_zero() {
            std::time::Duration::from_secs(1)
        } else {
            remaining
        }
    }
}

/// Helper function to send an error response
pub async fn send_error(session: &SessionTx, status: Status) -> Result<(), Status> {
    let message = status.message().unwrap_or("").to_string();
    let metadata = create_status_metadata(status.code());
    let handle = session
        .publish(
            message.into_bytes(),
            Some("msg".to_string()),
            Some(metadata),
        )
        .await
        .map_err(|e| Status::internal(format!("Failed to send error: {}", e)))?;
    handle.await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to send error response");
        Status::internal(format!("Failed to complete error send: {}", e))
    })
}

/// Helper function to create status code metadata
fn create_status_metadata(code: Code) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    metadata.insert(STATUS_CODE_KEY.to_string(), code.as_i32().to_string());
    metadata
}

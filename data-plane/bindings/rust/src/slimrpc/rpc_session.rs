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
use tokio::sync::mpsc;

use super::{
    Context, HandlerResponse, HandlerType, ItemStream, RPC_ID_KEY, ReceivedMessage, RpcCode,
    RpcError, RpcHandler, STATUS_CODE_KEY, SessionRx, SessionTx, StreamRpcHandler,
};

/// Handler information retrieved from registry
pub enum HandlerInfo {
    Stream(StreamRpcHandler, HandlerType),
    Unary(RpcHandler, HandlerType),
}

/// Abstraction over two kinds of stream-input receivers:
/// - `Session`: a raw `SessionRx` (used when the session is dedicated to one RPC call)
/// - `Channel`: an mpsc receiver fed by a per-session demultiplexer task
pub enum StreamMessageRx {
    Session(SessionRx),
    Channel(mpsc::UnboundedReceiver<ReceivedMessage>),
}

impl StreamMessageRx {
    async fn get_message(&mut self) -> Result<ReceivedMessage, RpcError> {
        match self {
            Self::Session(rx) => rx.get_message(None).await,
            Self::Channel(rx) => rx
                .recv()
                .await
                .ok_or_else(|| RpcError::internal("Stream channel closed")),
        }
    }
}

/// RPC session handler for unary methods
///
/// This struct holds the session state and provides instance methods for handling
/// RPC sessions with unary input patterns (unary-unary, unary-stream).
pub struct RpcSession<'a> {
    session_tx: &'a SessionTx,
    session_rx: Option<&'a mut SessionRx>,
    method_path: String,
    ctx: Context,
    /// Pre-read first message (from metadata-based dispatch). If set,
    /// `receive_first_message()` returns it instead of reading from `session_rx`.
    first_message: Option<ReceivedMessage>,
    /// RPC call identifier, included in every response message so the client
    /// can route the response to the correct waiting caller over a shared session.
    rpc_id: String,
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
            session_rx: Some(session_rx),
            method_path,
            ctx,
            first_message: None,
            rpc_id: String::new(),
        }
    }

    /// Create a new RPC session handler with a pre-read first message.
    ///
    /// Used by `Server::serve_internal` when the first message has already been
    /// read to extract the RPC method from its SLIM metadata. The stored message
    /// is returned by `receive_first_message()` rather than reading from `session_rx`.
    pub fn new_with_first_message(
        session_tx: &'a SessionTx,
        session_rx: &'a mut SessionRx,
        method_path: String,
        first_message: ReceivedMessage,
    ) -> Self {
        let ctx = Context::from_session_tx(session_tx);
        Self {
            session_tx,
            session_rx: Some(session_rx),
            method_path,
            ctx,
            first_message: Some(first_message),
            rpc_id: String::new(),
        }
    }

    /// Create a new RPC session handler with a pre-read first message and an RPC ID.
    ///
    /// Used by the per-session demultiplexer in `Server::serve_internal` when the
    /// session is shared across multiple concurrent RPC calls. The `rpc_id` is
    /// embedded in every response message so the client can route it back to the
    /// correct waiting caller.  No `SessionRx` is required because all subsequent
    /// messages are demultiplexed before reaching this handler.
    ///
    /// The context is initialised from the first message's metadata so that
    /// the deadline (which no longer travels in the session-level config but in
    /// the first message body) is correctly set before `handle()` checks it.
    pub fn new_with_rpc_id(
        session_tx: &'a SessionTx,
        method_path: String,
        first_message: ReceivedMessage,
        rpc_id: String,
    ) -> Self {
        let ctx = Context::from_session_tx(session_tx)
            .with_message_metadata(first_message.metadata.clone());
        Self {
            session_tx,
            session_rx: None,
            method_path,
            ctx,
            first_message: Some(first_message),
            rpc_id,
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
        let metadata = Self::create_status_metadata(code, &self.rpc_id);
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

    /// Receive first message from session.
    ///
    /// Returns the pre-read first message (stored in `self.first_message`) if
    /// one was supplied at construction time, otherwise reads the next message
    /// from the underlying session receiver.
    async fn receive_first_message(&mut self) -> Result<ReceivedMessage, RpcError> {
        if let Some(msg) = self.first_message.take() {
            return Ok(msg);
        }
        if let Some(ref mut rx) = self.session_rx {
            return rx.get_message(None).await.map_err(|e| {
                tracing::debug!(error = %e, "Session closed or error receiving message");
                RpcError::internal(format!("Failed to receive message: {}", e))
            });
        }
        Err(RpcError::internal(
            "No session receiver and no pre-read first message",
        ))
    }

    /// Create status code metadata, including the rpc_id when it is non-empty
    fn create_status_metadata(code: RpcCode, rpc_id: &str) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        let code_i32: i32 = code.into();
        metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
        if !rpc_id.is_empty() {
            metadata.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
        }
        metadata
    }
}

/// RPC session handler for stream-based methods
///
/// This struct holds the session state for handling RPC sessions with
/// streaming input patterns (stream-unary, stream-stream).
pub struct StreamRpcSession<'a> {
    session_tx: &'a SessionTx,
    session_rx: StreamMessageRx,
    method_path: String,
    ctx: Context,
    /// Pre-read first message (from metadata-based dispatch). If set,
    /// the request stream yields its payload before reading from `session_rx`.
    first_message: Option<ReceivedMessage>,
    /// RPC call identifier, included in every response message.
    rpc_id: String,
}

impl<'a> StreamRpcSession<'a> {
    /// Create a new RPC session handler for stream-based methods
    pub fn new(session_tx: &'a SessionTx, session_rx: SessionRx, method_path: String) -> Self {
        let ctx = Context::from_session_tx(session_tx);
        Self {
            session_tx,
            session_rx: StreamMessageRx::Session(session_rx),
            method_path,
            ctx,
            first_message: None,
            rpc_id: String::new(),
        }
    }

    /// Create a new stream RPC session handler with a pre-read first message.
    ///
    /// The first message's payload is prepended to the request stream so that
    /// the handler receives the complete sequence of client messages.
    pub fn new_with_first_message(
        session_tx: &'a SessionTx,
        session_rx: SessionRx,
        method_path: String,
        first_message: ReceivedMessage,
    ) -> Self {
        let ctx = Context::from_session_tx(session_tx);
        Self {
            session_tx,
            session_rx: StreamMessageRx::Session(session_rx),
            method_path,
            ctx,
            first_message: Some(first_message),
            rpc_id: String::new(),
        }
    }

    /// Create a new stream RPC session handler with an RPC ID and an mpsc channel receiver.
    ///
    /// Used by the per-session demultiplexer when the session is shared across multiple
    /// concurrent RPC calls. The `channel_rx` receives messages routed by the demux task.
    /// The `rpc_id` is embedded in every response so the client can route it back.
    ///
    /// The context is initialised from the first message's metadata so that
    /// the deadline travels correctly (it is no longer in the session config).
    pub fn new_with_rpc_id(
        session_tx: &'a SessionTx,
        channel_rx: mpsc::UnboundedReceiver<ReceivedMessage>,
        method_path: String,
        first_message: ReceivedMessage,
        rpc_id: String,
    ) -> Self {
        let ctx = Context::from_session_tx(session_tx)
            .with_message_metadata(first_message.metadata.clone());
        Self {
            session_tx,
            session_rx: StreamMessageRx::Channel(channel_rx),
            method_path,
            ctx,
            first_message: Some(first_message),
            rpc_id,
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
        let first_message = self.first_message;
        let rpc_id = self.rpc_id;

        // Create a stream of incoming requests
        // Wrap session_rx in Arc<Mutex> to allow capturing in stream! macro
        let session_rx = Arc::new(tokio::sync::Mutex::new(session_rx));
        let session_rx_for_stream = session_rx.clone();

        let request_stream = stream! {
            // Handle the pre-read first message (consumed for metadata-based dispatch).
            // It may be a normal application message, an error, or an EOS marker
            // (the latter occurs when the client's request stream was empty and the
            // method metadata was attached to the EOS rather than a data frame).
            if let Some(first_msg) = first_message {
                let code = first_msg.metadata.get(STATUS_CODE_KEY)
                    .and_then(|s| s.parse::<i32>().ok())
                    .and_then(|code| RpcCode::try_from(code).ok())
                    .unwrap_or(RpcCode::Ok);

                if code == RpcCode::Ok && first_msg.payload.is_empty() {
                    // EOS marker — the request stream is empty, nothing to yield.
                    return;
                }
                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&first_msg.payload).to_string();
                    yield Err(RpcError::new(code, message));
                    return;
                }
                yield Ok(first_msg.payload);
            }

            loop {
                let received_result = {
                    let mut rx = session_rx_for_stream.lock().await;
                    rx.get_message().await
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

                Self::send_message_static(session_tx, response, RpcCode::Ok, &rpc_id).await?;
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

                Self::send_response_stream_static(session_tx, response_stream, &rpc_id).await?;
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
        rpc_id: &str,
    ) -> Result<(), RpcError> {
        let metadata = Self::create_status_metadata(code, rpc_id);
        let handle = session_tx
            .publish(payload, Some("msg".to_string()), Some(metadata))
            .await
            .map_err(|e| RpcError::internal(format!("Failed to send message: {}", e)))?;
        handle
            .await
            .map_err(|e| RpcError::internal(format!("Failed to complete message send: {}", e)))
    }

    /// Send end-of-stream marker (static version for after self is consumed)
    async fn send_end_of_stream_static(
        session_tx: &SessionTx,
        rpc_id: &str,
    ) -> Result<(), RpcError> {
        Self::send_message_static(session_tx, Vec::new(), RpcCode::Ok, rpc_id).await
    }

    /// Send all responses from a stream (static version for after self is consumed)
    async fn send_response_stream_static(
        session_tx: &SessionTx,
        mut stream: ItemStream,
        rpc_id: &str,
    ) -> Result<(), RpcError> {
        while let Some(result) = stream.next().await {
            match result {
                Ok(response_bytes) => {
                    Self::send_message_static(session_tx, response_bytes, RpcCode::Ok, rpc_id)
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        Self::send_end_of_stream_static(session_tx, rpc_id).await
    }

    /// Create status code metadata, including the rpc_id when it is non-empty
    fn create_status_metadata(code: RpcCode, rpc_id: &str) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        let code_i32: i32 = code.into();
        metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
        if !rpc_id.is_empty() {
            metadata.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
        }
        metadata
    }
}

/// Send a session-level error response (no rpc_id — used before dispatch)
pub async fn send_error(session: &SessionTx, error: RpcError) -> Result<(), RpcError> {
    send_error_for_rpc(session, error, "").await
}

/// Send an RPC-level error response including the rpc_id so the client can
/// route it back to the correct waiting caller over the shared session.
pub async fn send_error_for_rpc(
    session: &SessionTx,
    error: RpcError,
    rpc_id: &str,
) -> Result<(), RpcError> {
    let message = error.message().to_string();
    let mut metadata = create_status_metadata(error.code());
    if !rpc_id.is_empty() {
        metadata.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    }
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

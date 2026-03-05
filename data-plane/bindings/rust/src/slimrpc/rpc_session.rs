// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC session handling implementation
//!
//! This module contains the logic for handling individual RPC sessions,
//! including message sending/receiving, timeout handling, and stream processing.

use std::collections::HashMap;

use async_stream::stream;
use futures::StreamExt;
use tokio::sync::mpsc;

use slim_datapath::messages::Name;

use super::{
    Context, HandlerResponse, HandlerType, ItemStream, RPC_ID_KEY, ReceivedMessage, RpcCode,
    RpcError, RpcHandler, STATUS_CODE_KEY, SessionTx, StreamRpcHandler,
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
    method_path: String,
    ctx: Context,
    /// First message read from the wire, carrying the request payload and metadata.
    first_message: ReceivedMessage,
    /// RPC call identifier, included in every response message so the client
    /// can route the response to the correct waiting caller over a shared session.
    rpc_id: String,
}

impl<'a> RpcSession<'a> {
    /// Create a new RPC session handler with a pre-read first message and an RPC ID.
    ///
    /// Used by the per-session demultiplexer in `Server::serve_internal` when the
    /// session is shared across multiple concurrent RPC calls. The `rpc_id` is
    /// embedded in every response message so the client can route it back to the
    /// correct waiting caller.  No `SessionRx` is required because all subsequent
    /// messages are demultiplexed before reaching this handler.
    ///
    /// The context is initialised from the first message's metadata.
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
            method_path,
            ctx,
            first_message,
            rpc_id,
        }
    }

    /// Handle a session for a specific method
    pub async fn handle(self, handler_info: HandlerInfo) -> Result<(), RpcError> {
        tracing::debug!(method_path = %self.method_path, "Processing RPC");

        if self.ctx.is_deadline_exceeded() {
            return Err(RpcError::deadline_exceeded("Deadline exceeded"));
        }

        let method_path = self.method_path.clone();
        let deadline = tokio::time::Instant::now() + self.ctx.remaining_time();
        let sleep_fut = std::pin::pin!(tokio::time::sleep_until(deadline));

        let result = tokio::select! {
            result = async move {
                match handler_info {
                    HandlerInfo::Unary(handler, handler_type) => match handler_type {
                        HandlerType::UnaryUnary => {
                            Self::handle_unary_unary(self.session_tx, self.ctx, self.first_message, handler, &self.rpc_id).await
                        }
                        HandlerType::UnaryStream => {
                            Self::handle_unary_stream(self.session_tx, self.ctx, self.first_message, handler, &self.rpc_id).await
                        }
                        _ => Err(RpcError::internal("Invalid handler type for unary-input method")),
                    },
                    HandlerInfo::Stream(_, _) => {
                        Err(RpcError::internal("Stream methods should be handled separately"))
                    }
                }
            } => result,
            _ = sleep_fut => {
                tracing::debug!("RPC handler execution exceeded deadline");
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

    async fn handle_unary_unary(
        session_tx: &SessionTx,
        ctx: Context,
        first_message: ReceivedMessage,
        handler: RpcHandler,
        rpc_id: &str,
    ) -> Result<(), RpcError> {
        let source = first_message.source.clone();
        let ctx = ctx.with_message_metadata(first_message.metadata);
        let response = handler(first_message.payload, ctx).await?;
        match response {
            HandlerResponse::Unary(response_bytes) => {
                send_message(session_tx, &source, response_bytes, RpcCode::Ok, rpc_id).await?;
                // Send EOS so GROUP/multicast callers can count per-member stream ends.
                // P2P callers drop the stream after reading the single item, so the EOS
                // is harmlessly discarded if the dispatcher has already been unregistered.
                send_message(session_tx, &source, Vec::new(), RpcCode::Ok, rpc_id).await
            }
            _ => Err(RpcError::internal(
                "Handler returned unexpected response type",
            )),
        }
    }

    async fn handle_unary_stream(
        session_tx: &SessionTx,
        ctx: Context,
        first_message: ReceivedMessage,
        handler: RpcHandler,
        rpc_id: &str,
    ) -> Result<(), RpcError> {
        let source = first_message.source.clone();
        let ctx = ctx.with_message_metadata(first_message.metadata);
        let response = handler(first_message.payload, ctx).await?;
        match response {
            HandlerResponse::Stream(stream) => {
                send_response_stream(session_tx, stream, rpc_id, &source).await
            }
            _ => Err(RpcError::internal(
                "Handler returned unexpected response type",
            )),
        }
    }
}

/// RPC session handler for stream-based methods
///
/// This struct holds the session state for handling RPC sessions with
/// streaming input patterns (stream-unary, stream-stream).
pub struct StreamRpcSession<'a> {
    session_tx: &'a SessionTx,
    session_rx: mpsc::UnboundedReceiver<ReceivedMessage>,
    method_path: String,
    ctx: Context,
    /// First message read from the wire; its payload is prepended to the request
    /// stream so the handler receives the complete sequence of client messages.
    first_message: ReceivedMessage,
    /// RPC call identifier, included in every response message.
    rpc_id: String,
}

impl<'a> StreamRpcSession<'a> {
    /// Create a new stream RPC session handler with an RPC ID and an mpsc channel receiver.
    ///
    /// Used by the per-session demultiplexer when the session is shared across multiple
    /// concurrent RPC calls. The `channel_rx` receives messages routed by the demux task.
    /// The `rpc_id` is embedded in every response so the client can route it back.
    ///
    /// The context is initialised from the first message's metadata
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
            session_rx: channel_rx,
            method_path,
            ctx,
            first_message,
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
        let mut session_rx = self.session_rx;
        let ctx = self.ctx;
        let first_message = self.first_message;
        let source = first_message.source.clone();
        let rpc_id = self.rpc_id;

        let request_stream = stream! {
            // Handle the first message. It may be a normal application message,
            // an error, or an EOS marker (the latter occurs when the client's
            // request stream was empty and the method metadata was attached to
            // the EOS rather than a data frame).
            {
                let code = first_message.metadata.get(STATUS_CODE_KEY)
                    .and_then(|s| s.parse::<i32>().ok())
                    .and_then(|code| RpcCode::try_from(code).ok())
                    .unwrap_or(RpcCode::Ok);

                if code == RpcCode::Ok && first_message.payload.is_empty() {
                    // EOS marker — the request stream is empty, nothing to yield.
                    return;
                }
                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&first_message.payload).to_string();
                    yield Err(RpcError::new(code, message));
                    return;
                }
                yield Ok(first_message.payload);
            }

            loop {
                tracing::debug!("Received message in stream-based method");

                let received = match session_rx.recv().await {
                    Some(msg) => msg,
                    None => {
                        yield Err(RpcError::internal("Stream channel closed"));
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
                // Send single response followed by EOS, consistent with the streaming
                // path so GROUP/multicast callers can count per-member stream ends.
                let response = match handler_result {
                    HandlerResponse::Unary(bytes) => bytes,
                    _ => {
                        return Err(RpcError::internal(
                            "Handler returned unexpected response type",
                        ));
                    }
                };

                send_message(session_tx, &source, response, RpcCode::Ok, &rpc_id).await?;
                send_message(session_tx, &source, Vec::new(), RpcCode::Ok, &rpc_id).await?;
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

                send_response_stream(session_tx, response_stream, &rpc_id, &source).await?;
            }
            _ => {
                return Err(RpcError::internal(
                    "Invalid handler type for stream-based method",
                ));
            }
        }

        Ok(())
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
    let metadata = create_status_metadata(error.code(), rpc_id);
    let handle = session
        .publish(
            &session.destination(),
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

fn create_status_metadata(code: RpcCode, rpc_id: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    let code_i32: i32 = code.into();
    metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
    if !rpc_id.is_empty() {
        metadata.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    }
    metadata
}

async fn send_message(
    session_tx: &SessionTx,
    target: &Name,
    payload: Vec<u8>,
    code: RpcCode,
    rpc_id: &str,
) -> Result<(), RpcError> {
    let handle = session_tx
        .publish(
            target,
            payload,
            Some("msg".to_string()),
            Some(create_status_metadata(code, rpc_id)),
        )
        .await
        .map_err(|e| RpcError::internal(format!("Failed to send message: {}", e)))?;
    handle
        .await
        .map_err(|e| RpcError::internal(format!("Failed to complete message send: {}", e)))
}

async fn send_response_stream(
    session_tx: &SessionTx,
    mut stream: ItemStream,
    rpc_id: &str,
    target: &Name,
) -> Result<(), RpcError> {
    while let Some(result) = stream.next().await {
        match result {
            Ok(response_bytes) => {
                send_message(session_tx, target, response_bytes, RpcCode::Ok, rpc_id).await?;
            }
            Err(e) => return Err(e),
        }
    }
    send_message(session_tx, target, Vec::new(), RpcCode::Ok, rpc_id).await
}

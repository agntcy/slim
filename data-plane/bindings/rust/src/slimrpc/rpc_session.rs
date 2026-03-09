// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC session handling implementation
//!
//! This module contains the logic for handling individual RPC sessions,
//! including message sending/receiving, timeout handling, and stream processing.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use slim_session::CompletionHandle;
use tokio::sync::mpsc;

use slim_datapath::messages::Name;

use super::{
    Context, RPC_ID_KEY, ReceivedMessage, RpcCode, RpcError, RpcHandler, STATUS_CODE_KEY,
    SessionTx, StreamRpcHandler, StreamSource,
};

/// Handler information retrieved from registry
pub enum HandlerInfo {
    Stream(StreamRpcHandler),
    Unary(RpcHandler),
}

/// RPC session handler for all four interaction patterns.
///
/// For stream-input methods (stream-unary, stream-stream), construct with
/// [`RpcSession::new_stream`]. For unary-input methods (unary-unary,
/// unary-stream), construct with [`RpcSession::new_unary`].
pub struct RpcSession<'a> {
    session_tx: &'a SessionTx,
    /// Present for stream-input handlers; absent for unary-input handlers.
    session_rx: Option<mpsc::UnboundedReceiver<ReceivedMessage>>,
    method_path: &'a str,
    /// First message read from the wire, carrying the request payload and metadata.
    first_message: ReceivedMessage,
}

impl<'a> RpcSession<'a> {
    /// Create a session for a unary-input RPC (unary-unary, unary-stream).
    pub fn new_unary(
        session_tx: &'a SessionTx,
        method_path: &'a str,
        first_message: ReceivedMessage,
    ) -> Self {
        Self {
            session_tx,
            session_rx: None,
            method_path,
            first_message,
        }
    }

    /// Create a session for a stream-input RPC (stream-unary, stream-stream).
    pub fn new_stream(
        session_tx: &'a SessionTx,
        channel_rx: mpsc::UnboundedReceiver<ReceivedMessage>,
        method_path: &'a str,
        first_message: ReceivedMessage,
    ) -> Self {
        Self {
            session_tx,
            session_rx: Some(channel_rx),
            method_path,
            first_message,
        }
    }

    /// Handle the session, dispatching to the appropriate interaction pattern.
    pub async fn handle(self, handler_info: HandlerInfo, rpc_id: Arc<str>) -> Result<(), RpcError> {
        let Self {
            session_tx,
            session_rx,
            method_path,
            first_message,
        } = self;

        tracing::debug!(%method_path, "Processing RPC");

        // Pre-compute first-message status before moving fields into ctx / stream.
        let first_is_eos = first_message.is_eos();
        let first_code = RpcCode::from_metadata_str(
            first_message
                .metadata
                .get(STATUS_CODE_KEY)
                .map(String::as_str),
        );
        let ReceivedMessage {
            metadata,
            payload,
            source,
        } = first_message;
        let ctx = Context::from_session_tx(session_tx).with_message_metadata(metadata);

        if ctx.is_deadline_exceeded() {
            return Err(RpcError::deadline_exceeded("Deadline exceeded"));
        }

        let deadline = tokio::time::Instant::now() + ctx.remaining_time();

        let result = tokio::select! {
            result = async move {
                match &handler_info {
                    HandlerInfo::Unary(handler) => {
                        handler(payload, ctx, session_tx.clone(), source, rpc_id).await
                    }
                    HandlerInfo::Stream(handler) => {
                        let stream_source = StreamSource { session_rx, payload, first_is_eos, first_code };
                        handler(stream_source, ctx, session_tx.clone(), source, rpc_id).await
                    }
                }
            } => result,
            _ = tokio::time::sleep_until(deadline) => {
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
            session.destination(),
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

/// Send an end-of-stream marker to `target`.
///
/// An EOS is an empty-payload message with `slimrpc-code = Ok`. Every server
/// handler must send one after its final response so GROUP/multicast callers
/// can count per-member stream ends. P2P callers discard it harmlessly.
///
/// `extra` is merged into the EOS metadata after the status code. Pass
/// `None` for the common case where no additional metadata is needed.
/// When the EOS is also the first message (empty request stream), pass
/// service + method so the server can dispatch without a preceding data frame.
pub async fn send_eos(
    session_tx: &SessionTx,
    target: &Name,
    rpc_id: &str,
    extra: Option<HashMap<String, String>>,
) -> Result<CompletionHandle, RpcError> {
    let mut metadata = create_status_metadata(RpcCode::Ok, rpc_id);
    if let Some(extra) = extra {
        metadata.extend(extra);
    }
    session_tx
        .publish(target, Vec::new(), Some("msg".to_string()), Some(metadata))
        .await
        .map_err(|e| RpcError::internal(format!("Failed to send EOS: {}", e)))
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

pub async fn send_response_stream<S>(
    session_tx: &SessionTx,
    stream: S,
    rpc_id: &str,
    target: &Name,
) -> Result<(), RpcError>
where
    S: futures::Stream<Item = Result<Vec<u8>, RpcError>>,
{
    futures::pin_mut!(stream);

    // Publish every response message without awaiting its CompletionHandle,
    // collecting all handles so we can wait for them in a single batch once
    // the stream is exhausted.  This avoids a per-message round-trip delay
    // caused by the session layer's reliable-delivery acknowledgement protocol.
    let mut handles: Vec<CompletionHandle> = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(response_bytes) => {
                let handle = session_tx
                    .publish(
                        target,
                        response_bytes,
                        Some("msg".to_string()),
                        Some(create_status_metadata(RpcCode::Ok, rpc_id)),
                    )
                    .await
                    .map_err(|e| RpcError::internal(format!("Failed to send response: {}", e)))?;
                handles.push(handle);
            }
            Err(e) => return Err(e),
        }
    }

    // Queue the EOS marker and collect its handle too.
    handles.push(send_eos(session_tx, target, rpc_id, None).await?);

    // Wait for all in-flight sends to be acknowledged by the session layer.
    futures::future::try_join_all(handles)
        .await
        .map_err(|e| RpcError::internal(format!("Failed to complete sending: {}", e)))?;

    Ok(())
}

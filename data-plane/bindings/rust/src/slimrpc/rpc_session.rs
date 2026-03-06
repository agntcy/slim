// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC session handling implementation
//!
//! This module contains the logic for handling individual RPC sessions,
//! including message sending/receiving, timeout handling, and stream processing.

use std::collections::HashMap;
use std::pin::Pin;
use std::task::Poll;

use futures::StreamExt;
use tokio::sync::mpsc;

use slim_datapath::messages::Name;

use super::{
    Context, HandlerResponse, RPC_ID_KEY, ReceivedMessage, RpcCode, RpcError, RpcHandler,
    STATUS_CODE_KEY, SessionTx, StreamRpcHandler,
};

/// Raw byte stream for a stream-input RPC call.
///
/// Implements [`Stream`] directly using `poll_recv` — no heap allocation.
/// All fields are [`Unpin`], so this type is `Unpin` too.
///
/// Yielded values:
/// - `Ok(payload)` — normal message bytes
/// - `Err(e)` — protocol error (bad status code, channel closed)
///
/// The stream ends (returns `None`) on EOS or after yielding an error.
pub struct RawStream {
    session_rx: Option<mpsc::UnboundedReceiver<ReceivedMessage>>,
    payload: Vec<u8>,
    first_is_eos: bool,
    first_code: RpcCode,
    /// `true` once the first message has been yielded or skipped.
    first_done: bool,
    /// `true` once the stream has permanently ended (EOS, error, or channel close).
    done: bool,
}

impl futures::Stream for RawStream {
    type Item = Result<Vec<u8>, RpcError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }

        // ── First message ────────────────────────────────────────────────────
        if !self.first_done {
            self.first_done = true;

            if self.first_is_eos {
                self.done = true;
                return Poll::Ready(None);
            }

            let first_code = self.first_code;
            if first_code != RpcCode::Ok {
                let payload = std::mem::take(&mut self.payload);
                self.done = true;
                return Poll::Ready(Some(Err(RpcError::new(
                    first_code,
                    String::from_utf8_lossy(&payload).into_owned(),
                ))));
            }

            let payload = std::mem::take(&mut self.payload);
            return Poll::Ready(Some(Ok(payload)));
        }

        // ── Continuation messages ────────────────────────────────────────────
        let Some(ref mut rx) = self.session_rx else {
            self.done = true;
            return Poll::Ready(None);
        };

        match rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => {
                tracing::debug!("Received message in stream-based method");

                if msg.is_eos() {
                    self.done = true;
                    return Poll::Ready(None);
                }

                let code = RpcCode::from_metadata_str(
                    msg.metadata.get(STATUS_CODE_KEY).map(String::as_str),
                );
                if code != RpcCode::Ok {
                    let message = String::from_utf8_lossy(&msg.payload).into_owned();
                    self.done = true;
                    return Poll::Ready(Some(Err(RpcError::new(code, message))));
                }

                Poll::Ready(Some(Ok(msg.payload)))
            }
            Poll::Ready(None) => {
                self.done = true;
                Poll::Ready(Some(Err(RpcError::internal("Stream channel closed"))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A decoded request stream: `RawStream` items mapped through `Req::decode`.
///
/// Using a concrete named type (instead of `BoxStream`) lets the compiler
/// stack-allocate the stream combinator and eliminates the `BoxStream`
/// allocation that `RequestStream<Req>` required.
///
/// Handlers registered via `register_stream_unary_internal` /
/// `register_stream_stream_internal` receive `DecodedStream<Req>` directly.
pub type DecodedStream<Req> =
    futures::stream::Map<RawStream, fn(Result<Vec<u8>, RpcError>) -> Result<Req, RpcError>>;

/// Raw stream components passed to stream-input handlers.
///
/// Holding the raw components avoids boxing at the dispatch level. Each typed
/// wrapper calls [`StreamSource::into_raw_stream`] to get a [`RawStream`], then
/// maps it to the decoded type — with zero heap allocations.
pub struct StreamSource {
    pub session_rx: Option<mpsc::UnboundedReceiver<ReceivedMessage>>,
    pub payload: Vec<u8>,
    pub first_is_eos: bool,
    pub first_code: RpcCode,
}

impl StreamSource {
    /// Consume `self` and produce an allocation-free [`RawStream`].
    pub fn into_raw_stream(self) -> RawStream {
        RawStream {
            session_rx: self.session_rx,
            payload: self.payload,
            first_is_eos: self.first_is_eos,
            first_code: self.first_code,
            first_done: false,
            done: false,
        }
    }
}

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
    /// RPC call identifier, included in every response message so the client
    /// can route the response to the correct waiting caller over a shared session.
    rpc_id: &'a str,
}

impl<'a> RpcSession<'a> {
    /// Create a session for a unary-input RPC (unary-unary, unary-stream).
    pub fn new_unary(
        session_tx: &'a SessionTx,
        method_path: &'a str,
        first_message: ReceivedMessage,
        rpc_id: &'a str,
    ) -> Self {
        Self {
            session_tx,
            session_rx: None,
            method_path,
            first_message,
            rpc_id,
        }
    }

    /// Create a session for a stream-input RPC (stream-unary, stream-stream).
    pub fn new_stream(
        session_tx: &'a SessionTx,
        channel_rx: mpsc::UnboundedReceiver<ReceivedMessage>,
        method_path: &'a str,
        first_message: ReceivedMessage,
        rpc_id: &'a str,
    ) -> Self {
        Self {
            session_tx,
            session_rx: Some(channel_rx),
            method_path,
            first_message,
            rpc_id,
        }
    }

    /// Handle the session, dispatching to the appropriate interaction pattern.
    pub async fn handle(self, handler_info: HandlerInfo) -> Result<(), RpcError> {
        let Self {
            session_tx,
            session_rx,
            method_path,
            first_message,
            rpc_id,
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
                match handler_info {
                    HandlerInfo::Unary(handler) => {
                        let handler_result = (handler.as_ref())(payload, ctx).await?;
                        dispatch_response(session_tx, &source, rpc_id, handler_result).await
                    }
                    HandlerInfo::Stream(handler) => {
                        let stream_source = StreamSource { session_rx, payload, first_is_eos, first_code };
                        let handler_result = (handler.as_ref())(stream_source, ctx).await?;
                        dispatch_response(session_tx, &source, rpc_id, handler_result).await
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

/// Send the handler result to the client.
async fn dispatch_response(
    session_tx: &SessionTx,
    source: &Name,
    rpc_id: &str,
    handler_result: HandlerResponse,
) -> Result<(), RpcError> {
    match handler_result {
        HandlerResponse::Unary(bytes) => {
            send_response_stream(
                session_tx,
                futures::stream::once(std::future::ready(Ok(bytes))),
                rpc_id,
                source,
            )
            .await
        }
        HandlerResponse::Stream(stream) => {
            send_response_stream(session_tx, stream, rpc_id, source).await
        }
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
) -> Result<(), RpcError> {
    let mut metadata = create_status_metadata(RpcCode::Ok, rpc_id);
    if let Some(extra) = extra {
        metadata.extend(extra);
    }
    let handle = session_tx
        .publish(target, Vec::new(), Some("msg".to_string()), Some(metadata))
        .await
        .map_err(|e| RpcError::internal(format!("Failed to send EOS: {}", e)))?;
    handle
        .await
        .map_err(|e| RpcError::internal(format!("Failed to complete EOS send: {}", e)))
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

async fn send_response_stream<S>(
    session_tx: &SessionTx,
    mut stream: S,
    rpc_id: &str,
    target: &Name,
) -> Result<(), RpcError>
where
    S: futures::Stream<Item = Result<Vec<u8>, RpcError>> + Unpin,
{
    while let Some(result) = stream.next().await {
        match result {
            Ok(response_bytes) => {
                send_message(session_tx, target, response_bytes, RpcCode::Ok, rpc_id).await?;
            }
            Err(e) => return Err(e),
        }
    }
    send_eos(session_tx, target, rpc_id, None).await
}

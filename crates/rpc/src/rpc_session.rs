// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! RPC session handling implementation
//!
//! This module contains the logic for handling individual RPC sessions,
//! including message sending/receiving, timeout handling, and stream processing.

use std::{collections::HashMap, sync::Arc};

use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use slim_session::CompletionHandle;
use slim_session::errors::SessionError;
use tokio::sync::mpsc;

use slim_datapath::api::ProtoName as Name;

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
///
/// Goes to the session's destination, which in a group session is the whole
/// group; a server replies with [`send_error_to`] instead.
pub async fn send_error_for_rpc(
    session: &SessionTx,
    error: RpcError,
    rpc_id: &str,
) -> Result<(), RpcError> {
    send_error_to(session, session.destination(), error, rpc_id).await
}

/// Send an RPC-level error response to `target` only, as successful
/// responses are.
pub async fn send_error_to(
    session: &SessionTx,
    target: &Name,
    error: RpcError,
    rpc_id: &str,
) -> Result<(), RpcError> {
    let message = error.message().to_string();
    let metadata = create_status_metadata(error.code(), rpc_id);
    let handle = session
        .publish(
            target,
            message.into_bytes(),
            Some("msg".to_string()),
            Some(metadata),
        )
        .await
        .map_err(|e| RpcError::internal(format!("Failed to send error: {e}")))?;
    handle.await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to send error response");
        RpcError::internal(format!("Failed to complete error send: {e}"))
    })
}

/// Send an end-of-stream marker to `target`.
///
/// An EOS is an empty-payload message carrying `slimrpc-code = Ok`. The
/// status header is what marks it terminal — data frames carry no status —
/// so a response that encodes to zero bytes is not mistaken for the end of
/// the stream. Every server handler must send one after its final response so
/// GROUP/multicast callers can count per-member stream ends. P2P callers
/// discard it harmlessly.
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
        .map_err(|e| RpcError::internal(format!("Failed to send EOS: {e}")))
}

/// Metadata for a terminal message — an EOS or an error.
///
/// The presence of [`STATUS_CODE_KEY`] is what makes a message terminal, so
/// this must never be used for a data frame; see [`create_data_metadata`] and
/// [`ReceivedMessage::is_eos`](crate::ReceivedMessage::is_eos).
fn create_status_metadata(code: RpcCode, rpc_id: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    let code_i32: i32 = code.into();
    metadata.insert(STATUS_CODE_KEY.to_string(), code_i32.to_string());
    if !rpc_id.is_empty() {
        metadata.insert(RPC_ID_KEY.to_string(), rpc_id.to_string());
    }
    metadata
}

/// Metadata for a response data frame.
///
/// Deliberately omits [`STATUS_CODE_KEY`]: a status marks the end of the
/// stream, so attaching one to a data frame makes a response that encodes to
/// zero bytes indistinguishable from the terminator. Mirrors the client side,
/// where `first_msg_metadata` / `continuation_metadata` already omit it.
fn create_data_metadata(rpc_id: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
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

    let sending_failed = |e| RpcError::internal(format!("Failed to complete sending: {e}"));
    let mut in_flight = InFlightSends::default();

    while let Some(result) = stream.next().await {
        let response_bytes = result?;
        let handle = session_tx
            .publish(
                target,
                response_bytes,
                Some("msg".to_string()),
                Some(create_data_metadata(rpc_id)),
            )
            .await
            .map_err(|e| RpcError::internal(format!("Failed to send response: {e}")))?;
        in_flight.push(handle).await.map_err(sending_failed)?;
    }

    in_flight
        .push(send_eos(session_tx, target, rpc_id, None).await?)
        .await
        .map_err(sending_failed)?;
    in_flight.finish().await.map_err(sending_failed)
}

/// Most sends of one stream that may wait for their delivery confirmation at
/// once. Enough to keep a stream from waiting a round trip per message, while
/// a long-lived stream holds a bounded number of handles.
const MAX_IN_FLIGHT_SENDS: usize = 256;

/// The sends of one stream that haven't been confirmed yet. Confirmed ones are
/// dropped as the stream goes, so a failure surfaces at the next send instead
/// of when the stream ends, and a full window makes the sender wait.
#[derive(Default)]
pub(crate) struct InFlightSends {
    pending: FuturesUnordered<CompletionHandle>,
}

impl InFlightSends {
    /// Track `handle`, then wait while the window is full.
    pub(crate) async fn push(&mut self, handle: CompletionHandle) -> Result<(), SessionError> {
        self.pending.push(handle);
        while let Some(Some(result)) = self.pending.next().now_or_never() {
            result?;
        }
        while self.pending.len() >= MAX_IN_FLIGHT_SENDS {
            if let Some(result) = self.pending.next().await {
                result?;
            }
        }
        Ok(())
    }

    /// Wait for every send still in flight.
    pub(crate) async fn finish(mut self) -> Result<(), SessionError> {
        while let Some(result) = self.pending.next().await {
            result?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;

    fn pending_send() -> (oneshot::Sender<Result<(), SessionError>>, CompletionHandle) {
        let (tx, rx) = oneshot::channel();
        (tx, CompletionHandle::from_oneshot_receiver(rx))
    }

    #[tokio::test]
    async fn in_flight_sends_wait_while_the_window_is_full() {
        let mut in_flight = InFlightSends::default();
        let mut senders = Vec::new();
        for _ in 0..MAX_IN_FLIGHT_SENDS - 1 {
            let (tx, handle) = pending_send();
            senders.push(tx);
            in_flight.push(handle).await.unwrap();
        }

        let (last_tx, last) = pending_send();
        let mut push = Box::pin(in_flight.push(last));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut push)
                .await
                .is_err(),
            "a push into a full window must wait"
        );

        senders.pop().unwrap().send(Ok(())).unwrap();
        push.await.unwrap();

        for tx in senders.into_iter().chain([last_tx]) {
            tx.send(Ok(())).unwrap();
        }
        in_flight.finish().await.unwrap();
    }

    #[tokio::test]
    async fn in_flight_sends_report_a_failure_at_the_next_push() {
        let mut in_flight = InFlightSends::default();
        let (failed_tx, failed) = pending_send();
        in_flight.push(failed).await.unwrap();
        failed_tx
            .send(Err(SessionError::AckReception("lost".into())))
            .unwrap();

        let (_tx, next) = pending_send();
        assert!(matches!(
            in_flight.push(next).await,
            Err(SessionError::AckReception(_))
        ));
    }
}

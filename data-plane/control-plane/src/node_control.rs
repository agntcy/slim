// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::time::timeout;
use tonic::Status;

use crate::api::proto::controller::proto::v1::{ControlMessage, control_message::Payload};
use crate::error::{Error, Result};

pub const DEFAULT_RESPONSE_TIMEOUT_SECS: u64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Unknown,
    Connected,
    NotConnected,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeStatus::Unknown => write!(f, "Unknown"),
            NodeStatus::Connected => write!(f, "Connected"),
            NodeStatus::NotConnected => write!(f, "NotConnected"),
        }
    }
}

/// Discriminator used as the key for pending response channels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MessageKind {
    Ack,
    ConfigCommandAck,
    SubscriptionListResponse,
    ConnectionListResponse,
}

/// Returns true if the payload kind uses multi-chunk streaming (mpsc) rather than oneshot.
fn is_chunked_kind(kind: &MessageKind) -> bool {
    matches!(
        kind,
        MessageKind::ConnectionListResponse | MessageKind::SubscriptionListResponse
    )
}

/// Returns true when a chunked response message signals end-of-stream.
fn chunk_is_done(msg: &ControlMessage) -> bool {
    match &msg.payload {
        Some(Payload::ConnectionListResponse(r)) => r.done,
        Some(Payload::SubscriptionListResponse(r)) => r.done,
        _ => false,
    }
}

fn kind_from_payload(payload: &Payload) -> Option<MessageKind> {
    match payload {
        Payload::Ack(_) => Some(MessageKind::Ack),
        Payload::ConfigCommandAck(_) => Some(MessageKind::ConfigCommandAck),
        Payload::SubscriptionListResponse(_) => Some(MessageKind::SubscriptionListResponse),
        Payload::ConnectionListResponse(_) => Some(MessageKind::ConnectionListResponse),
        _ => None,
    }
}

fn original_message_id(payload: &Payload) -> Option<&str> {
    match payload {
        Payload::Ack(a) => Some(&a.original_message_id),
        Payload::ConfigCommandAck(a) => Some(&a.original_message_id),
        Payload::SubscriptionListResponse(r) => Some(&r.original_message_id),
        Payload::ConnectionListResponse(r) => Some(&r.original_message_id),
        _ => None,
    }
}

type PendingKey = (String, MessageKind, String); // (node_id, kind, original_msg_id)
type StreamTx = tokio::sync::mpsc::UnboundedSender<Result<ControlMessage, Status>>;

#[derive(Default)]
struct Inner {
    /// control stream towards a node
    streams: RwLock<HashMap<String, StreamTx>>,

    /// node statuses
    statuses: RwLock<HashMap<String, NodeStatus>>,

    /// in-flight response registry (oneshot, for single-message responses)
    pending: Mutex<HashMap<PendingKey, oneshot::Sender<ControlMessage>>>,

    /// in-flight chunked response registry (mpsc, for multi-chunk responses)
    chunked_pending: Mutex<HashMap<PendingKey, mpsc::UnboundedSender<ControlMessage>>>,
}

/// Thread-safe handler for per-node bidirectional gRPC streams.
#[derive(Clone, Default)]
pub struct DefaultNodeCommandHandler(Arc<Inner>);

impl DefaultNodeCommandHandler {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            streams: RwLock::new(HashMap::new()),
            statuses: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            chunked_pending: Mutex::new(HashMap::new()),
        }))
    }
}

impl DefaultNodeCommandHandler {
    pub async fn add_stream(&self, node_id: &str, tx: StreamTx) {
        self.0.streams.write().await.insert(node_id.to_string(), tx);
        self.update_connection_status(node_id, NodeStatus::Connected)
            .await;
    }

    pub async fn remove_stream(&self, node_id: &str) -> Result<()> {
        if self.0.streams.write().await.remove(node_id).is_none() {
            return Err(Error::StreamNotFound {
                node_id: node_id.to_string(),
            });
        }
        self.update_connection_status(node_id, NodeStatus::NotConnected)
            .await;
        // Cancel any in-flight waiters for this node so they unblock
        // immediately (ResponseChannelClosed) rather than blocking until the
        // 90 s timeout. Dropping the sender closes the channel; the receiver
        // in wait_for_response / send_and_wait gets Err from rx.await.
        {
            let mut pending = self.0.pending.lock().await;
            pending.retain(|k, _| k.0 != node_id);
        }
        {
            let mut chunked = self.0.chunked_pending.lock().await;
            chunked.retain(|k, _| k.0 != node_id);
        }
        self.0.statuses.write().await.remove(node_id);
        Ok(())
    }

    pub async fn get_connection_status(&self, node_id: &str) -> NodeStatus {
        self.0
            .statuses
            .read()
            .await
            .get(node_id)
            .copied()
            .unwrap_or(NodeStatus::Unknown)
    }

    pub async fn update_connection_status(&self, node_id: &str, status: NodeStatus) {
        self.0
            .statuses
            .write()
            .await
            .insert(node_id.to_string(), status);
    }

    pub async fn send_message(&self, node_id: &str, msg: ControlMessage) -> Result<()> {
        if node_id.is_empty() {
            return Err(Error::EmptyNodeId);
        }

        let status = self.get_connection_status(node_id).await;
        if status != NodeStatus::Connected {
            return Err(Error::NodeNotConnected {
                node_id: node_id.to_string(),
                status,
            });
        }

        let streams = self.0.streams.read().await;
        let tx = streams.get(node_id).ok_or_else(|| Error::StreamNotFound {
            node_id: node_id.to_string(),
        })?;

        tx.send(Ok(msg)).map_err(|e| Error::SendFailed {
            node_id: node_id.to_string(),
            reason: e.to_string(),
        })
    }

    /// Called when a response is received from a node. Wakes any waiter
    /// registered for the corresponding original message ID.
    pub async fn response_received(&self, node_id: &str, msg: ControlMessage) {
        if node_id.is_empty() {
            return;
        }
        let payload = match &msg.payload {
            Some(p) => p,
            None => return,
        };
        let kind = match kind_from_payload(payload) {
            Some(k) => k,
            None => {
                tracing::warn!("response_received: unsupported payload kind for node {node_id}");
                return;
            }
        };
        let orig_id = match original_message_id(payload) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                tracing::warn!("response_received: empty original_message_id for node {node_id}");
                return;
            }
        };
        let key = (node_id.to_string(), kind.clone(), orig_id);

        if is_chunked_kind(&kind) {
            let done = chunk_is_done(&msg);
            let mut chunked = self.0.chunked_pending.lock().await;
            if let Some(tx) = chunked.get(&key) {
                let _ = tx.send(msg);
                if done {
                    chunked.remove(&key);
                }
            } else {
                tracing::warn!(
                    "response_received: no chunked waiter for node={node_id} key={key:?}"
                );
            }
        } else {
            let mut pending = self.0.pending.lock().await;
            if let Some(tx) = pending.remove(&key) {
                let _ = tx.send(msg);
            } else {
                tracing::warn!("response_received: no waiter for node={node_id} key={key:?}");
            }
        }
    }

    /// Wait for a specific response from a node with the default timeout.
    ///
    /// **Prefer [`send_and_wait`] over calling `send_message` + `wait_for_response`
    /// separately.**  Registering the waiter after the send leaves a window where
    /// a fast response can arrive and be silently dropped.  This method is kept
    /// for test use only and is intentionally not part of the public API.
    #[cfg(test)]
    async fn wait_for_response(
        &self,
        node_id: &str,
        kind: ResponseKind,
        original_message_id: &str,
    ) -> Result<ControlMessage> {
        self.wait_for_response_with_timeout(
            node_id,
            kind,
            original_message_id,
            Duration::from_secs(DEFAULT_RESPONSE_TIMEOUT_SECS),
        )
        .await
    }

    #[cfg(test)]
    async fn wait_for_response_with_timeout(
        &self,
        node_id: &str,
        kind: ResponseKind,
        original_message_id: &str,
        dur: Duration,
    ) -> Result<ControlMessage> {
        if node_id.is_empty() {
            return Err(Error::EmptyNodeId);
        }
        if original_message_id.is_empty() {
            return Err(Error::EmptyMessageId);
        }
        let msg_kind: MessageKind = kind.into();
        let key = (
            node_id.to_string(),
            msg_kind,
            original_message_id.to_string(),
        );
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.0.pending.lock().await;
            pending.insert(key.clone(), tx);
        }
        match timeout(dur, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(Error::ResponseChannelClosed {
                node_id: node_id.to_string(),
            }),
            Err(_) => {
                // Remove the entry so it doesn't leak.
                self.0.pending.lock().await.remove(&key);
                Err(Error::ResponseTimeout {
                    node_id: node_id.to_string(),
                    kind,
                })
            }
        }
    }

    // Race-free combined send-and-wait helpers.
    //
    // These register the response waiter BEFORE calling send_message, eliminating
    // the window where a fast response could arrive between the send and the
    // wait_for_response call.

    /// Register a waiter, send `msg`, then wait for a single response of `kind`.
    pub async fn send_and_wait(
        &self,
        node_id: &str,
        msg: ControlMessage,
        kind: ResponseKind,
    ) -> Result<ControlMessage> {
        self.send_and_wait_with_timeout(
            node_id,
            msg,
            kind,
            Duration::from_secs(DEFAULT_RESPONSE_TIMEOUT_SECS),
        )
        .await
    }

    pub async fn send_and_wait_with_timeout(
        &self,
        node_id: &str,
        msg: ControlMessage,
        kind: ResponseKind,
        dur: Duration,
    ) -> Result<ControlMessage> {
        if node_id.is_empty() {
            return Err(Error::EmptyNodeId);
        }
        let message_id = msg.message_id.clone();
        if message_id.is_empty() {
            return Err(Error::EmptyMessageId);
        }
        let msg_kind: MessageKind = kind.into();
        let key = (node_id.to_string(), msg_kind, message_id);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.0.pending.lock().await;
            pending.insert(key.clone(), tx); // register BEFORE send
        }
        // Clean up the waiter if send fails.
        if let Err(e) = self.send_message(node_id, msg).await {
            self.0.pending.lock().await.remove(&key);
            return Err(e);
        }
        match timeout(dur, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(Error::ResponseChannelClosed {
                node_id: node_id.to_string(),
            }),
            Err(_) => {
                self.0.pending.lock().await.remove(&key);
                Err(Error::ResponseTimeout {
                    node_id: node_id.to_string(),
                    kind,
                })
            }
        }
    }

    /// Register a waiter, send `msg`, then collect all chunks until `done=true`.
    pub async fn send_and_wait_chunked(
        &self,
        node_id: &str,
        msg: ControlMessage,
        kind: ResponseKind,
    ) -> Result<Vec<ControlMessage>> {
        self.send_and_wait_chunked_with_timeout(
            node_id,
            msg,
            kind,
            Duration::from_secs(DEFAULT_RESPONSE_TIMEOUT_SECS),
        )
        .await
    }

    pub async fn send_and_wait_chunked_with_timeout(
        &self,
        node_id: &str,
        msg: ControlMessage,
        kind: ResponseKind,
        dur: Duration,
    ) -> Result<Vec<ControlMessage>> {
        if node_id.is_empty() {
            return Err(Error::EmptyNodeId);
        }
        let message_id = msg.message_id.clone();
        if message_id.is_empty() {
            return Err(Error::EmptyMessageId);
        }
        let msg_kind: MessageKind = kind.into();
        let key = (node_id.to_string(), msg_kind, message_id);
        let (tx, mut rx) = mpsc::unbounded_channel::<ControlMessage>();
        {
            let mut chunked = self.0.chunked_pending.lock().await;
            chunked.insert(key.clone(), tx); // register BEFORE send
        }
        if let Err(e) = self.send_message(node_id, msg).await {
            self.0.chunked_pending.lock().await.remove(&key);
            return Err(e);
        }
        let collect = async move {
            let mut chunks = Vec::new();
            while let Some(msg) = rx.recv().await {
                chunks.push(msg);
            }
            chunks
        };
        match timeout(dur, collect).await {
            Ok(chunks) => Ok(chunks),
            Err(_) => {
                self.0.chunked_pending.lock().await.remove(&key);
                Err(Error::ResponseTimeout {
                    node_id: node_id.to_string(),
                    kind,
                })
            }
        }
    }
}

/// Public enum used by callers to specify which response type they are waiting for.
#[derive(Debug, Clone, Copy)]
pub enum ResponseKind {
    Ack,
    ConfigCommandAck,
    SubscriptionListResponse,
    ConnectionListResponse,
}

impl From<ResponseKind> for MessageKind {
    fn from(k: ResponseKind) -> Self {
        match k {
            ResponseKind::Ack => MessageKind::Ack,
            ResponseKind::ConfigCommandAck => MessageKind::ConfigCommandAck,
            ResponseKind::SubscriptionListResponse => MessageKind::SubscriptionListResponse,
            ResponseKind::ConnectionListResponse => MessageKind::ConnectionListResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::api::proto::controller::proto::v1::{
        Ack, ConfigurationCommandAck, ControlMessage, control_message::Payload,
    };

    fn make_handler() -> DefaultNodeCommandHandler {
        DefaultNodeCommandHandler::new()
    }

    fn ack_message(original_id: &str, success: bool) -> ControlMessage {
        ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::Ack(Ack {
                original_message_id: original_id.to_string(),
                success,
                messages: vec![],
            })),
        }
    }

    fn config_ack_message(original_id: &str) -> ControlMessage {
        ControlMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            payload: Some(Payload::ConfigCommandAck(ConfigurationCommandAck {
                original_message_id: original_id.to_string(),
                connections_status: vec![],
                subscriptions_status: vec![],
            })),
        }
    }

    // ── NodeStatus Display ─────────────────────────────────────────────────

    #[test]
    fn node_status_display() {
        assert_eq!(NodeStatus::Unknown.to_string(), "Unknown");
        assert_eq!(NodeStatus::Connected.to_string(), "Connected");
        assert_eq!(NodeStatus::NotConnected.to_string(), "NotConnected");
    }

    // ── add/remove stream + status ─────────────────────────────────────────

    #[tokio::test]
    async fn add_stream_sets_connected() {
        let h = make_handler();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("node1", tx).await;
        assert_eq!(
            h.get_connection_status("node1").await,
            NodeStatus::Connected
        );
    }

    #[tokio::test]
    async fn remove_stream_prunes_status() {
        let h = make_handler();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("node1", tx).await;
        h.remove_stream("node1").await.unwrap();
        assert_eq!(h.get_connection_status("node1").await, NodeStatus::Unknown);
    }

    #[tokio::test]
    async fn remove_stream_missing_returns_error() {
        let h = make_handler();
        assert!(h.remove_stream("ghost").await.is_err());
    }

    #[tokio::test]
    async fn unknown_node_has_unknown_status() {
        let h = make_handler();
        assert_eq!(h.get_connection_status("nope").await, NodeStatus::Unknown);
    }

    // ── send_message ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_message_delivers_to_stream() {
        let h = make_handler();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let msg = ack_message("orig", true);
        h.send_message("n1", msg.clone()).await.unwrap();

        let received = rx.recv().await.unwrap().unwrap();
        assert_eq!(received.message_id, msg.message_id);
    }

    #[tokio::test]
    async fn send_message_empty_node_id_fails() {
        let h = make_handler();
        assert!(h.send_message("", ack_message("x", true)).await.is_err());
    }

    #[tokio::test]
    async fn send_message_not_connected_fails() {
        let h = make_handler();
        assert!(
            h.send_message("disconnected", ack_message("x", true))
                .await
                .is_err()
        );
    }

    // ── wait_for_response / response_received ──────────────────────────────

    #[tokio::test]
    async fn wait_for_response_ack() {
        let h = make_handler();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let msg_id = "req-1";
        // Spawn a task that reads from the stream and calls response_received.
        let h2 = h.clone();
        tokio::spawn(async move {
            // Drain the outbound message.
            let _ = rx.recv().await;
            let ack = ack_message(msg_id, true);
            h2.response_received("n1", ack).await;
        });

        let outbound = ControlMessage {
            message_id: msg_id.to_string(),
            payload: Some(Payload::Ack(Ack {
                original_message_id: String::new(),
                success: true,
                messages: vec![],
            })),
        };
        h.send_message("n1", outbound).await.unwrap();

        let resp = h
            .wait_for_response("n1", ResponseKind::Ack, msg_id)
            .await
            .unwrap();
        match resp.payload {
            Some(Payload::Ack(a)) => assert!(a.success),
            _ => panic!("unexpected payload"),
        }
    }

    #[tokio::test]
    async fn wait_for_response_config_command_ack() {
        let h = make_handler();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let msg_id = "req-cc";
        let h2 = h.clone();
        tokio::spawn(async move {
            let _ = rx.recv().await;
            h2.response_received("n1", config_ack_message(msg_id)).await;
        });

        h.send_message(
            "n1",
            ControlMessage {
                message_id: msg_id.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap();

        let resp = h
            .wait_for_response("n1", ResponseKind::ConfigCommandAck, msg_id)
            .await
            .unwrap();
        assert!(matches!(resp.payload, Some(Payload::ConfigCommandAck(_))));
    }

    #[tokio::test]
    async fn wait_for_response_timeout() {
        let h = make_handler();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let result = h
            .wait_for_response_with_timeout(
                "n1",
                ResponseKind::Ack,
                "will-never-arrive",
                Duration::from_millis(50),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::ResponseTimeout { .. }));
    }

    #[tokio::test]
    async fn wait_for_response_empty_node_id_fails() {
        let h = make_handler();
        let result = h
            .wait_for_response_with_timeout("", ResponseKind::Ack, "x", Duration::from_millis(10))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wait_for_response_empty_msg_id_fails() {
        let h = make_handler();
        let result = h
            .wait_for_response_with_timeout("n1", ResponseKind::Ack, "", Duration::from_millis(10))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn response_received_empty_node_id_is_noop() {
        let h = make_handler();
        // Should not panic.
        h.response_received("", ack_message("x", true)).await;
    }

    #[tokio::test]
    async fn response_received_no_waiter_is_logged() {
        let h = make_handler();
        // Should not panic even when there is no registered waiter.
        h.response_received("n1", ack_message("unknown-id", true))
            .await;
    }

    #[tokio::test]
    async fn response_received_no_payload_is_noop() {
        let h = make_handler();
        let msg = ControlMessage {
            message_id: "x".to_string(),
            payload: None,
        };
        h.response_received("n1", msg).await;
    }
}

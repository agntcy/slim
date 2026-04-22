// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, oneshot};
use tokio::time::timeout;
use tonic::Status;

use crate::api::proto::controller::proto::v1::{ControlMessage, control_message::Payload};

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

/// Thread-safe handler for per-node bidirectional gRPC streams.
pub struct DefaultNodeCommandHandler {
    /// node_id → unbounded sender into the server's response stream
    streams: RwLock<HashMap<String, StreamTx>>,
    /// node_id → connection status
    statuses: RwLock<HashMap<String, NodeStatus>>,
    /// per-node send mutex to serialise concurrent Send calls
    send_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// pending oneshot channels waiting for a specific response
    pending: Mutex<HashMap<PendingKey, oneshot::Sender<ControlMessage>>>,
}

impl DefaultNodeCommandHandler {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            streams: RwLock::new(HashMap::new()),
            statuses: RwLock::new(HashMap::new()),
            send_locks: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        })
    }
}

impl Default for DefaultNodeCommandHandler {
    fn default() -> Self {
        Self {
            streams: RwLock::new(HashMap::new()),
            statuses: RwLock::new(HashMap::new()),
            send_locks: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl DefaultNodeCommandHandler {
    pub async fn add_stream(&self, node_id: &str, tx: StreamTx) {
        self.streams.write().await.insert(node_id.to_string(), tx);
        self.update_connection_status(node_id, NodeStatus::Connected)
            .await;
    }

    pub async fn remove_stream(&self, node_id: &str) -> Result<(), String> {
        if self.streams.write().await.remove(node_id).is_none() {
            return Err(format!("no stream found for node {node_id}"));
        }
        self.update_connection_status(node_id, NodeStatus::NotConnected)
            .await;
        Ok(())
    }

    pub async fn get_connection_status(&self, node_id: &str) -> NodeStatus {
        self.statuses
            .read()
            .await
            .get(node_id)
            .copied()
            .unwrap_or(NodeStatus::Unknown)
    }

    pub async fn update_connection_status(&self, node_id: &str, status: NodeStatus) {
        self.statuses
            .write()
            .await
            .insert(node_id.to_string(), status);
    }

    pub async fn send_message(&self, node_id: &str, msg: ControlMessage) -> Result<(), String> {
        if node_id.is_empty() {
            return Err("nodeID cannot be empty".to_string());
        }

        // Per-node send lock.
        let lock = {
            let mut locks = self.send_locks.lock().await;
            locks
                .entry(node_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        let status = self.get_connection_status(node_id).await;
        if status != NodeStatus::Connected {
            return Err(format!(
                "node {node_id} is not connected, current status: {status}"
            ));
        }

        let streams = self.streams.read().await;
        let tx = streams
            .get(node_id)
            .ok_or_else(|| format!("no stream found for node {node_id}"))?;

        tx.send(Ok(msg))
            .map_err(|e| format!("failed to send message to node {node_id}: {e}"))
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
        let key = (node_id.to_string(), kind, orig_id);
        let mut pending = self.pending.lock().await;
        if let Some(tx) = pending.remove(&key) {
            let _ = tx.send(msg);
        } else {
            tracing::warn!("response_received: no waiter for node={node_id} key={key:?}");
        }
    }

    /// Wait for a specific response from a node with the default timeout.
    pub async fn wait_for_response(
        &self,
        node_id: &str,
        kind: ResponseKind,
        original_message_id: &str,
    ) -> Result<ControlMessage, String> {
        self.wait_for_response_with_timeout(
            node_id,
            kind,
            original_message_id,
            Duration::from_secs(DEFAULT_RESPONSE_TIMEOUT_SECS),
        )
        .await
    }

    pub async fn wait_for_response_with_timeout(
        &self,
        node_id: &str,
        kind: ResponseKind,
        original_message_id: &str,
        dur: Duration,
    ) -> Result<ControlMessage, String> {
        if node_id.is_empty() {
            return Err("nodeID cannot be empty".to_string());
        }
        if original_message_id.is_empty() {
            return Err("originalMessageID cannot be empty".to_string());
        }
        let msg_kind: MessageKind = kind.into();
        let key = (
            node_id.to_string(),
            msg_kind,
            original_message_id.to_string(),
        );
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(key, tx);
        }
        match timeout(dur, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err("response channel closed".to_string()),
            Err(_) => Err(format!(
                "timeout waiting for {kind:?} response from node {node_id}"
            )),
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

    fn make_handler() -> Arc<DefaultNodeCommandHandler> {
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
    async fn remove_stream_sets_not_connected() {
        let h = make_handler();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("node1", tx).await;
        h.remove_stream("node1").await.unwrap();
        assert_eq!(
            h.get_connection_status("node1").await,
            NodeStatus::NotConnected
        );
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
        assert!(result.unwrap_err().contains("timeout"));
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

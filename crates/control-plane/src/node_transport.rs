// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc};
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
    RouteListResponse,
    ConnectionListResponse,
}

/// Returns true when a response message signals end-of-stream (single-message
/// kinds always signal done).
fn is_done(msg: &ControlMessage) -> bool {
    match &msg.payload {
        Some(Payload::ConnectionListResponse(r)) => r.done,
        Some(Payload::RouteListResponse(r)) => r.done,
        Some(Payload::Ack(_)) | Some(Payload::ConfigCommandAck(_)) => true,
        _ => true,
    }
}

fn kind_from_payload(payload: &Payload) -> Option<MessageKind> {
    match payload {
        Payload::Ack(_) => Some(MessageKind::Ack),
        Payload::ConfigCommandAck(_) => Some(MessageKind::ConfigCommandAck),
        Payload::RouteListResponse(_) => Some(MessageKind::RouteListResponse),
        Payload::ConnectionListResponse(_) => Some(MessageKind::ConnectionListResponse),
        _ => None,
    }
}

fn original_message_id(payload: &Payload) -> Option<&str> {
    match payload {
        Payload::Ack(a) => Some(&a.original_message_id),
        Payload::ConfigCommandAck(a) => Some(&a.original_message_id),
        Payload::RouteListResponse(r) => Some(&r.original_message_id),
        Payload::ConnectionListResponse(r) => Some(&r.original_message_id),
        _ => None,
    }
}

type PendingKey = (String, MessageKind, String); // (node_id, kind, original_msg_id)
type StreamTx = tokio::sync::mpsc::UnboundedSender<Result<ControlMessage, Status>>;

struct Inner {
    /// control stream towards a node, with a generation counter to detect stale removals
    streams: RwLock<HashMap<String, (StreamTx, u64)>>,

    /// monotonically increasing counter assigned to each stream registration
    stream_epoch: AtomicU64,

    /// node statuses
    statuses: RwLock<HashMap<String, NodeStatus>>,

    /// in-flight response registry
    pending: Mutex<HashMap<PendingKey, mpsc::UnboundedSender<ControlMessage>>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            streams: RwLock::new(HashMap::new()),
            stream_epoch: AtomicU64::new(0),
            statuses: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }
}

/// Thread-safe handler for per-node bidirectional gRPC streams.
#[derive(Clone, Default)]
pub struct DefaultNodeCommandHandler(Arc<Inner>);

impl DefaultNodeCommandHandler {
    pub fn new() -> Self {
        Self(Arc::new(Inner::default()))
    }
}

impl DefaultNodeCommandHandler {
    /// Register a stream for a node, returning the epoch (generation) of this
    /// registration. Pass the epoch to `remove_stream` so stale cleanup from a
    /// previous connection doesn't remove a newer stream.
    pub async fn add_stream(&self, node_id: &str, tx: StreamTx) -> u64 {
        let epoch = self.0.stream_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        self.0
            .streams
            .write()
            .await
            .insert(node_id.to_string(), (tx, epoch));
        self.update_connection_status(node_id, NodeStatus::Connected)
            .await;
        epoch
    }

    /// Remove a stream only if its epoch matches the currently registered one.
    /// This prevents a stale cleanup task from removing a newer stream that
    /// replaced it during a rapid reconnect.
    pub async fn remove_stream(&self, node_id: &str, epoch: u64) -> Result<()> {
        let mut streams = self.0.streams.write().await;
        match streams.get(node_id) {
            Some((_, current_epoch)) if *current_epoch == epoch => {
                streams.remove(node_id);
            }
            Some(_) => {
                // A newer stream has been registered; skip cleanup.
                return Ok(());
            }
            None => {
                return Err(Error::StreamNotFound {
                    node_id: node_id.to_string(),
                });
            }
        }
        drop(streams);
        self.update_connection_status(node_id, NodeStatus::NotConnected)
            .await;
        {
            let mut pending = self.0.pending.lock().await;
            pending.retain(|k, _| k.0 != node_id);
        }
        self.0.statuses.write().await.remove(node_id);
        Ok(())
    }

    /// Forcibly remove the stream for a node regardless of epoch.
    /// Used when administratively removing a domain — all nodes in the domain
    /// must be disconnected.
    pub async fn force_remove_stream(&self, node_id: &str) {
        let removed = {
            let mut streams = self.0.streams.write().await;
            streams.remove(node_id).is_some()
        };
        if removed {
            self.update_connection_status(node_id, NodeStatus::NotConnected)
                .await;
            {
                let mut pending = self.0.pending.lock().await;
                pending.retain(|k, _| k.0 != node_id);
            }
            self.0.statuses.write().await.remove(node_id);
        }
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
        let (tx, _) = streams.get(node_id).ok_or_else(|| Error::StreamNotFound {
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
        let key = (node_id.to_string(), kind, orig_id);
        let done = is_done(&msg);

        let mut pending = self.0.pending.lock().await;
        if let Some(tx) = pending.get(&key) {
            let _ = tx.send(msg);
            if done {
                pending.remove(&key);
            }
        } else {
            tracing::warn!("response_received: no waiter for node={node_id} key={key:?}");
        }
    }

    /// Register a waiter, send `msg`, then collect all response messages until
    /// the stream signals done (single-message kinds complete after one message).
    pub async fn send_and_wait(
        &self,
        node_id: &str,
        msg: ControlMessage,
        kind: ResponseKind,
    ) -> Result<Vec<ControlMessage>> {
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
            let mut pending = self.0.pending.lock().await;
            pending.insert(key.clone(), tx);
        }

        if let Err(e) = self.send_message(node_id, msg).await {
            self.0.pending.lock().await.remove(&key);
            return Err(e);
        }

        let collect = async move {
            let mut msgs = Vec::new();
            while let Some(m) = rx.recv().await {
                msgs.push(m);
            }
            msgs
        };
        match timeout(dur, collect).await {
            Ok(msgs) => Ok(msgs),
            Err(_) => {
                self.0.pending.lock().await.remove(&key);
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
    RouteListResponse,
    ConnectionListResponse,
}

impl From<ResponseKind> for MessageKind {
    fn from(k: ResponseKind) -> Self {
        match k {
            ResponseKind::Ack => MessageKind::Ack,
            ResponseKind::ConfigCommandAck => MessageKind::ConfigCommandAck,
            ResponseKind::RouteListResponse => MessageKind::RouteListResponse,
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
                routes_status: vec![],
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
        let epoch = h.add_stream("node1", tx).await;
        h.remove_stream("node1", epoch).await.unwrap();
        assert_eq!(h.get_connection_status("node1").await, NodeStatus::Unknown);
    }

    #[tokio::test]
    async fn remove_stream_stale_epoch_is_noop() {
        let h = make_handler();
        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let epoch1 = h.add_stream("node1", tx1).await;
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let _epoch2 = h.add_stream("node1", tx2).await;
        // Removing with the old epoch should be a no-op.
        h.remove_stream("node1", epoch1).await.unwrap();
        assert_eq!(
            h.get_connection_status("node1").await,
            NodeStatus::Connected
        );
    }

    #[tokio::test]
    async fn remove_stream_missing_returns_error() {
        let h = make_handler();
        assert!(h.remove_stream("ghost", 1).await.is_err());
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

    // ── send_and_wait / response_received ──────────────────────────────────

    #[tokio::test]
    async fn send_and_wait_ack() {
        let h = make_handler();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let msg_id = "req-1";
        let h2 = h.clone();
        tokio::spawn(async move {
            let _ = rx.recv().await;
            h2.response_received("n1", ack_message(msg_id, true)).await;
        });

        let outbound = ControlMessage {
            message_id: msg_id.to_string(),
            payload: Some(Payload::Ack(Ack {
                original_message_id: String::new(),
                success: true,
                messages: vec![],
            })),
        };

        let msgs = h
            .send_and_wait("n1", outbound, ResponseKind::Ack)
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        match msgs.into_iter().next().unwrap().payload {
            Some(Payload::Ack(a)) => assert!(a.success),
            _ => panic!("unexpected payload"),
        }
    }

    #[tokio::test]
    async fn send_and_wait_config_command_ack() {
        let h = make_handler();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let msg_id = "req-cc";
        let h2 = h.clone();
        tokio::spawn(async move {
            let _ = rx.recv().await;
            h2.response_received("n1", config_ack_message(msg_id)).await;
        });

        let outbound = ControlMessage {
            message_id: msg_id.to_string(),
            payload: None,
        };

        let msgs = h
            .send_and_wait("n1", outbound, ResponseKind::ConfigCommandAck)
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(
            msgs.into_iter().next().unwrap().payload,
            Some(Payload::ConfigCommandAck(_))
        ));
    }

    #[tokio::test]
    async fn send_and_wait_timeout() {
        let h = make_handler();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        h.add_stream("n1", tx).await;

        let outbound = ControlMessage {
            message_id: "will-never-arrive".to_string(),
            payload: None,
        };

        let result = h
            .send_and_wait_with_timeout(
                "n1",
                outbound,
                ResponseKind::Ack,
                Duration::from_millis(50),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::ResponseTimeout { .. }));
    }

    #[tokio::test]
    async fn send_and_wait_empty_node_id_fails() {
        let h = make_handler();
        let outbound = ControlMessage {
            message_id: "x".to_string(),
            payload: None,
        };
        let result = h
            .send_and_wait_with_timeout("", outbound, ResponseKind::Ack, Duration::from_millis(10))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_and_wait_empty_msg_id_fails() {
        let h = make_handler();
        let outbound = ControlMessage {
            message_id: String::new(),
            payload: None,
        };
        let result = h
            .send_and_wait_with_timeout(
                "n1",
                outbound,
                ResponseKind::Ack,
                Duration::from_millis(10),
            )
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

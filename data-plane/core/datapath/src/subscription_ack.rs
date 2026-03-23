// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Remote subscription ACK lifecycle management.
//!
//! This module owns all state and logic for tracking, retrying, and resolving
//! remote subscription ACKs that are in-flight between relay nodes.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::oneshot;
use tracing::debug;

use crate::api::proto::dataplane::v1::Message;
use crate::connection::Connection;
use crate::errors::DataPathError;
use crate::message_processing::MessageProcessor;

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_RETRIES: u32 = 3;

/// Minimum remote SLIM version that supports subscription ACKs.
pub(crate) fn min_version() -> semver::Version {
    semver::Version::new(1, 2, 0)
}

/// Owns the in-flight pending ACK state.
#[derive(Debug)]
pub(crate) struct RemoteSubAckManager {
    pending: RwLock<HashMap<String, oneshot::Sender<Result<(), DataPathError>>>>,
}

impl RemoteSubAckManager {
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new in-flight ACK; returns the result receiver.
    pub fn register(&self, ack_id: &str) -> oneshot::Receiver<Result<(), DataPathError>> {
        let (tx, rx) = oneshot::channel();
        self.pending.write().insert(ack_id.to_string(), tx);
        rx
    }

    /// Deliver result to a waiting retry loop (no-op if unknown id).
    ///
    /// Removes the entry atomically — `oneshot::Sender::send` takes ownership.
    pub fn resolve(&self, ack_id: &str, result: Result<(), DataPathError>) {
        if let Some(tx) = self.pending.write().remove(ack_id) {
            let _ = tx.send(result);
        }
    }

    /// Remove a pending entry that never received a result (e.g. all retries exhausted).
    pub fn remove(&self, ack_id: &str) {
        self.pending.write().remove(ack_id);
    }
}

/// Returns `true` if the remote peer supports subscription ACKs (version ≥ 1.2.0).
pub(crate) fn supports(conn: &Connection) -> bool {
    conn.remote_slim_version()
        .is_some_and(|v| v >= min_version())
}

/// Send/timeout/retry loop for a remote subscription ACK.
///
/// Runs until an ACK is received, the channel is closed, or max retries are
/// exhausted. Cleans up the manager entry and notifies the upstream requester.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn retry_loop(
    processor: MessageProcessor,
    ack_id: String,
    forwarded_msg: Message,
    out_conn: u64,
    in_connection: u64,
    upstream_ack_id: Option<String>,
    mut rx: oneshot::Receiver<Result<(), DataPathError>>,
) {
    let mut final_result = Err(DataPathError::RemoteSubscriptionAckTimeout(MAX_RETRIES));

    'retry: for attempt in 0..=MAX_RETRIES {
        if let Err(e) = processor.send_msg(forwarded_msg.clone(), out_conn).await {
            final_result = Err(e);
            break;
        }
        tokio::select! {
            result = &mut rx => {
                match result {
                    Ok(r) => {
                        debug!(%ack_id, "subscription: remote ack received");
                        final_result = r;
                        break 'retry;
                    }
                    Err(_) => break, // sender dropped (processor shutdown)
                }
            }
            _ = tokio::time::sleep(TIMEOUT) => {
                debug!(attempt = attempt + 1, "remote sub ack timeout, retrying");
            }
        }
    }

    processor.remove_sub_ack(&ack_id);

    if let Some(id) = upstream_ack_id {
        processor
            .send_subscription_ack(in_connection, id, &final_result)
            .await;
    } else if let Err(e) = &final_result {
        use display_error_chain::ErrorChainExt;
        debug!(error = %e.chain(), "remote sub ack failed, no upstream to notify");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{Channel, Type as ConnectionType};

    #[tokio::test]
    async fn test_register_and_resolve_delivers_ok() {
        let manager = RemoteSubAckManager::new();
        let rx = manager.register("id-1");
        manager.resolve("id-1", Ok(()));
        let result = rx.await.expect("sender dropped unexpectedly");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_and_resolve_delivers_err() {
        let manager = RemoteSubAckManager::new();
        let rx = manager.register("id-err");
        manager.resolve(
            "id-err",
            Err(DataPathError::RemoteSubscriptionAckError(
                "boom".to_string(),
            )),
        );
        let result = rx.await.expect("sender dropped unexpectedly");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_id_is_noop() {
        let manager = RemoteSubAckManager::new();
        let mut rx = manager.register("id-known");
        // Resolve a different (unknown) id — must not affect the registered one.
        manager.resolve("id-unknown", Ok(()));
        assert!(
            rx.try_recv().is_err(),
            "registered channel must not have received anything"
        );
    }

    #[test]
    fn test_remove_cleans_up() {
        let manager = RemoteSubAckManager::new();
        manager.register("id-to-remove");
        assert!(manager.pending.read().contains_key("id-to-remove"));
        manager.remove("id-to-remove");
        assert!(!manager.pending.read().contains_key("id-to-remove"));
    }

    #[test]
    fn test_supports_no_version() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel(1);
        let conn = Connection::new(ConnectionType::Remote, Channel::Server(tx));
        assert!(!supports(&conn));
    }

    #[test]
    fn test_supports_old_version() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel(1);
        let conn = Connection::new(ConnectionType::Remote, Channel::Server(tx));
        conn.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::new(1, 1, 0),
        );
        assert!(!supports(&conn));
    }

    #[test]
    fn test_supports_exact_min_version() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel(1);
        let conn = Connection::new(ConnectionType::Remote, Channel::Server(tx));
        conn.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::new(1, 2, 0),
        );
        assert!(supports(&conn));
    }

    #[test]
    fn test_supports_newer_version() {
        use tokio::sync::mpsc;
        let (tx, _rx) = mpsc::channel(1);
        let conn = Connection::new(ConnectionType::Remote, Channel::Server(tx));
        conn.complete_negotiation_as_server(
            &uuid::Uuid::new_v4().to_string(),
            semver::Version::new(2, 0, 0),
        );
        assert!(supports(&conn));
    }
}

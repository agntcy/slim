// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore, mpsc};
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, Connection, ControlMessage, control_message::Payload,
};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus, ResponseKind};
use crate::services::routes::ReconcilerConfig;

pub struct LinkReconciler {
    db: SharedDb,
    cmd_handler: Arc<DefaultNodeCommandHandler>,
    queue_rx: mpsc::UnboundedReceiver<String>,
    queue_tx: mpsc::UnboundedSender<String>,
    route_queue_tx: mpsc::UnboundedSender<String>,
    requeue_counts: Arc<Mutex<HashMap<String, usize>>>,
    max_requeues: usize,
    semaphore: Arc<Semaphore>,
}

impl LinkReconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: Arc<DefaultNodeCommandHandler>,
        queue_rx: mpsc::UnboundedReceiver<String>,
        queue_tx: mpsc::UnboundedSender<String>,
        route_queue_tx: mpsc::UnboundedSender<String>,
        requeue_counts: Arc<Mutex<HashMap<String, usize>>>,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue_rx,
            queue_tx,
            route_queue_tx,
            requeue_counts,
            max_requeues: config.max_requeues,
            semaphore: Arc::new(Semaphore::new(config.max_parallel_reconciles)),
        }
    }

    pub async fn run(mut self) {
        tracing::info!("link reconciler: starting");

        while let Some(node_id) = self.queue_rx.recv().await {
            let permit = match self.semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };

            let db = self.db.clone();
            let cmd_handler = self.cmd_handler.clone();
            let queue_tx = self.queue_tx.clone();
            let route_queue_tx = self.route_queue_tx.clone();
            let requeue_counts = self.requeue_counts.clone();
            let max_requeues = self.max_requeues;

            tokio::spawn(async move {
                let _permit = permit;

                if let Err(e) = handle_request(&db, &cmd_handler, &node_id, &route_queue_tx).await {
                    tracing::error!("link reconciler: failed for node {node_id}: {e}");

                    let count = {
                        let mut counts = requeue_counts.lock().await;
                        let c = counts.entry(node_id.clone()).or_insert(0);
                        *c += 1;
                        *c
                    };

                    if count <= max_requeues {
                        tracing::debug!(
                            "link reconciler: requeuing node {node_id} (attempt {count}/{max_requeues})"
                        );
                        queue_tx.send(node_id).ok();
                    } else {
                        tracing::warn!(
                            "link reconciler: dropping node {node_id} after {max_requeues} retries"
                        );
                        requeue_counts.lock().await.remove(&node_id.clone());
                    }
                } else {
                    requeue_counts.lock().await.remove(&node_id);
                }
            });
        }

        tracing::info!("link reconciler: shutting down");
    }
}

async fn handle_request(
    db: &SharedDb,
    cmd_handler: &Arc<DefaultNodeCommandHandler>,
    node_id: &str,
    route_queue_tx: &mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    if cmd_handler.get_connection_status(node_id).await != NodeStatus::Connected {
        tracing::info!("link reconciler: node {node_id} not connected, skipping");
        return Ok(());
    }

    let links = db.get_links_for_node(node_id).await;

    // Maps link_id → Connection proto (deduplicated).
    let mut create_conn_map: HashMap<String, Connection> = HashMap::new();
    // Maps link_id → list of db Link entries marked deleted.
    let mut deleted_links_by_id: HashMap<String, Vec<crate::db::Link>> = HashMap::new();

    // The current node always gets its routes reconciled after link reconciliation.
    let mut reconcile_routes_for: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    reconcile_routes_for.insert(node_id.to_string());

    for link in &links {
        if link.source_node_id != node_id {
            continue;
        }

        if link.deleted {
            deleted_links_by_id
                .entry(link.link_id.clone())
                .or_default()
                .push(link.clone());
            continue;
        }

        // Inject link_id into the config data JSON if missing.
        let config_data = inject_link_id(&link.conn_config_data, &link.link_id);

        create_conn_map
            .entry(link.link_id.clone())
            .or_insert(Connection {
                connection_id: link.link_id.clone(),
                config_data,
            });
    }

    let connections_to_create: Vec<Connection> = create_conn_map.values().cloned().collect();
    let connections_to_delete: Vec<String> = deleted_links_by_id.keys().cloned().collect();

    let message_id = Uuid::new_v4().to_string();
    let msg = ControlMessage {
        message_id: message_id.clone(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create,
            connections_to_delete,
            subscriptions_to_set: vec![],
            subscriptions_to_delete: vec![],
        })),
    };

    tracing::info!(
        "link reconciler: sending config command to node {node_id} (msg_id={message_id})"
    );

    cmd_handler
        .send_message(node_id, msg)
        .await
        .map_err(|e| format!("failed to send link config command to node {node_id}: {e}"))?;

    let response = cmd_handler
        .wait_for_response(node_id, ResponseKind::ConfigCommandAck, &message_id)
        .await?;

    let ack = match response.payload {
        Some(Payload::ConfigCommandAck(a)) => a,
        _ => return Err(format!("received unexpected response from node {node_id}")),
    };

    // Build a map of connection_id → (success, error_msg) from the ack.
    let mut ack_status_by_id: HashMap<String, (bool, String)> = HashMap::new();
    for conn_ack in &ack.connections_status {
        ack_status_by_id.insert(
            conn_ack.connection_id.clone(),
            (conn_ack.success, conn_ack.error_msg.clone()),
        );
    }

    // Update status of created links.
    for link in &links {
        if link.source_node_id != node_id || link.deleted {
            continue;
        }
        if !create_conn_map.contains_key(&link.link_id) {
            continue;
        }
        if let Some((success, err_msg)) = ack_status_by_id.get(&link.link_id) {
            let mut updated = link.clone();
            if *success {
                updated.status = crate::db::LinkStatus::Applied;
                updated.status_msg = String::new();
                tracing::info!(
                    "link reconciler: link {} ({}→{}) applied",
                    link.link_id,
                    link.source_node_id,
                    link.dest_node_id
                );
                // Enqueue route reconciliation for all routes using this link.
                let routes = db.get_routes_by_link_id(&link.link_id).await;
                for r in routes {
                    reconcile_routes_for.insert(r.source_node_id.clone());
                }
            } else {
                updated.status = crate::db::LinkStatus::Failed;
                updated.status_msg = err_msg.clone();
                tracing::warn!(
                    "link reconciler: link {} ({}→{}) failed: {err_msg}",
                    link.link_id,
                    link.source_node_id,
                    link.dest_node_id
                );
            }
            db.update_link(updated).await?;
        }
    }

    // ACK-driven cleanup for deleted links.
    let mut retry_delete: Vec<String> = Vec::new();
    for (link_id, deleted_links) in &deleted_links_by_id {
        match ack_status_by_id.get(link_id) {
            Some((true, _)) => {
                for link in deleted_links {
                    db.delete_link(link).await?;
                }
                tracing::info!("link reconciler: deleted link records for {link_id}");
            }
            other => {
                let reason = match other {
                    Some((false, msg)) if !msg.is_empty() => msg.clone(),
                    Some((false, _)) => "delete ack reported failure".to_string(),
                    None => "missing delete ack".to_string(),
                    _ => unreachable!(),
                };
                for link in deleted_links {
                    let mut updated = link.clone();
                    updated.status = crate::db::LinkStatus::Failed;
                    updated.status_msg = reason.clone();
                    db.update_link(updated).await?;
                }
                retry_delete.push(link_id.clone());
                tracing::warn!("link reconciler: delete ack failed for link {link_id}: {reason}");
            }
        }
    }

    // Enqueue route reconciliation for affected source nodes.
    for src_node_id in &reconcile_routes_for {
        tracing::debug!("link reconciler: enqueuing route reconcile for node {src_node_id}");
        route_queue_tx.send(src_node_id.clone()).ok();
    }

    if !retry_delete.is_empty() {
        return Err(format!(
            "delete ack failed or missing for links: {retry_delete:?}"
        ));
    }

    Ok(())
}

/// Inject `"link_id"` into the JSON config data string if it is not already present.
fn inject_link_id(config_data: &str, link_id: &str) -> String {
    if let Ok(mut cfg) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(config_data)
        && !cfg.contains_key("link_id")
    {
        cfg.insert(
            "link_id".to_string(),
            serde_json::Value::String(link_id.to_string()),
        );
        if let Ok(updated) = serde_json::to_string(&cfg) {
            return updated;
        }
    }
    config_data.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_link_id_adds_missing_key() {
        let data = r#"{"endpoint":"http://x:8080"}"#;
        let result = inject_link_id(data, "my-link-id");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["link_id"], "my-link-id");
        assert_eq!(v["endpoint"], "http://x:8080");
    }

    #[test]
    fn inject_link_id_does_not_overwrite_existing() {
        let data = r#"{"link_id":"existing","endpoint":"http://x:8080"}"#;
        let result = inject_link_id(data, "new-link-id");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["link_id"], "existing");
    }

    #[test]
    fn inject_link_id_passthrough_on_invalid_json() {
        let data = "not json at all";
        let result = inject_link_id(data, "lid");
        assert_eq!(result, data);
    }

    #[test]
    fn inject_link_id_passthrough_on_empty_string() {
        let result = inject_link_id("", "lid");
        assert_eq!(result, "");
    }

    #[test]
    fn inject_link_id_passthrough_on_json_array() {
        // Arrays are not objects so the injection path is skipped.
        let data = r#"[1,2,3]"#;
        let result = inject_link_id(data, "lid");
        assert_eq!(result, data);
    }
}

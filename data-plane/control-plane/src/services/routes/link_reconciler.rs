// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use parking_lot::Mutex;

use crate::error::{Error, Result};

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, Connection, ConnectionDirection, ConnectionEntry, ConnectionListRequest,
    ControlMessage, control_message::Payload,
};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus, ResponseKind};
use crate::services::routes::ReconcilerConfig;
use crate::workqueue::WorkQueue;

use super::is_connection_not_found;

#[derive(Clone)]
pub struct LinkReconciler {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    queue: WorkQueue<String>,
    route_queue: WorkQueue<String>,
    max_requeues: usize,
    base_retry_delay: Duration,
    enable_orphan_detection: bool,
    requeue_counts: Arc<Mutex<HashMap<String, usize>>>,
}

impl LinkReconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        queue: WorkQueue<String>,
        route_queue: WorkQueue<String>,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue,
            route_queue,
            max_requeues: config.max_requeues,
            base_retry_delay: config.base_retry_delay.into(),
            enable_orphan_detection: config.enable_orphan_detection,
            requeue_counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(self) {
        tracing::info!("link reconciler: starting");

        while let Some(node_id) = self.queue.pop().await {
            if let Err(e) = handle_request(
                &self.db,
                &self.cmd_handler,
                &node_id,
                &self.route_queue,
                self.enable_orphan_detection,
            )
            .await
            {
                tracing::error!("link reconciler: failed for node {node_id}: {e}");

                let count = {
                    let mut counts = self.requeue_counts.lock();
                    let c = counts.entry(node_id.clone()).or_insert(0);
                    *c += 1;
                    *c
                };

                if count <= self.max_requeues {
                    let delay = crate::backoff::backoff_delay(count, self.base_retry_delay);
                    tracing::debug!(
                        "link reconciler: requeuing node {node_id} in {delay:?} (attempt {count}/{})",
                        self.max_requeues
                    );
                    self.queue.add_after(node_id.clone(), delay);
                } else {
                    tracing::warn!(
                        "link reconciler: dropping node {node_id} after {} retries",
                        self.max_requeues
                    );
                    self.requeue_counts.lock().remove(&node_id);
                }
            } else {
                self.requeue_counts.lock().remove(&node_id);
            }

            self.queue.done(&node_id);
        }

        tracing::info!("link reconciler: shutting down");
    }
}

/// Query the node's active connections and return all entries.
async fn query_node_connections(
    cmd_handler: &DefaultNodeCommandHandler,
    node_id: &str,
) -> Result<Vec<ConnectionEntry>> {
    let msg = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::ConnectionListRequest(ConnectionListRequest {})),
    };
    let chunks = cmd_handler
        .send_and_wait_chunked(node_id, msg, ResponseKind::ConnectionListResponse)
        .await?;
    let mut entries = Vec::new();
    for chunk in chunks {
        if let Some(Payload::ConnectionListResponse(r)) = chunk.payload {
            entries.extend(r.entries);
        }
    }
    Ok(entries)
}

async fn handle_request(
    db: &SharedDb,
    cmd_handler: &DefaultNodeCommandHandler,
    node_id: &str,
    route_queue: &WorkQueue<String>,
    enable_orphan_detection: bool,
) -> Result<()> {
    if cmd_handler.get_connection_status(node_id).await != NodeStatus::Connected {
        tracing::info!("link reconciler: node {node_id} not connected, skipping");
        return Ok(());
    }

    let links = db.get_links_for_node(node_id).await;

    // Pre-fetch all routes for this node once and group by link_id.
    // This avoids O(L) individual get_routes_by_link_id calls inside the loops.
    let mut routes_by_link: HashMap<String, Vec<String>> = HashMap::new();
    for r in db.get_routes_for_node_id(node_id).await {
        if !r.link_id.is_empty() {
            routes_by_link
                .entry(r.link_id.clone())
                .or_default()
                .push(r.source_node_id.clone());
        }
    }

    // Maps link_id → Connection proto (deduplicated).
    let mut create_conn_map: HashMap<String, Connection> = HashMap::new();
    // Maps link_id → list of db Link entries marked deleted.
    let mut deleted_links_by_id: HashMap<String, Vec<crate::db::Link>> = HashMap::new();
    // Set of link IDs the CP expects this node to have as outgoing connections
    // (includes both pending and applied, excludes deleted).
    let mut desired_link_ids: HashSet<String> = HashSet::new();

    // The current node always gets its routes reconciled after link reconciliation.
    let mut reconcile_routes_for: HashSet<String> = HashSet::new();
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
        if !link.link_id.is_empty() {
            desired_link_ids.insert(link.link_id.clone());
        }
    }

    // Query the node's live connections once:
    //   1. idempotency — Applied links already on the node don't need to be re-sent.
    //   2. orphan detection — outgoing connections absent from the DB must be removed.
    let live_connections = query_node_connections(cmd_handler, node_id).await?;
    let outgoing_direction = ConnectionDirection::Outgoing as i32;

    // Build the set of outgoing link IDs actually present on the node right now.
    let mut live_link_ids: HashSet<String> = HashSet::new();
    for entry in &live_connections {
        if entry.direction != outgoing_direction {
            continue;
        }
        if let Some(lid) = &entry.link_id
            && !lid.is_empty()
        {
            live_link_ids.insert(lid.clone());
        }
    }

    // Determine which non-deleted links actually need to be (re)applied vs.
    // which are already correctly established on the node.
    for link in &links {
        if link.source_node_id != node_id || link.deleted {
            continue;
        }

        // Idempotency: the link is Applied in the DB and the connection is
        // confirmed live on the node — no need to resend a create command.
        // Trigger route reconciliation so that any pending subscriptions on
        // this link are applied, but skip the connection create itself.
        if link.status == crate::db::LinkStatus::Applied && live_link_ids.contains(&link.link_id) {
            if let Some(src_ids) = routes_by_link.get(&link.link_id) {
                for src_id in src_ids {
                    reconcile_routes_for.insert(src_id.clone());
                }
            }
            tracing::debug!(
                "link reconciler: link {} ({}→{}) already live on node, skipping create",
                link.link_id,
                link.source_node_id,
                link.dest_node_id
            );
            continue;
        }

        // Connection is missing from the node or the link is Pending/Failed —
        // include it in the create list.
        let config_data = inject_link_id(&link.conn_config_data, &link.link_id);
        create_conn_map
            .entry(link.link_id.clone())
            .or_insert(Connection {
                connection_id: link.link_id.clone(),
                config_data,
            });
    }

    // Orphan detection: outgoing connections present on the node but absent
    // from the DB desired state (and not already being deleted via a DB record).
    // Only active when `enable_orphan_detection` is set in the reconciler config.
    // Disabled by default to avoid deleting connections created outside the CP
    // (e.g. by a previous CP instance or manual configuration).
    let mut orphan_link_ids: HashSet<String> = HashSet::new();
    if enable_orphan_detection {
        for lid in &live_link_ids {
            if desired_link_ids.contains(lid) || deleted_links_by_id.contains_key(lid.as_str()) {
                continue;
            }
            tracing::info!(
                "link reconciler: orphan connection on node {node_id}: link {lid} — scheduling removal"
            );
            orphan_link_ids.insert(lid.clone());
        }
    }

    let connections_to_create: Vec<Connection> = create_conn_map.values().cloned().collect();
    let mut connections_to_delete: Vec<String> = deleted_links_by_id.keys().cloned().collect();
    connections_to_delete.extend(orphan_link_ids.iter().cloned());

    if connections_to_create.is_empty() && connections_to_delete.is_empty() {
        // Nothing to do — all desired links match node state and no orphans found.
        // Enqueue route reconciliation and return.
        for src_node_id in &reconcile_routes_for {
            route_queue.add(src_node_id.clone());
        }
        return Ok(());
    }

    {
        use super::DisplayConnection;
        let create_list: Vec<_> = connections_to_create.iter().map(|c| DisplayConnection(c).to_string()).collect();
        tracing::info!(
            "link reconciler: sending config command to node {node_id}: \
             create=[{}], delete=[{}]",
            create_list.join(", "),
            connections_to_delete.join(", "),
        );
    }

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

    let response = cmd_handler
        .send_and_wait(node_id, msg, ResponseKind::ConfigCommandAck)
        .await?;

    let ack = match response.payload {
        Some(Payload::ConfigCommandAck(a)) => a,
        _ => {
            return Err(Error::UnexpectedResponse(format!(
                "received unexpected response from node {node_id}"
            )));
        }
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
                if let Some(src_ids) = routes_by_link.get(&link.link_id) {
                    for src_id in src_ids {
                        reconcile_routes_for.insert(src_id.clone());
                    }
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
                // If the connection was already gone on the data plane, the
                // desired state is reached — treat it as a successful delete.
                if is_connection_not_found(&reason) {
                    for link in deleted_links {
                        db.delete_link(link).await?;
                    }
                    tracing::info!(
                        "link reconciler: connection already removed on dataplane, deleted link records for {link_id}"
                    );
                    continue;
                }
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

    // ACK-driven cleanup for orphan connections (no DB records to update).
    for link_id in &orphan_link_ids {
        match ack_status_by_id.get(link_id.as_str()) {
            Some((true, _)) => {
                tracing::info!(
                    "link reconciler: removed orphan connection {link_id} from node {node_id}"
                );
            }
            Some((false, msg)) => {
                if is_connection_not_found(msg) {
                    tracing::debug!(
                        "link reconciler: orphan connection {link_id} already absent on node {node_id}"
                    );
                } else {
                    tracing::warn!(
                        "link reconciler: failed to remove orphan connection {link_id} from node {node_id}: {msg}"
                    );
                    retry_delete.push(link_id.clone());
                }
            }
            None => {
                tracing::debug!(
                    "link reconciler: no ack for orphan connection {link_id} on node {node_id}"
                );
            }
        }
    }

    // Enqueue route reconciliation for affected source nodes.
    for src_node_id in &reconcile_routes_for {
        tracing::debug!("link reconciler: enqueuing route reconcile for node {src_node_id}");
        route_queue.add(src_node_id.clone());
    }

    if !retry_delete.is_empty() {
        return Err(Error::InvalidInput(format!(
            "delete ack failed or missing for links: {retry_delete:?}"
        )));
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

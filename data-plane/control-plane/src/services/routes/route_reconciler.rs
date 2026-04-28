// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;
use uuid::Uuid;

use crate::config::ReconcilerConfig;
use crate::error::{Error, Result};

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, ConnectionListRequest, ControlMessage, Subscription, SubscriptionEntry,
    SubscriptionListRequest, control_message::Payload,
};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus, ResponseKind};
use crate::workqueue::WorkQueue;

pub struct RouteReconciler {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    queue: WorkQueue<String>,
    link_queue: WorkQueue<String>,
    max_requeues: usize,
    base_retry_delay_ms: u64,
}

impl RouteReconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        queue: WorkQueue<String>,
        link_queue: WorkQueue<String>,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue,
            link_queue,
            max_requeues: config.max_requeues,
            base_retry_delay_ms: config.base_retry_delay_ms,
        }
    }

    pub async fn run(self) {
        tracing::info!("route reconciler: starting");

        let mut requeue_counts: HashMap<String, usize> = HashMap::new();

        while let Some(node_id) = self.queue.pop().await {
            if let Err(e) =
                handle_request(&self.db, &self.cmd_handler, &self.link_queue, &node_id).await
            {
                tracing::error!("route reconciler: failed for node {node_id}: {e}");

                let count = {
                    let c = requeue_counts.entry(node_id.clone()).or_insert(0);
                    *c += 1;
                    *c
                };

                if count <= self.max_requeues {
                    let delay = crate::backoff::backoff_delay(count, self.base_retry_delay_ms);
                    tracing::debug!(
                        "route reconciler: requeuing node {node_id} in {delay:?} (attempt {count}/{})",
                        self.max_requeues
                    );
                    self.queue.add_after(node_id.clone(), delay);
                } else {
                    tracing::warn!(
                        "route reconciler: dropping node {node_id} after {} retries",
                        self.max_requeues
                    );
                    requeue_counts.remove(&node_id);
                }
            } else {
                requeue_counts.remove(&node_id);
            }

            self.queue.done(&node_id);
        }

        tracing::info!("route reconciler: shutting down");
    }
}

/// Query the node's active connection table and return the set of link IDs
/// currently registered on it. Aggregates all chunks until `done=true`.
async fn query_node_connections(
    cmd_handler: &DefaultNodeCommandHandler,
    node_id: &str,
) -> Result<HashSet<String>> {
    let msg = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::ConnectionListRequest(ConnectionListRequest {})),
    };

    let chunks = cmd_handler
        .send_and_wait_chunked(node_id, msg, ResponseKind::ConnectionListResponse)
        .await?;

    let mut result = HashSet::new();
    for chunk in chunks {
        if let Some(Payload::ConnectionListResponse(r)) = chunk.payload {
            for entry in r.entries {
                if let Some(id) = entry.link_id
                    && !id.is_empty()
                {
                    result.insert(id);
                }
            }
        }
    }
    Ok(result)
}

/// `NULL_COMPONENT` is the sentinel used by the data-plane for subscriptions
/// without an explicit component ID (see `slim_datapath::messages::Name::NULL_COMPONENT`).
const NULL_COMPONENT_ID: u64 = u64::MAX;

/// Query the node's active subscription table. Aggregates all chunks until `done=true`.
async fn query_node_subscriptions(
    cmd_handler: &DefaultNodeCommandHandler,
    node_id: &str,
) -> Result<Vec<SubscriptionEntry>> {
    let msg = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::SubscriptionListRequest(SubscriptionListRequest {})),
    };

    let chunks = cmd_handler
        .send_and_wait_chunked(node_id, msg, ResponseKind::SubscriptionListResponse)
        .await?;

    let mut entries = Vec::new();
    for chunk in chunks {
        if let Some(Payload::SubscriptionListResponse(r)) = chunk.payload {
            entries.extend(r.entries);
        }
    }
    Ok(entries)
}

/// Reset a link to Pending and poke the link reconciler to re-apply it.
///
/// Used in two places:
/// - **Pre-flight** (proactive): the node's connection table doesn't contain
///   the link_id yet, so we defer rather than sending a doomed subscription.
/// - **Ack safety-net** (reactive): the node returned "connection not found"
///   despite passing the pre-flight check (TOCTOU race).
async fn defer_link(
    db: &SharedDb,
    link_queue: &WorkQueue<String>,
    link_id: &str,
    source_node_id: &str,
    dest_node_id: &str,
) {
    if let Some(link) = db
        .get_link(link_id, source_node_id, dest_node_id)
        .await
        .filter(|l| !l.deleted)
    {
        if link.status == crate::db::LinkStatus::Applied {
            let mut updated = link.clone();
            updated.status = crate::db::LinkStatus::Pending;
            updated.status_msg = "reset: connection not registered on node".to_string();
            updated.last_updated = SystemTime::now();
            if let Err(e) = db.update_link(updated).await {
                tracing::error!("route reconciler: failed to reset link {link_id} to pending: {e}");
            }
        }
        link_queue.add(link.source_node_id.clone());
    }
}

async fn handle_request(
    db: &SharedDb,
    cmd_handler: &DefaultNodeCommandHandler,
    link_queue: &WorkQueue<String>,
    node_id: &str,
) -> Result<()> {
    if cmd_handler.get_connection_status(node_id).await != NodeStatus::Connected {
        tracing::info!("route reconciler: node {node_id} not connected, skipping");
        return Ok(());
    }

    let routes = db.get_routes_for_node_id(node_id).await;

    // Query the node's live connection table and subscription table once before
    // iterating routes.  This pre-flight state is used for two purposes:
    //
    // 1. Connection pre-flight: any subscription referencing a link_id not yet
    //    registered on the node is deferred — the link reconciler re-enqueues
    //    route reconciliation once the connection is established.
    //
    // 2. Subscription idempotency: if a route is already applied on the node
    //    (verified live), we mark the DB record Applied and skip the send.
    //    If a route marked deleted is already absent from the node, we delete
    //    the DB record directly without a round-trip.
    let node_link_ids = query_node_connections(cmd_handler, node_id).await?;
    tracing::debug!(
        "route reconciler: node {node_id} has {} registered link(s): {:?}",
        node_link_ids.len(),
        node_link_ids
    );

    let node_subs = query_node_subscriptions(cmd_handler, node_id).await?;
    tracing::debug!(
        "route reconciler: node {node_id} has {} subscription entries",
        node_subs.len()
    );

    // Build a set of (c0, c1, c2, component_id, link_id) representing subscriptions
    // currently active on the node.  NULL_COMPONENT_ID (u64::MAX) is normalised to
    // None so it matches routes that have no explicit component_id.
    type SubKey = (String, String, String, Option<u64>, String);
    let mut applied_sub_keys: HashSet<SubKey> = HashSet::new();
    for entry in &node_subs {
        let cid = match entry.id {
            Some(x) if x == NULL_COMPONENT_ID => None,
            other => other,
        };
        for rc in &entry.remote_connections {
            if let Some(lid) = &rc.link_id
                && !lid.is_empty()
            {
                applied_sub_keys.insert((
                    entry.component_0.clone(),
                    entry.component_1.clone(),
                    entry.component_2.clone(),
                    cid,
                    lid.clone(),
                ));
            }
        }
    }

    // Build the complete set of (c0, c1, c2, component_id, link_id) keys tracked
    // by the CP for this node — both desired (non-deleted) and being-deleted routes.
    // Subscriptions on the node that are NOT in this set have no CP record at all
    // and are true orphans.  Deleted-route subscriptions are intentionally included
    // here so they are NOT double-queued: the route processing loop below handles
    // them via the normal deletion path.
    let mut tracked_sub_keys: HashSet<SubKey> = HashSet::new();
    for route in &routes {
        let key: SubKey = (
            route.component0.clone(),
            route.component1.clone(),
            route.component2.clone(),
            route.component_id.map(|v| v as u64),
            route.link_id.clone(),
        );
        tracked_sub_keys.insert(key);
    }

    let mut subscriptions_to_set: Vec<Subscription> = Vec::new();
    let mut subscriptions_to_delete: Vec<Subscription> = Vec::new();

    // Schedule deletion of orphan subscriptions: present on node, absent from DB.
    for entry in &node_subs {
        let cid = match entry.id {
            Some(x) if x == NULL_COMPONENT_ID => None,
            other => other,
        };
        for rc in &entry.remote_connections {
            if let Some(lid) = &rc.link_id {
                if lid.is_empty() {
                    continue;
                }
                let key: SubKey = (
                    entry.component_0.clone(),
                    entry.component_1.clone(),
                    entry.component_2.clone(),
                    cid,
                    lid.clone(),
                );
                if !tracked_sub_keys.contains(&key) {
                    tracing::info!(
                        "route reconciler: orphan subscription on node {node_id}: \
                         ({}, {}, {}, {:?}) via link {lid} — scheduling removal",
                        entry.component_0,
                        entry.component_1,
                        entry.component_2,
                        cid,
                    );
                    subscriptions_to_delete.push(Subscription {
                        connection_id: lid.clone(),
                        component_0: entry.component_0.clone(),
                        component_1: entry.component_1.clone(),
                        component_2: entry.component_2.clone(),
                        id: entry.id,
                        link_id: Some(lid.clone()),
                        node_id: None,
                        ..Default::default()
                    });
                }
            }
        }
    }

    for route in &routes {
        let link_id = route.link_id.clone();
        let sub = Subscription {
            connection_id: link_id.clone(),
            component_0: route.component0.clone(),
            component_1: route.component1.clone(),
            component_2: route.component2.clone(),
            id: route.component_id.map(|v| v as u64),
            link_id: Some(link_id.clone()),
            node_id: if route.dest_node_id.is_empty() {
                None
            } else {
                Some(route.dest_node_id.clone())
            },
            ..Default::default()
        };

        if route.deleted {
            // If the subscription is already gone from the node, just clean up the DB.
            let key: SubKey = (
                route.component0.clone(),
                route.component1.clone(),
                route.component2.clone(),
                route.component_id.map(|v| v as u64),
                link_id.clone(),
            );
            if !applied_sub_keys.contains(&key) {
                db.delete_route(route.id).await?;
                tracing::debug!(
                    "route reconciler: subscription already absent on node, deleted route {} from DB",
                    route.id
                );
                continue;
            }
            subscriptions_to_delete.push(sub);
            continue;
        }

        // Check that the link exists and is Applied in the DB.
        let link = db
            .get_link(&link_id, &route.source_node_id, &route.dest_node_id)
            .await;
        match link {
            None => {
                tracing::warn!(
                    "route reconciler: skipping route {:?} — link not found",
                    route
                );
                continue;
            }
            Some(l) if l.status == crate::db::LinkStatus::Failed => {
                let msg = if l.status_msg.is_empty() {
                    "link configuration failed".to_string()
                } else {
                    l.status_msg.clone()
                };
                db.mark_route_failed(route.id, &msg).await?;
                continue;
            }
            Some(l) if l.status != crate::db::LinkStatus::Applied => {
                tracing::debug!(
                    "route reconciler: waiting for link {link_id} to apply before sending route, poking link reconciler for {}",
                    l.source_node_id
                );
                link_queue.add(l.source_node_id.clone());
                continue;
            }
            _ => {}
        }

        // Pre-flight connection check: the link is Applied in the DB, but verify
        // the underlying connection is actually present on the node right now.
        if !link_id.is_empty() && !node_link_ids.contains(&link_id) {
            tracing::debug!(
                "route reconciler: link {link_id} not yet registered on {node_id}, deferring route"
            );
            defer_link(
                db,
                link_queue,
                &link_id,
                &route.source_node_id,
                &route.dest_node_id,
            )
            .await;
            continue;
        }

        // Idempotency check: subscription already active on the node.
        let key: SubKey = (
            route.component0.clone(),
            route.component1.clone(),
            route.component2.clone(),
            route.component_id.map(|v| v as u64),
            link_id.clone(),
        );
        if applied_sub_keys.contains(&key) {
            db.mark_route_applied(route.id).await?;
            tracing::debug!(
                "route reconciler: route {} already applied on node, skipping send",
                route.id
            );
            continue;
        }

        subscriptions_to_set.push(sub);
    }

    if subscriptions_to_set.is_empty() && subscriptions_to_delete.is_empty() {
        // All routes are either deferred (link not yet applied or connection not
        // yet registered on the node) or skipped.  The link reconciler will
        // re-enqueue this node when the link applies — no retry needed here.
        return Ok(());
    }

    let message_id = Uuid::new_v4().to_string();
    let msg = ControlMessage {
        message_id: message_id.clone(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: vec![],
            connections_to_delete: vec![],
            subscriptions_to_set,
            subscriptions_to_delete,
        })),
    };

    tracing::info!(
        "route reconciler: sending config command to node {node_id} (msg_id={message_id})"
    );

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

    for sub_ack in &ack.subscriptions_status {
        let sub = match &sub_ack.subscription {
            Some(s) => s,
            None => continue,
        };

        let dest_node_id = match sub.node_id.clone() {
            Some(id) if !id.is_empty() => id,
            _ => {
                tracing::warn!(
                    "route reconciler: subscription ack has no node_id, skipping: {:?}",
                    sub
                );
                continue;
            }
        };
        let link_id = sub.link_id.clone().unwrap_or_default();
        let component_id = sub.id.map(|v| v as i64);

        let route = db
            .get_route_for_src_dest_name(
                node_id,
                &crate::db::SubscriptionName {
                    component0: &sub.component_0,
                    component1: &sub.component_1,
                    component2: &sub.component_2,
                    component_id,
                },
                &dest_node_id,
                &link_id,
            )
            .await;

        let route = match route {
            Some(r) => r,
            None => {
                tracing::warn!(
                    "route reconciler: no route found for subscription ack: {:?}",
                    sub
                );
                continue;
            }
        };

        if sub_ack.success {
            if route.deleted {
                db.delete_route(route.id).await?;
                tracing::info!("route reconciler: deleted route {}", route.id);
            } else {
                db.mark_route_applied(route.id).await?;
                tracing::debug!("route reconciler: marked route {} as applied", route.id);
            }
        } else {
            let err_msg = sub_ack.error_msg.clone();

            // If we were trying to delete and the subscription wasn't found,
            // the desired state is already reached — treat as success.
            if route.deleted && is_subscription_not_found(&err_msg) {
                db.delete_route(route.id).await?;
                tracing::info!(
                    "route reconciler: subscription already removed on dataplane, deleted route {}",
                    route.id
                );
                continue;
            }

            // Safety net for the TOCTOU race: the pre-flight check passed but
            // the connection disappeared between the query and the subscribe
            // send (extremely unlikely in practice).  Treat identically to the
            // pre-flight deferral path — reset and hand back to the link
            // reconciler without consuming a retry slot.
            if !route.link_id.is_empty() && is_connection_not_found(&err_msg) {
                tracing::warn!(
                    "route reconciler: connection not found for route {} (link {}) — resetting link for re-reconciliation",
                    route.id,
                    route.link_id
                );
                defer_link(
                    db,
                    link_queue,
                    &route.link_id,
                    &route.source_node_id,
                    &route.dest_node_id,
                )
                .await;
                continue;
            }

            db.mark_route_failed(route.id, &err_msg).await?;
            tracing::error!(
                "route reconciler: marked route {} as failed: {err_msg}",
                route.id
            );
        }
    }

    Ok(())
}

fn is_subscription_not_found(msg: &str) -> bool {
    msg.to_lowercase().contains("subscription not found")
}

fn is_connection_not_found(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("connection not found")
        || lower.contains("no such connection")
        || (lower.contains("connection") && lower.contains("not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_subscription_not_found ─────────────────────────────────────────

    #[test]
    fn sub_not_found_exact_match() {
        assert!(is_subscription_not_found("subscription not found"));
    }

    #[test]
    fn sub_not_found_case_insensitive() {
        assert!(is_subscription_not_found("Subscription Not Found"));
        assert!(is_subscription_not_found("SUBSCRIPTION NOT FOUND"));
    }

    #[test]
    fn sub_not_found_embedded_in_longer_message() {
        assert!(is_subscription_not_found(
            "error: subscription not found for id=42"
        ));
    }

    #[test]
    fn sub_not_found_no_match() {
        assert!(!is_subscription_not_found("connection not found"));
        assert!(!is_subscription_not_found(""));
        assert!(!is_subscription_not_found("subscription was applied"));
    }

    // ── is_connection_not_found ───────────────────────────────────────────

    #[test]
    fn conn_not_found_direct_phrase() {
        assert!(is_connection_not_found("connection not found"));
        assert!(is_connection_not_found("Connection Not Found"));
    }

    #[test]
    fn conn_not_found_no_such_connection() {
        assert!(is_connection_not_found("no such connection"));
        assert!(is_connection_not_found("No Such Connection exists"));
    }

    #[test]
    fn conn_not_found_connection_and_not_found_split() {
        assert!(is_connection_not_found(
            "the connection a3916184 was not found in the table"
        ));
    }

    #[test]
    fn conn_not_found_no_match() {
        assert!(!is_connection_not_found("subscription not found"));
        assert!(!is_connection_not_found(""));
        assert!(!is_connection_not_found(
            "connection established successfully"
        ));
        assert!(!is_connection_not_found("route not found"));
    }
}

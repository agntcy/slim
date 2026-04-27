// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{Error, Result};

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, ControlMessage, Subscription, control_message::Payload,
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
}

impl RouteReconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        queue: WorkQueue<String>,
        link_queue: WorkQueue<String>,
        max_requeues: usize,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue,
            link_queue,
            max_requeues,
        }
    }

    pub async fn run(self) {
        tracing::info!("route reconciler: starting");

        let mut requeue_counts: HashMap<String, usize> = HashMap::new();

        while let Some(node_id) = self.queue.pop().await {
            if let Err(e) = handle_request(&self.db, &self.cmd_handler, &self.link_queue, &node_id).await {
                tracing::error!("route reconciler: failed for node {node_id}: {e}");

                let count = {
                    let c = requeue_counts.entry(node_id.clone()).or_insert(0);
                    *c += 1;
                    *c
                };

                if count <= self.max_requeues {
                    tracing::debug!(
                        "route reconciler: requeuing node {node_id} (attempt {count}/{})",
                        self.max_requeues
                    );
                    self.queue.add(node_id.clone());
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

    let mut subscriptions_to_set: Vec<Subscription> = Vec::new();
    let mut subscriptions_to_delete: Vec<Subscription> = Vec::new();

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
            subscriptions_to_delete.push(sub);
            continue;
        }

        // Check that the link exists and is applied.
        let link = db.get_link(&link_id, &route.source_node_id, &route.dest_node_id).await;
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

        subscriptions_to_set.push(sub);
    }

    if subscriptions_to_set.is_empty() && subscriptions_to_delete.is_empty() {
        // All routes are either deferred (link not yet applied) or skipped.
        // The link reconciler will re-enqueue this node when the link applies — no retry needed.
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

    cmd_handler
        .send_message(node_id, msg)
        .await?;

    let response = cmd_handler
        .wait_for_response(node_id, ResponseKind::ConfigCommandAck, &message_id)
        .await?;

    let ack = match response.payload {
        Some(Payload::ConfigCommandAck(a)) => a,
        _ => return Err(Error::UnexpectedResponse(format!("received unexpected response from node {node_id}"))),
    };

    for sub_ack in &ack.subscriptions_status {
        let sub = match &sub_ack.subscription {
            Some(s) => s,
            None => continue,
        };

        let dest_node_id = sub.node_id.clone().unwrap_or_default();
        let link_id = sub.link_id.clone().unwrap_or_default();
        let component_id = sub.id.map(|v| v as i64);

        let route = db.get_route_for_src_dest_name(
            node_id,
            &crate::db::SubscriptionName {
                component0: &sub.component_0,
                component1: &sub.component_1,
                component2: &sub.component_2,
                component_id,
            },
            &dest_node_id,
            &link_id,
        ).await;

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

            // If the underlying DP connection is temporarily gone (e.g. the remote
            // peer restarted), the data-plane will reconnect on its own — the CP
            // must not interfere with that.  Leave the route in its current state
            // and return an error so the reconciler requeues and retries once the
            // connection is back.
            if !route.link_id.is_empty() && is_connection_not_found(&err_msg) {
                tracing::warn!(
                    "route reconciler: connection not found for route {} (link {}), will retry",
                    route.id,
                    route.link_id
                );
                return Err(Error::InvalidInput(format!(
                    "connection not found for route {} on node {node_id}: {err_msg}",
                    route.id
                )));
            }

            db.mark_route_failed(route.id, &err_msg).await?;
            tracing::error!(
                "route reconciler: marked route {} as failed: {err_msg}",
                route.id
            );
        }
    }

    // If some routes were deferred (link not yet applied on the dataplane side), the link
    // reconciler will re-enqueue this node when the link applies — no retry needed here.
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

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, ControlMessage, Subscription, control_message::Payload,
};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus, ResponseKind};
use crate::workqueue::WorkQueue;

pub struct RouteReconciler {
    db: SharedDb,
    cmd_handler: Arc<DefaultNodeCommandHandler>,
    queue: WorkQueue<String>,
    requeue_counts: Arc<Mutex<HashMap<String, usize>>>,
    max_requeues: usize,
    semaphore: Arc<Semaphore>,
}

impl RouteReconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: Arc<DefaultNodeCommandHandler>,
        queue: WorkQueue<String>,
        max_requeues: usize,
        max_parallel_reconciles: usize,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue,
            requeue_counts: Arc::new(Mutex::new(HashMap::new())),
            max_requeues,
            semaphore: Arc::new(Semaphore::new(max_parallel_reconciles)),
        }
    }

    pub async fn run(self) {
        tracing::info!("route reconciler: starting");

        while let Some(node_id) = self.queue.pop().await {
            let permit = match self.semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break, // semaphore closed
            };

            let db = self.db.clone();
            let cmd_handler = self.cmd_handler.clone();
            let queue = self.queue.clone();
            let requeue_counts = self.requeue_counts.clone();
            let max_requeues = self.max_requeues;

            tokio::spawn(async move {
                let _permit = permit; // released when this task completes

                if let Err(e) = handle_request(&db, &cmd_handler, &node_id).await {
                    tracing::error!("route reconciler: failed for node {node_id}: {e}");

                    let count = {
                        let mut counts = requeue_counts.lock().await;
                        let c = counts.entry(node_id.clone()).or_insert(0);
                        *c += 1;
                        *c
                    };

                    if count <= max_requeues {
                        tracing::debug!(
                            "route reconciler: requeuing node {node_id} (attempt {count}/{max_requeues})"
                        );
                        // Adding while in-flight marks dirty; done() will re-queue.
                        queue.add(node_id.clone());
                    } else {
                        tracing::warn!(
                            "route reconciler: dropping node {node_id} after {max_requeues} retries"
                        );
                        requeue_counts.lock().await.remove(&node_id);
                    }
                } else {
                    requeue_counts.lock().await.remove(&node_id);
                }

                queue.done(&node_id);
            });
        }

        tracing::info!("route reconciler: shutting down");
    }
}

async fn handle_request(
    db: &SharedDb,
    cmd_handler: &Arc<DefaultNodeCommandHandler>,
    node_id: &str,
) -> Result<(), String> {
    if cmd_handler.get_connection_status(node_id).await != NodeStatus::Connected {
        tracing::info!("route reconciler: node {node_id} not connected, skipping");
        return Ok(());
    }

    let routes = db.get_routes_for_node_id(node_id).await;

    let mut subscriptions_to_set: Vec<Subscription> = Vec::new();
    let mut subscriptions_to_delete: Vec<Subscription> = Vec::new();
    let mut needs_requeue = false;

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
                    "route reconciler: waiting for link {link_id} to apply before sending route"
                );
                needs_requeue = true;
                continue;
            }
            _ => {}
        }

        subscriptions_to_set.push(sub);
    }

    if subscriptions_to_set.is_empty() && subscriptions_to_delete.is_empty() {
        if needs_requeue {
            return Err("route reconciliation deferred: waiting for link(s) to apply".to_string());
        }
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
        .await
        .map_err(|e| format!("failed to send config command to node {node_id}: {e}"))?;

    let response = cmd_handler
        .wait_for_response(node_id, ResponseKind::ConfigCommandAck, &message_id)
        .await?;

    let ack = match response.payload {
        Some(Payload::ConfigCommandAck(a)) => a,
        _ => return Err(format!("received unexpected response from node {node_id}")),
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

            db.mark_route_failed(route.id, &err_msg).await?;
            tracing::error!(
                "route reconciler: marked route {} as failed: {err_msg}",
                route.id
            );

            if should_retry_missing_link(&err_msg) {
                needs_requeue = true;
            }
        }
    }

    if needs_requeue {
        return Err("route reconciliation deferred: waiting for link(s) to apply".to_string());
    }

    Ok(())
}

fn should_retry_missing_link(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("connection with link_id") && lower.contains("not found")
}

fn is_subscription_not_found(msg: &str) -> bool {
    msg.to_lowercase().contains("subscription not found")
}

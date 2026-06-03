// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use parking_lot::Mutex;

use crate::backoff;
use crate::config::{ReconcilerConfig, TopologyConfig};
use crate::error::{Error, Result};

use crate::api::proto::controller::proto::v1::{
    ConfigurationCommand, Connection, ControlMessage, Route, control_message::Payload,
};
use crate::db::{LinkStatus, RouteStatus, SharedDb};
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus, ResponseKind};
use crate::workqueue::WorkQueue;
use slim_datapath::api::{NameId, ProtoName};

use super::is_connection_not_found;

#[derive(Clone)]
pub struct Reconciler {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    queue: WorkQueue<String>,
    max_requeues: usize,
    base_retry_delay: Duration,
    requeue_counts: Arc<Mutex<HashMap<String, usize>>>,
    topology: TopologyConfig,
}

impl Reconciler {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        queue: WorkQueue<String>,
        config: ReconcilerConfig,
        topology: TopologyConfig,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            queue,
            max_requeues: config.max_requeues,
            base_retry_delay: config.base_retry_delay.into(),
            requeue_counts: Arc::new(Mutex::new(HashMap::new())),
            topology,
        }
    }

    pub async fn run(self) {
        tracing::info!("reconciler: starting");

        while let Some(node_id) = self.queue.pop().await {
            if let Err(e) = handle_request(&self.db, &self.cmd_handler, &self.queue, &node_id, &self.topology).await
            {
                tracing::error!("reconciler: failed for node {node_id}: {e}");

                let count = {
                    let mut counts = self.requeue_counts.lock();
                    let c = counts.entry(node_id.clone()).or_insert(0);
                    *c += 1;
                    *c
                };

                if count <= self.max_requeues {
                    let delay = backoff::backoff_delay(count, self.base_retry_delay);
                    tracing::debug!(
                        "reconciler: requeuing node {node_id} in {delay:?} (attempt {count}/{})",
                        self.max_requeues
                    );
                    self.queue.add_after(node_id.clone(), delay);
                } else {
                    tracing::warn!(
                        "reconciler: dropping node {node_id} after {} retries",
                        self.max_requeues
                    );
                    self.requeue_counts.lock().remove(&node_id);
                }
            } else {
                self.requeue_counts.lock().remove(&node_id);
            }

            self.queue.done(&node_id);
        }

        tracing::info!("reconciler: shutting down");
    }
}

// ── Main reconciliation logic ───────────────────────────────────────────────

type SubKey = (String, String, String, Option<String>, String);

async fn handle_request(
    db: &SharedDb,
    cmd_handler: &DefaultNodeCommandHandler,
    queue: &WorkQueue<String>,
    node_id: &str,
    topology: &TopologyConfig,
) -> Result<()> {
    if cmd_handler.get_connection_status(node_id).await != NodeStatus::Connected {
        tracing::info!("reconciler: node {node_id} not connected, skipping");
        return Ok(());
    }

    let mut links = db.get_links_for_node(node_id).await?;
    let routes = db.get_routes_for_node(node_id).await?;

    let (desired_connections, desired_link_ids, deleted_links) =
        build_desired_connections(&mut links, node_id)?;
    let (desired_routes, included_routes, mut needs_requeue) =
        build_desired_routes(db, &routes, node_id, topology).await?;

    if desired_connections.is_empty() && desired_routes.is_empty() && deleted_links.is_empty() {
        if needs_requeue {
            queue.add_after(node_id.to_string(), Duration::from_secs(5));
        }
        return Ok(());
    }

    tracing::info!(
        "reconciler: sending desired state to node {node_id}: {} connections, {} routes",
        desired_connections.len(),
        desired_routes.len(),
    );

    let msg = ControlMessage {
        message_id: Uuid::new_v4().to_string(),
        payload: Some(Payload::ConfigCommand(ConfigurationCommand {
            connections_to_create: desired_connections,
            routes_to_set: desired_routes,
            routes_to_delete: vec![],
            connections_to_delete: vec![],
            reconcile: true,
        })),
    };

    let responses = cmd_handler
        .send_and_wait(node_id, msg, ResponseKind::ConfigCommandAck)
        .await?;

    let ack = match responses.into_iter().next().and_then(|r| r.payload) {
        Some(Payload::ConfigCommandAck(a)) => a,
        _ => {
            return Err(Error::UnexpectedResponse(format!(
                "received unexpected response from node {node_id}"
            )));
        }
    };

    let enqueue_nodes =
        process_connection_acks(db, &ack, &links, &desired_link_ids, node_id).await?;

    for link in &deleted_links {
        db.delete_link(link).await?;
        tracing::info!("reconciler: deleted link record for {}", link.link_id);
    }

    if process_route_acks(db, &ack, &included_routes, node_id).await? {
        needs_requeue = true;
    }

    for nid in &enqueue_nodes {
        if *nid != node_id {
            tracing::debug!("reconciler: enqueuing reconciliation for node {nid}");
            queue.add(nid.clone());
        }
    }

    if needs_requeue {
        queue.add_after(node_id.to_string(), Duration::from_secs(5));
    }

    Ok(())
}

// ── Build desired connections from links ───────────────────────────────────

fn build_desired_connections(
    links: &mut [crate::db::Link],
    node_id: &str,
) -> Result<(Vec<Connection>, HashSet<String>, Vec<crate::db::Link>)> {
    let mut desired_connections: Vec<Connection> = Vec::new();
    let mut desired_link_ids: HashSet<String> = HashSet::new();
    let mut deleted_links: Vec<crate::db::Link> = Vec::new();

    for link in links {
        if link.source_node_id != node_id {
            continue;
        }
        if link.status == LinkStatus::Deleted {
            deleted_links.push(link.clone());
            continue;
        }
        if link.link_id.is_empty() {
            continue;
        }
        desired_link_ids.insert(link.link_id.clone());
        let config = &mut link.conn_config_data;
        config.link_id = link.link_id.clone();
        let config_data = serde_json::to_string(config)?;
        desired_connections.push(Connection {
            link_id: link.link_id.clone(),
            config_data,
        });
    }

    Ok((desired_connections, desired_link_ids, deleted_links))
}

// ── Build desired routes ───────────────────────────────────────────────────

async fn build_desired_routes<'a>(
    db: &SharedDb,
    routes: &'a [crate::db::Route],
    _node_id: &str,
    topology: &TopologyConfig,
) -> Result<(Vec<Route>, HashMap<SubKey, &'a crate::db::Route>, bool)> {
    let mut desired_routes: Vec<Route> = Vec::new();
    let mut included_routes: HashMap<SubKey, &crate::db::Route> = HashMap::new();
    let mut needs_requeue = false;

    for route in routes {
        if route.status == RouteStatus::Deleted {
            continue;
        }

        let link_id = match route.link_id.as_deref() {
            Some(id) => id,
            None => {
                // No link_id yet, try to find a direct link in the database.
                match db
                    .find_link_between_nodes(&route.source_node_id, &route.dest_node_id)
                    .await?
                {
                    Some(l) if l.status != LinkStatus::Deleted => {
                        if let Err(e) = db.update_route_link_id(&route.id, &l.link_id).await {
                            tracing::warn!(
                                "reconciler: failed to update route {} link_id: {e}",
                                route.id
                            );
                        }
                        needs_requeue = true;
                    }
                    _ => {
                        // No direct link. In star topology, try to resolve the hub
                        // node as the next-hop for transit routing.
                        if topology.is_star() {
                            if let Some(hub_link) = resolve_hub_link(db, topology, &route.source_node_id).await {
                                if let Err(e) = db.update_route_link_id(&route.id, &hub_link).await {
                                    tracing::warn!(
                                        "reconciler: failed to update route {} hub link_id: {e}",
                                        route.id
                                    );
                                }
                                needs_requeue = true;
                            } else {
                                tracing::debug!(
                                    "reconciler: no hub link for route {} ({}→{}), deferring",
                                    route.id,
                                    route.source_node_id,
                                    route.dest_node_id
                                );
                            }
                        } else {
                            tracing::debug!(
                                "reconciler: no link yet for route {} ({}→{}), deferring",
                                route.id,
                                route.source_node_id,
                                route.dest_node_id
                            );
                        }
                    }
                }
                continue;
            }
        };

        let link_lookup = db
            .get_link(link_id, &route.source_node_id, &route.dest_node_id)
            .await?
            .or(db
                .get_link(link_id, &route.dest_node_id, &route.source_node_id)
                .await?);
        // In star topology, the link may be between source and hub (transit),
        // not between source and dest. Try that lookup as well.
        let link_lookup = match link_lookup {
            Some(l) => Some(l),
            None if topology.is_star() => {
                if let Some(hub_group) = topology.hub_group() {
                    let all_nodes = db.list_nodes().await?;
                    if let Some(hub_id) = all_nodes
                        .iter()
                        .find(|n| n.group_name.as_deref() == Some(hub_group))
                        .map(|n| n.id.as_str())
                    {
                        db.get_link(link_id, &route.source_node_id, hub_id)
                            .await?
                            .or(db.get_link(link_id, hub_id, &route.source_node_id).await?)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            None => None,
        };
        if let Some(l) = link_lookup {
            if l.status == LinkStatus::Failed {
                let msg = if l.status_msg.is_empty() {
                    "link configuration failed"
                } else {
                    &l.status_msg
                };
                if let Err(e) = db.mark_route_failed(&route.id, msg).await {
                    tracing::warn!(
                        "reconciler: failed to mark route {} as failed: {e}",
                        route.id
                    );
                }
                continue;
            }
        } else {
            tracing::warn!(
                "reconciler: skipping route {} — link {link_id} not found",
                route.id
            );
            continue;
        }

        let sub = Route {
            name: Some(
                ProtoName::from_strings([&route.component0, &route.component1, &route.component2])
                    .with_id(
                        route
                            .component_id
                            .as_deref()
                            .and_then(|s| uuid::Uuid::parse_str(s).ok())
                            .map(|u| u.as_u128())
                            .unwrap_or(NameId::NULL_COMPONENT),
                    ),
            ),
            link_id: Some(link_id.to_string()),
            ..Default::default()
        };

        let key: SubKey = (
            route.component0.clone(),
            route.component1.clone(),
            route.component2.clone(),
            route.component_id.clone(),
            link_id.to_string(),
        );
        included_routes.insert(key, route);
        desired_routes.push(sub);
    }

    Ok((desired_routes, included_routes, needs_requeue))
}

// ── Process connection ACKs ────────────────────────────────────────────────

async fn process_connection_acks(
    db: &SharedDb,
    ack: &crate::api::proto::controller::proto::v1::ConfigurationCommandAck,
    links: &[crate::db::Link],
    desired_link_ids: &HashSet<String>,
    node_id: &str,
) -> Result<HashSet<String>> {
    let mut enqueue_nodes: HashSet<String> = HashSet::new();

    let links_by_id: HashMap<&str, &crate::db::Link> = links
        .iter()
        .filter(|l| l.source_node_id == node_id && desired_link_ids.contains(&l.link_id))
        .map(|l| (l.link_id.as_str(), l))
        .collect();

    for conn_ack in &ack.connections_status {
        let link = match links_by_id.get(conn_ack.link_id.as_str()) {
            Some(l) => *l,
            None => continue,
        };

        let mut updated = link.clone();
        if conn_ack.success {
            updated.status = LinkStatus::Applied;
            updated.status_msg = String::new();
            tracing::info!(
                "reconciler: link {} ({}→{}) applied",
                link.link_id,
                link.source_node_id,
                link.dest_node_id
            );
            enqueue_nodes.insert(link.dest_node_id.clone());
            for r in db.get_routes_by_link_id(&link.link_id).await? {
                enqueue_nodes.insert(r.source_node_id.clone());
            }
        } else {
            updated.status = LinkStatus::Failed;
            updated.status_msg = conn_ack.error_msg.clone();
            tracing::warn!(
                "reconciler: link {} ({}→{}) failed: {}",
                link.link_id,
                link.source_node_id,
                link.dest_node_id,
                conn_ack.error_msg
            );
        }
        db.update_link(updated).await?;
    }

    Ok(enqueue_nodes)
}

// ── Process route ACKs ─────────────────────────────────────────────────────

async fn process_route_acks(
    db: &SharedDb,
    ack: &crate::api::proto::controller::proto::v1::ConfigurationCommandAck,
    included_routes: &HashMap<SubKey, &crate::db::Route>,
    node_id: &str,
) -> Result<bool> {
    let mut needs_requeue = false;

    for route_ack in &ack.routes_status {
        let sub = match &route_ack.route {
            Some(s) => s,
            None => continue,
        };

        let link_id = sub.link_id.clone().unwrap_or_default();
        let proto_name = sub.name.as_ref().unwrap();
        let (c0, c1, c2) = proto_name.str_components();
        let comp_id = if proto_name.id() == NameId::NULL_COMPONENT {
            None
        } else {
            Some(proto_name.string_id())
        };

        let key: SubKey = (
            c0.to_string(),
            c1.to_string(),
            c2.to_string(),
            comp_id.clone(),
            link_id.clone(),
        );

        let route = match included_routes.get(&key) {
            Some(r) => (*r).clone(),
            None => {
                match db
                    .get_route_for_src_dest_name(
                        node_id,
                        &crate::db::RouteName {
                            component0: c0,
                            component1: c1,
                            component2: c2,
                            component_id: comp_id.as_deref(),
                        },
                        "",
                        Some(&link_id),
                    )
                    .await?
                {
                    Some(r) => r,
                    None => {
                        tracing::warn!("reconciler: no route found for route ack: {:?}", sub);
                        continue;
                    }
                }
            }
        };

        if route_ack.success {
            db.mark_route_applied(&route.id).await?;
            tracing::debug!("reconciler: marked route {} as applied", route.id);
        } else {
            let err_msg = route_ack.error_msg.clone();

            if route.link_id.is_some() && is_connection_not_found(&err_msg) {
                tracing::warn!(
                    "reconciler: connection not found for route {} (link {:?}) — requeuing",
                    route.id,
                    route.link_id
                );
                needs_requeue = true;
                continue;
            }

            db.mark_route_failed(&route.id, &err_msg).await?;
            tracing::error!("reconciler: marked route {} as failed: {err_msg}", route.id);
        }
    }

    Ok(needs_requeue)
}

/// In a star topology, finds the link from `source_node_id` to the hub node.
/// Used by the reconciler and route_service to resolve the next-hop for transit
/// routes where source and destination have no direct link.
pub(crate) async fn resolve_hub_link(
    db: &SharedDb,
    topology: &TopologyConfig,
    source_node_id: &str,
) -> Option<String> {
    let hub_group = topology.hub_group()?;

    // Find the hub node (single node per group).
    let all_nodes = db.list_nodes().await.ok()?;
    let hub_node_id = all_nodes
        .iter()
        .find(|n| n.group_name.as_deref() == Some(hub_group))
        .map(|n| n.id.as_str())?;

    // Find the link between the source node and the hub node.
    let link = db
        .find_link_between_nodes(source_node_id, hub_node_id)
        .await
        .ok()??;
    if link.status != LinkStatus::Deleted {
        Some(link.link_id)
    } else {
        None
    }
}

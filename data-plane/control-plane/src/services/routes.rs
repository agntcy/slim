// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod link_reconciler;
pub mod route_reconciler;

use std::sync::Arc;
use std::time::SystemTime;

use serde_json::json;
use uuid::Uuid;

use crate::workqueue::WorkQueue;

use crate::api::proto::controller::proto::v1::{
    ConnectionListResponse, ControlMessage, SubscriptionListResponse, control_message::Payload,
};
use crate::db::{LinkStatus, RouteStatus, SharedDb, SubscriptionName};
use crate::node_control::{DefaultNodeCommandHandler, ResponseKind};

pub const ALL_NODES_ID: &str = crate::db::ALL_NODES_ID;

/// A lightweight route descriptor used by the service layer.
#[derive(Debug, Clone)]
pub struct Route {
    pub source_node_id: String,
    pub dest_node_id: String,
    pub link_id: String,
    pub component0: String,
    pub component1: String,
    pub component2: String,
    pub component_id: Option<u64>,
}

pub struct ReconcilerConfig {
    pub max_requeues: usize,
    pub max_parallel_reconciles: usize,
}

pub struct RouteService {
    db: SharedDb,
    cmd_handler: Arc<DefaultNodeCommandHandler>,
    reconciler_config: ReconcilerConfig,
    /// Work queue for route reconciliation.
    route_queue: WorkQueue<String>,
    /// Work queue for link reconciliation.
    link_queue: WorkQueue<String>,
}

impl RouteService {
    pub fn new(
        db: SharedDb,
        cmd_handler: Arc<DefaultNodeCommandHandler>,
        reconciler_config: ReconcilerConfig,
    ) -> Arc<Self> {
        let route_queue: WorkQueue<String> = WorkQueue::new();
        let link_queue: WorkQueue<String> = WorkQueue::new();

        let svc = Arc::new(Self {
            db: db.clone(),
            cmd_handler: cmd_handler.clone(),
            reconciler_config,
            route_queue: route_queue.clone(),
            link_queue: link_queue.clone(),
        });

        // Spawn reconciler workers.
        let route_reconciler = route_reconciler::RouteReconciler::new(
            db.clone(),
            cmd_handler.clone(),
            route_queue,
            svc.reconciler_config.max_requeues,
            svc.reconciler_config.max_parallel_reconciles,
        );
        let link_reconciler = link_reconciler::LinkReconciler::new(
            db,
            cmd_handler,
            link_queue,
            svc.route_queue.clone(),
            ReconcilerConfig {
                max_requeues: svc.reconciler_config.max_requeues,
                max_parallel_reconciles: svc.reconciler_config.max_parallel_reconciles,
            },
        );
        tokio::spawn(async move { route_reconciler.run().await });
        tokio::spawn(async move { link_reconciler.run().await });

        svc
    }

    /// Stop the reconciler workers and wait for any in-flight reconciliations
    /// to finish before returning.
    pub async fn shutdown(&self) {
        tracing::info!("route service: shutting down reconcilers");
        tokio::join!(
            self.route_queue.shutdown_with_drain(),
            self.link_queue.shutdown_with_drain(),
        );
        tracing::info!("route service: reconcilers stopped");
    }

    pub async fn add_route(&self, route: Route) -> Result<String, String> {
        if route.source_node_id.is_empty() {
            return Err("source node ID cannot be empty".to_string());
        }
        if route.dest_node_id.is_empty() {
            return Err("destination node ID cannot be empty".to_string());
        }
        if route.source_node_id == route.dest_node_id {
            return Err("destination node ID cannot be the same as source node ID".to_string());
        }

        let db_route = crate::db::Route {
            id: 0,
            source_node_id: route.source_node_id.clone(),
            dest_node_id: route.dest_node_id.clone(),
            link_id: String::new(),
            component0: route.component0.clone(),
            component1: route.component1.clone(),
            component2: route.component2.clone(),
            component_id: route.component_id.map(|v| v as i64),
            status: if route.source_node_id != ALL_NODES_ID {
                RouteStatus::Pending
            } else {
                RouteStatus::Applied
            },
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        };

        let route_id = self.add_single_route(db_route.clone()).await?;

        // For wildcard source, create per-node routes for all existing nodes.
        if route.source_node_id == ALL_NODES_ID {
            let all_nodes = self.db.list_nodes().await;
            for n in all_nodes {
                if n.id == route.dest_node_id {
                    continue;
                }
                let link_id = self
                    .find_matching_link(&n.id, &route.dest_node_id)
                    .await
                    .unwrap_or_default();
                let per_node = crate::db::Route {
                    id: 0,
                    source_node_id: n.id.clone(),
                    dest_node_id: route.dest_node_id.clone(),
                    link_id,
                    component0: route.component0.clone(),
                    component1: route.component1.clone(),
                    component2: route.component2.clone(),
                    component_id: route.component_id.map(|v| v as i64),
                    status: RouteStatus::Pending,
                    status_msg: String::new(),
                    deleted: false,
                    last_updated: SystemTime::now(),
                };
                let _ = self.add_single_route(per_node).await;
            }
        }

        Ok(route_id)
    }

    async fn add_single_route(&self, mut db_route: crate::db::Route) -> Result<String, String> {
        if db_route.source_node_id != ALL_NODES_ID {
            db_route.link_id = self
                .find_matching_link(&db_route.source_node_id, &db_route.dest_node_id)
                .await
                .map_err(|e| format!("failed to find matching link: {e}"))?;
        }

        let route_str;
        match self.db.add_route(db_route.clone()).await {
            Ok(r) => {
                route_str = r.to_string();
                tracing::info!("route added: {route_str}");
            }
            Err(e) => {
                // If the route already exists and is marked deleted, clean it up and retry.
                let unique_id = db_route.compute_id();
                if let Some(existing) = self.db.get_route_by_id(unique_id).await {
                    if existing.deleted {
                        tracing::warn!("removing stale deleted route {} to allow re-add", existing);
                        self.db.delete_route(existing.id).await?;
                        let r = self.db.add_route(db_route.clone()).await?;
                        route_str = r.to_string();
                    } else {
                        return Err(format!("failed to add route: {e}"));
                    }
                } else {
                    return Err(format!("failed to add route: {e}"));
                }
            }
        }

        if db_route.source_node_id != ALL_NODES_ID {
            self.route_queue.add(db_route.source_node_id);
        }
        Ok(route_str)
    }

    pub async fn delete_route(&self, route: Route) -> Result<(), String> {
        if route.dest_node_id.is_empty() {
            return Err("destNodeID must be set".to_string());
        }

        if route.source_node_id == ALL_NODES_ID {
            // Delete the wildcard route itself.
            let db_route = self
                .db
                .get_route_for_src_dest_name(
                    &route.source_node_id,
                    &SubscriptionName {
                        component0: &route.component0,
                        component1: &route.component1,
                        component2: &route.component2,
                        component_id: route.component_id.map(|v| v as i64),
                    },
                    &route.dest_node_id,
                    "",
                )
                .await
                .ok_or_else(|| "route not found".to_string())?;
            self.db.delete_route(db_route.id).await?;

            // Also delete all per-node expansions.
            let per_node = self.db.get_routes_for_dest_node_id_and_name(
                &route.dest_node_id,
                &route.component0,
                &route.component1,
                &route.component2,
                route.component_id.map(|v| v as i64),
            ).await;
            for r in per_node {
                self.delete_single_route(&r.source_node_id, r.id, &r.to_string())
                    .await?;
            }
            return Ok(());
        }

        let link_id = if route.link_id.is_empty() {
            self.find_matching_link(&route.source_node_id, &route.dest_node_id).await?
        } else {
            route.link_id.clone()
        };

        let db_route = self
            .db
            .get_route_for_src_dest_name(
                &route.source_node_id,
                &SubscriptionName {
                    component0: &route.component0,
                    component1: &route.component1,
                    component2: &route.component2,
                    component_id: route.component_id.map(|v| v as i64),
                },
                &route.dest_node_id,
                &link_id,
            )
            .await
            .ok_or_else(|| "route not found".to_string())?;

        self.delete_single_route(&route.source_node_id, db_route.id, &db_route.to_string())
            .await
    }

    async fn delete_single_route(
        &self,
        node_id: &str,
        route_id: i64,
        route_key: &str,
    ) -> Result<(), String> {
        self.db.mark_route_deleted(route_id).await?;
        tracing::info!("route marked for delete: {route_key}");
        if node_id != ALL_NODES_ID {
            self.route_queue.add(node_id.to_string());
        }
        Ok(())
    }

    /// Called when a new node registers. Creates links/routes and triggers reconciliation.
    pub async fn node_registered(&self, node_id: &str, conn_details_updated: bool) {
        let mut reconcile_link_for_nodes: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        if conn_details_updated {
            for nid in self.reconnect_existing_links(node_id).await {
                reconcile_link_for_nodes.insert(nid);
            }
        }
        for nid in self.ensure_links_for_node(node_id).await {
            reconcile_link_for_nodes.insert(nid);
        }
        self.ensure_routes_for_node(node_id).await;
        reconcile_link_for_nodes.insert(node_id.to_string());

        for nid in reconcile_link_for_nodes {
            self.link_queue.add(nid);
        }
    }

    /// Re-add links that point to a reconnecting node with potentially new
    /// connection details, and repoint existing routes to the new links.
    async fn reconnect_existing_links(&self, node_id: &str) -> Vec<String> {
        let mut affected: std::collections::HashSet<String> =
            [node_id.to_string()].into_iter().collect();
        for link in self.db.get_links_for_node(node_id).await {
            if link.deleted || link.dest_node_id != node_id {
                continue;
            }
            let (endpoint, config_data) = match self
                .get_connection_details(&link.source_node_id, node_id)
                .await
            {
                Ok(cd) => cd,
                Err(e) => {
                    tracing::error!(
                        "reconnect: failed to get connection details for {} -> {}: {e}",
                        link.source_node_id,
                        node_id
                    );
                    continue;
                }
            };
            let replacement = match self.db.add_link(crate::db::Link {
                link_id: Uuid::new_v4().to_string(),
                source_node_id: link.source_node_id.clone(),
                dest_node_id: link.dest_node_id.clone(),
                dest_endpoint: endpoint,
                conn_config_data: config_data,
                status: LinkStatus::Pending,
                status_msg: String::new(),
                deleted: false,
                last_updated: SystemTime::now(),
            }).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("reconnect: failed to add replacement link: {e}");
                    continue;
                }
            };

            affected.insert(replacement.source_node_id.clone());

            for r in self.db.get_routes_by_link_id(&link.link_id).await {
                if let Err(e) = self.db.repoint_route(
                    r.id,
                    &replacement.link_id,
                    RouteStatus::Pending,
                    "waiting for replacement link apply",
                ).await {
                    tracing::error!("reconnect: failed to repoint route {}: {e}", r);
                } else {
                    affected.insert(r.source_node_id.clone());
                }
            }

            let mut old_link = link;
            old_link.deleted = true;
            old_link.status_msg = "marked deleted after replacement link creation".to_string();
            if let Err(e) = self.db.update_link(old_link).await {
                tracing::error!("reconnect: failed to mark old link deleted: {e}");
            }
        }
        affected.into_iter().collect()
    }

    /// Ensure direct or group links exist between `node_id` and every other node.
    async fn ensure_links_for_node(&self, node_id: &str) -> Vec<String> {
        let src_node = match self.db.get_node(node_id).await {
            Some(n) => n,
            None => {
                tracing::error!("ensure_links: node {node_id} not found");
                return vec![node_id.to_string()];
            }
        };
        let mut affected: std::collections::HashSet<String> =
            [node_id.to_string()].into_iter().collect();

        for other in self.db.list_nodes().await {
            if other.id == node_id {
                continue;
            }
            if self
                .db
                .find_link_between_nodes(node_id, &other.id)
                .await
                .map(|l| !l.deleted)
                .unwrap_or(false)
            {
                continue;
            }
            let same_group = src_node.group_name == other.group_name;
            if same_group {
                if let Some(src) = self.ensure_direct_link(node_id, &other.id).await {
                    affected.insert(src);
                }
                continue;
            }
            let has_dst_external = other.conn_details.iter().any(|d| {
                d.external_endpoint
                    .as_deref()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
            });
            if has_dst_external {
                if let Some(src) = self.ensure_group_link(node_id, &other.id).await {
                    affected.insert(src);
                }
                continue;
            }
            let has_src_external = src_node.conn_details.iter().any(|d| {
                d.external_endpoint
                    .as_deref()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
            });
            if has_src_external {
                if let Some(src) = self.ensure_group_link(&other.id, node_id).await {
                    affected.insert(src);
                }
                continue;
            }
            tracing::error!(
                "cannot create link between {node_id} and {}: no external endpoint available",
                other.id
            );
        }
        affected.into_iter().collect()
    }

    async fn ensure_direct_link(&self, source_node_id: &str, dest_node_id: &str) -> Option<String> {
        match self
            .get_connection_details(source_node_id, dest_node_id)
            .await
        {
            Ok((endpoint, config_data)) => {
                let link_id = Uuid::new_v4().to_string();
                if let Err(e) = self.db.add_link(crate::db::Link {
                    link_id,
                    source_node_id: source_node_id.to_string(),
                    dest_node_id: dest_node_id.to_string(),
                    dest_endpoint: endpoint,
                    conn_config_data: config_data,
                    status: LinkStatus::Pending,
                    status_msg: String::new(),
                    deleted: false,
                    last_updated: SystemTime::now(),
                }).await {
                    tracing::error!(
                        "ensure_direct_link: failed to add link {source_node_id}->{dest_node_id}: {e}"
                    );
                    return None;
                }
                Some(source_node_id.to_string())
            }
            Err(e) => {
                tracing::error!("ensure_direct_link: failed to get connection details: {e}");
                None
            }
        }
    }

    async fn ensure_group_link(&self, source_node_id: &str, dest_node_id: &str) -> Option<String> {
        let (endpoint, config_data) = match self
            .get_connection_details(source_node_id, dest_node_id)
            .await
        {
            Ok(cd) => cd,
            Err(e) => {
                tracing::error!("ensure_group_link: failed to get connection details: {e}");
                return None;
            }
        };

        // Reuse an existing link with the same source + endpoint.
        let link_id = self
            .db
            .get_link_for_source_and_endpoint(source_node_id, &endpoint)
            .await
            .map(|l| l.link_id)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        if let Err(e) = self.db.add_link(crate::db::Link {
            link_id,
            source_node_id: source_node_id.to_string(),
            dest_node_id: dest_node_id.to_string(),
            dest_endpoint: endpoint,
            conn_config_data: config_data,
            status: LinkStatus::Pending,
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        }).await {
            tracing::error!(
                "ensure_group_link: failed to add link {source_node_id}->{dest_node_id}: {e}"
            );
            return None;
        }
        Some(source_node_id.to_string())
    }

    /// For each wildcard route, create a per-node route for `node_id` if one
    /// does not exist yet.
    async fn ensure_routes_for_node(&self, node_id: &str) {
        for r in self.db.get_routes_for_node_id(ALL_NODES_ID).await {
            if r.dest_node_id == node_id {
                continue;
            }
            if self
                .db
                .get_route_for_src_dest_name(
                    node_id,
                    &SubscriptionName {
                        component0: &r.component0,
                        component1: &r.component1,
                        component2: &r.component2,
                        component_id: r.component_id,
                    },
                    &r.dest_node_id,
                    "",
                )
                .await
                .is_some()
            {
                continue;
            }
            let link_id = match self.find_matching_link(node_id, &r.dest_node_id).await {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(
                        "ensure_routes: failed to find link for {node_id}->{}: {e}",
                        r.dest_node_id
                    );
                    continue;
                }
            };
            let new_route = crate::db::Route {
                id: 0,
                source_node_id: node_id.to_string(),
                dest_node_id: r.dest_node_id.clone(),
                link_id,
                component0: r.component0.clone(),
                component1: r.component1.clone(),
                component2: r.component2.clone(),
                component_id: r.component_id,
                status: RouteStatus::Pending,
                status_msg: String::new(),
                deleted: false,
                last_updated: SystemTime::now(),
            };
            match self.db.add_route(new_route).await {
                Ok(added) => tracing::info!("generic route created: {added}"),
                Err(e) => tracing::debug!("generic route already exists or cannot be added: {e}"),
            }
        }
    }

    async fn find_matching_link(&self, source: &str, dest: &str) -> Result<String, String> {
        match self.db.find_link_between_nodes(source, dest).await {
            Some(l) if !l.deleted => Ok(l.link_id),
            _ => Err(format!(
                "no matching link found for source={source} destination={dest}"
            )),
        }
    }

    /// Re-queue reconciliation for `node_id` as source when a subscription loss
    /// is reported by the data plane.
    pub async fn requeue_route_for_source_node(&self, node_id: &str, route: Route) {
        let db_route = match self
            .db
            .get_route_for_src_dest_name(
                node_id,
                &SubscriptionName {
                    component0: &route.component0,
                    component1: &route.component1,
                    component2: &route.component2,
                    component_id: route.component_id.map(|v| v as i64),
                },
                "",
                "",
            )
            .await
        {
            Some(r) => r,
            None => return,
        };

        tracing::info!("re-queuing route reconciliation for {node_id} after subscription loss");

        if !db_route.link_id.is_empty()
            && let Some(link) = self
                .db
                .get_link(
                    &db_route.link_id,
                    &db_route.source_node_id,
                    &db_route.dest_node_id,
                )
                .await
            && !link.deleted
            && link.status == LinkStatus::Applied
        {
            let mut updated_link = link;
            updated_link.status = LinkStatus::Pending;
            updated_link.status_msg = "reset after subscription loss on consumer node".to_string();
            if let Err(e) = self.db.update_link(updated_link).await {
                tracing::error!("failed to reset link to pending after subscription loss: {e}");
            } else {
                self.link_queue.add(db_route.source_node_id.clone());
            }
        }
        self.route_queue.add(node_id.to_string());
    }

    pub async fn list_subscriptions(
        &self,
        node_id: &str,
    ) -> Result<SubscriptionListResponse, String> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::SubscriptionListRequest(
                crate::api::proto::controller::proto::v1::SubscriptionListRequest {},
            )),
        };
        self.cmd_handler
            .send_message(node_id, msg)
            .await
            .map_err(|e| format!("failed to send message: {e}"))?;
        let resp = self
            .cmd_handler
            .wait_for_response(node_id, ResponseKind::SubscriptionListResponse, &message_id)
            .await?;
        match resp.payload {
            Some(Payload::SubscriptionListResponse(r)) => Ok(r),
            _ => Err("no SubscriptionListResponse received".to_string()),
        }
    }

    pub async fn list_connections(&self, node_id: &str) -> Result<ConnectionListResponse, String> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::ConnectionListRequest(
                crate::api::proto::controller::proto::v1::ConnectionListRequest {},
            )),
        };
        self.cmd_handler
            .send_message(node_id, msg)
            .await
            .map_err(|e| format!("failed to send message: {e}"))?;
        let resp = self
            .cmd_handler
            .wait_for_response(node_id, ResponseKind::ConnectionListResponse, &message_id)
            .await?;
        match resp.payload {
            Some(Payload::ConnectionListResponse(r)) => Ok(r),
            _ => Err("no ConnectionListResponse received".to_string()),
        }
    }

    /// Compute the effective endpoint and serialised JSON config data for a
    /// link from `source_node_id` to `dest_node_id`.
    pub async fn get_connection_details(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<(String, String), String> {
        let dest_node = self
            .db
            .get_node(dest_node_id)
            .await
            .ok_or_else(|| format!("destination node {dest_node_id} not found"))?;
        if dest_node.conn_details.is_empty() {
            return Err(format!(
                "no connections for destination node {dest_node_id}"
            ));
        }
        let src_node = self
            .db
            .get_node(source_node_id)
            .await
            .ok_or_else(|| format!("source node {source_node_id} not found"))?;

        let (conn, local_connection) = select_connection(&dest_node, &src_node);

        generate_config_data(conn, local_connection, &dest_node, &src_node)
    }
}

/// Select the best connection detail from `dst_node` relative to `src_node`.
fn select_connection<'a>(
    dst_node: &'a crate::db::Node,
    src_node: &crate::db::Node,
) -> (&'a crate::db::ConnectionDetails, bool) {
    let same_group = dst_node.group_name == src_node.group_name;
    if same_group {
        return (&dst_node.conn_details[0], true);
    }
    for conn in &dst_node.conn_details {
        if conn
            .external_endpoint
            .as_deref()
            .map(|e| !e.is_empty())
            .unwrap_or(false)
        {
            return (conn, false);
        }
    }
    (&dst_node.conn_details[0], false)
}

/// Generate the serialised JSON connection config data for a link.
fn generate_config_data(
    detail: &crate::db::ConnectionDetails,
    local_connection: bool,
    dest_node: &crate::db::Node,
    src_node: &crate::db::Node,
) -> Result<(String, String), String> {
    // Start with client_config as a mutable JSON object.
    let mut config: serde_json::Map<String, serde_json::Value> = match &detail.client_config {
        serde_json::Value::Object(m) => m.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            return Err(format!(
                "unexpected client_config type: expected object, got {}",
                other
            ));
        }
    };

    // Set a default backoff if not present.
    if !config.contains_key("backoff") {
        config.insert(
            "backoff".to_string(),
            json!({
                "type": "fixed_interval",
                "interval": "2000ms"
            }),
        );
    }

    // Determine effective endpoint.
    let endpoint = if local_connection {
        detail.endpoint.clone()
    } else {
        detail
            .external_endpoint
            .clone()
            .filter(|e| !e.is_empty())
            .ok_or_else(|| format!("no external endpoint for connection {}", detail.endpoint))?
    };

    // Set TLS / endpoint scheme.
    if config.contains_key("tls") {
        config.insert("endpoint".to_string(), json!(format!("https://{endpoint}")));
    } else if !detail.mtls_required {
        config.insert("endpoint".to_string(), json!(format!("http://{endpoint}")));
        config.insert("tls".to_string(), json!({ "insecure": true }));
    } else {
        // MTLS via SPIRE.
        let spire_socket = src_node
            .conn_details
            .iter()
            .find_map(|cd| {
                cd.client_config
                    .get("tls")
                    .and_then(|t| t.get("source"))
                    .and_then(|s| s.get("socket_path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .ok_or_else(|| format!("no SPIRE socket path found for source node {}", src_node.id))?;

        config.insert("endpoint".to_string(), json!(format!("https://{endpoint}")));

        let trust_domain = detail
            .trust_domain
            .as_deref()
            .or(dest_node.group_name.as_deref());

        let mut ca_source = json!({
            "type": "spire",
            "socket_path": spire_socket
        });
        if let Some(td) = trust_domain {
            ca_source["trust_domains"] = json!([td]);
        }
        config.insert(
            "tls".to_string(),
            json!({
                "insecure": false,
                "insecure_skip_verify": local_connection,
                "source": {
                    "type": "spire",
                    "socket_path": spire_socket
                },
                "ca_source": ca_source
            }),
        );
    }

    if !config.contains_key("headers") {
        config.insert("headers".to_string(), json!({}));
    }
    if !config.contains_key("keepalive") {
        config.insert(
            "keepalive".to_string(),
            json!({
                "http2_keepalive": "20s",
                "keep_alive_while_idle": false,
                "tcp_keepalive": "20s",
                "timeout": "20s"
            }),
        );
    }

    let config_data = serde_json::to_string(&config)
        .map_err(|e| format!("failed to encode connection config: {e}"))?;

    let effective_endpoint = config
        .get("endpoint")
        .and_then(|v| v.as_str())
        .unwrap_or(&endpoint)
        .to_string();

    Ok((effective_endpoint, config_data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::inmemory::InMemoryDb;
    use crate::db::ConnectionDetails;
    use crate::node_control::DefaultNodeCommandHandler;
    use std::time::SystemTime;

    fn make_conn_details(ep: &str, external: Option<&str>, mtls: bool) -> ConnectionDetails {
        ConnectionDetails {
            endpoint: ep.to_string(),
            external_endpoint: external.map(|s| s.to_string()),
            trust_domain: None,
            mtls_required: mtls,
            client_config: serde_json::Value::Null,
        }
    }

    fn make_node(id: &str, group: Option<&str>, details: Vec<ConnectionDetails>) -> crate::db::Node {
        crate::db::Node {
            id: id.to_string(),
            group_name: group.map(|s| s.to_string()),
            conn_details: details,
            last_updated: SystemTime::now(),
        }
    }

    fn make_route_service(db: crate::db::SharedDb) -> Arc<RouteService> {
        let handler = DefaultNodeCommandHandler::new();
        RouteService::new(
            db,
            handler,
            ReconcilerConfig {
                max_requeues: 3,
                max_parallel_reconciles: 10,
            },
        )
    }

    // ── select_connection ──────────────────────────────────────────────────

    #[test]
    fn select_connection_same_group_returns_first() {
        let dst = make_node(
            "dst",
            Some("grp"),
            vec![make_conn_details("dst:8080", Some("ext:9090"), false)],
        );
        let src = make_node("src", Some("grp"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    #[test]
    fn select_connection_different_group_prefers_external() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![
                make_conn_details("dst:8080", None, false),
                make_conn_details("dst:8081", Some("ext:9090"), false),
            ],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.external_endpoint.as_deref(), Some("ext:9090"));
    }

    #[test]
    fn select_connection_different_group_no_external_falls_back() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![make_conn_details("dst:8080", None, false)],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    // ── generate_config_data ───────────────────────────────────────────────

    #[test]
    fn generate_config_data_local_http() {
        let cd = make_conn_details("host:8080", None, false);
        let dest = make_node("dst", Some("g"), vec![cd.clone()]);
        let src = make_node("src", Some("g"), vec![]);
        let (ep, data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        assert!(ep.starts_with("http://"));
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["tls"]["insecure"], true);
        assert!(v["backoff"].is_object());
        assert!(v["keepalive"].is_object());
    }

    #[test]
    fn generate_config_data_external_no_mtls() {
        let cd = make_conn_details("host:8080", Some("ext:9090"), false);
        let dest = make_node("dst", Some("g1"), vec![cd.clone()]);
        let src = make_node("src", Some("g2"), vec![]);
        let (ep, data) = generate_config_data(&cd, false, &dest, &src).unwrap();
        assert!(ep.contains("ext:9090"));
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["tls"]["insecure"], true);
    }

    #[test]
    fn generate_config_data_no_external_endpoint_remote_returns_error() {
        let cd = make_conn_details("host:8080", None, false);
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        // local_connection=false + no external endpoint → error.
        assert!(generate_config_data(&cd, false, &dest, &src).is_err());
    }

    #[test]
    fn generate_config_data_non_object_client_config_returns_error() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!([1, 2, 3]);
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        assert!(generate_config_data(&cd, true, &dest, &src).is_err());
    }

    #[test]
    fn generate_config_data_preserves_existing_tls() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!({ "tls": { "insecure": false } });
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        let (ep, _data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        assert!(ep.starts_with("https://"));
    }

    #[test]
    fn generate_config_data_preserves_existing_backoff() {
        let mut cd = make_conn_details("host:8080", None, false);
        cd.client_config = serde_json::json!({ "backoff": { "type": "exponential" } });
        let dest = make_node("dst", None, vec![cd.clone()]);
        let src = make_node("src", None, vec![]);
        let (_ep, data) = generate_config_data(&cd, true, &dest, &src).unwrap();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["backoff"]["type"], "exponential");
    }

    // ── add_route validation ───────────────────────────────────────────────

    #[tokio::test]
    async fn add_route_empty_source_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: String::new(),
                dest_node_id: "dst".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("source"));
    }

    #[tokio::test]
    async fn add_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: String::new(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("destination"));
    }

    #[tokio::test]
    async fn add_route_same_src_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .add_route(Route {
                source_node_id: "node1".to_string(),
                dest_node_id: "node1".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("same"));
    }

    // ── delete_route validation ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .delete_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: String::new(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("destNodeID"));
    }

    #[tokio::test]
    async fn delete_route_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc
            .delete_route(Route {
                source_node_id: "src".to_string(),
                dest_node_id: "dst".to_string(),
                link_id: String::new(),
                component0: "o".to_string(),
                component1: "n".to_string(),
                component2: "t".to_string(),
                component_id: None,
            })
            .await;
        assert!(result.is_err());
    }

    // ── get_connection_details ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_connection_details_dest_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("src", "ghost_dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ghost_dst"));
    }

    #[tokio::test]
    async fn get_connection_details_src_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![make_conn_details("dst:8080", None, false)],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("ghost_src", "dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ghost_src"));
    }

    #[tokio::test]
    async fn get_connection_details_no_conn_details_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: None,
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_connection_details("src", "dst").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_connection_details_same_group_local() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![make_conn_details("dst:8080", None, false)],
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let (ep, _) = svc.get_connection_details("src", "dst").await.unwrap();
        assert!(ep.starts_with("http://dst:8080"));
    }
}

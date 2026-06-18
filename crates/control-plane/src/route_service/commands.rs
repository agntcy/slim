// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Public API commands called by the northbound gRPC handlers.
//! These are the entry points that `slimctl` and other clients invoke.

use slim_datapath::api::NameId;
use uuid::Uuid;

use crate::api::proto::controller::proto::v1::{
    ConnectionListResponse, ControlMessage, Route as ProtoRoute, RouteListResponse,
    control_message::Payload,
};
use crate::db::RouteName;
use crate::error::{Error, Result};
use crate::node_transport::ResponseKind;
use crate::types::validate_route_nodes;

use super::ALL_NODES_ID;

impl super::RouteService {
    pub async fn add_route(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
        route: &ProtoRoute,
    ) -> Result<String> {
        validate_route_nodes(source_node_id, dest_node_id)?;

        let Some(route_name) = route.name.as_ref() else {
            return Err(Error::InvalidInput("route name is required".to_string()));
        };

        // For wildcard routes, determine whether this is the first announcer
        // for this name (creates the full SPT) or a subsequent one (installs
        // downward path from root to new announcer).
        let existing_root = if source_node_id == ALL_NODES_ID {
            let (c0, c1, c2) = route_name.str_components();
            let comp_id = if route_name.id() == NameId::NULL_COMPONENT {
                None
            } else {
                Some(route_name.string_id())
            };
            self.0
                .db
                .get_destination_node_id_for_name(c0, c1, c2, comp_id.as_deref())
                .await
                .unwrap_or(None)
        } else {
            None
        };

        // Resolve groups for route record.
        let source_group = if source_node_id == ALL_NODES_ID {
            ALL_NODES_ID.to_string()
        } else {
            self.0
                .db
                .get_node(source_node_id)
                .await
                .ok()
                .flatten()
                .and_then(|n| n.group_name)
                .unwrap_or_default()
        };
        let dest_group = self
            .0
            .db
            .get_node(dest_node_id)
            .await
            .ok()
            .flatten()
            .and_then(|n| n.group_name)
            .unwrap_or_default();

        let db_route = route.to_db_route(source_node_id, &source_group, dest_node_id, &dest_group);
        let route_id = self.add_single_route(db_route).await?;

        if source_node_id == ALL_NODES_ID {
            let all_nodes = self.0.db.list_nodes().await?;
            let all_links = self.0.db.list_all_links().await?;
            let (c0, c1, c2) = route_name.str_components();
            let comp_id = if route_name.id() == NameId::NULL_COMPONENT {
                None
            } else {
                Some(route_name.string_id())
            };

            match existing_root {
                None => {
                    // First announcer: full SPT expansion (upward routes).
                    self.expand_route_via_spt(
                        dest_node_id,
                        c0,
                        c1,
                        c2,
                        comp_id.as_deref(),
                        &all_nodes,
                        &all_links,
                    )
                    .await;
                }
                Some(ref root_node_id) => {
                    // Subsequent announcer: install downward path from root
                    // to this new announcer.
                    self.install_downward_path(
                        root_node_id,
                        dest_node_id,
                        c0,
                        c1,
                        c2,
                        comp_id.as_deref(),
                        &all_nodes,
                        &all_links,
                    )
                    .await;
                }
            }
        }

        Ok(route_id)
    }

    pub async fn delete_route(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
        route: &ProtoRoute,
    ) -> Result<()> {
        if dest_node_id.is_empty() {
            return Err(Error::EmptyDestNodeId);
        }

        let Some(route_name) = route.name.as_ref() else {
            return Err(Error::InvalidInput("route name is required".to_string()));
        };

        let (c0, c1, c2) = route_name.str_components();
        let comp_id = if route_name.id() == NameId::NULL_COMPONENT {
            None
        } else {
            Some(route_name.string_id())
        };

        let name = RouteName {
            component0: c0,
            component1: c1,
            component2: c2,
            component_id: comp_id.as_deref(),
        };

        if source_node_id == ALL_NODES_ID {
            // Delete the wildcard route itself.
            let db_route = self
                .0
                .db
                .get_route_for_src_dest_name(source_node_id, &name, dest_node_id, None)
                .await?
                .ok_or(Error::InvalidInput("route not found".to_string()))?;
            self.0.db.delete_route(&db_route.id).await?;

            // Also delete all per-node expansions.
            let per_node = self
                .0
                .db
                .get_routes_for_dest_node_id_and_name(dest_node_id, c0, c1, c2, comp_id.as_deref())
                .await?;
            for r in per_node {
                self.delete_single_route(&r.source_node_id, &r.id, &r.to_string())
                    .await?;
            }
            return Ok(());
        }

        let link_id = match route.link_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => {
                let src_group = self
                    .0
                    .db
                    .get_node(source_node_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|n| n.group_name)
                    .unwrap_or_default();
                let dst_group = self
                    .0
                    .db
                    .get_node(dest_node_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|n| n.group_name)
                    .unwrap_or_default();
                self.find_matching_link(source_node_id, &src_group, dest_node_id, &dst_group)
                    .await?
            }
        };

        let db_route = self
            .0
            .db
            .get_route_for_src_dest_name(source_node_id, &name, dest_node_id, Some(&link_id))
            .await?
            .ok_or(Error::InvalidInput("route not found".to_string()))?;

        self.delete_single_route(source_node_id, &db_route.id, &db_route.to_string())
            .await
    }

    pub async fn list_node_routes(&self, node_id: &str) -> Result<RouteListResponse> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::RouteListRequest(
                crate::api::proto::controller::proto::v1::RouteListRequest {},
            )),
        };
        let chunks = self
            .0
            .cmd_handler
            .send_and_wait(node_id, msg, ResponseKind::RouteListResponse)
            .await?;
        let mut entries = Vec::new();
        for chunk in chunks {
            if let Some(Payload::RouteListResponse(r)) = chunk.payload {
                entries.extend(r.entries);
            }
        }
        Ok(RouteListResponse {
            original_message_id: message_id,
            entries,
            done: true,
        })
    }

    pub async fn list_connections(&self, node_id: &str) -> Result<ConnectionListResponse> {
        let message_id = Uuid::new_v4().to_string();
        let msg = ControlMessage {
            message_id: message_id.clone(),
            payload: Some(Payload::ConnectionListRequest(
                crate::api::proto::controller::proto::v1::ConnectionListRequest {},
            )),
        };
        let chunks = self
            .0
            .cmd_handler
            .send_and_wait(node_id, msg, ResponseKind::ConnectionListResponse)
            .await?;
        let mut entries = Vec::new();
        for chunk in chunks {
            if let Some(Payload::ConnectionListResponse(r)) = chunk.payload {
                entries.extend(r.entries);
            }
        }
        Ok(ConnectionListResponse {
            original_message_id: message_id,
            entries,
            done: true,
        })
    }

    /// Called when a node reports it received an incoming connection (after the
    /// data-plane link negotiation). Claims the unclaimed link for this node.
    pub async fn connection_received(&self, node_id: &str, link_id: &str) {
        // Determine the node's group.
        let node_group = match self.0.db.get_node(node_id).await {
            Ok(Some(n)) => n.group_name.unwrap_or_default(),
            Ok(None) => {
                tracing::warn!(
                    "connection_received: node {node_id} not found, ignoring claim for {link_id}"
                );
                return;
            }
            Err(e) => {
                tracing::error!("connection_received: failed to get node {node_id}: {e}");
                return;
            }
        };

        if node_group.is_empty() {
            tracing::warn!(
                "connection_received: node {node_id} has no group, cannot claim link {link_id}"
            );
            return;
        }

        match self.0.db.claim_link(link_id, &node_group, node_id).await {
            Ok(Some(claimed)) => {
                tracing::info!(
                    "connection_received: node {node_id} claimed link {link_id} from {}",
                    claimed.source_node_id
                );

                // Re-expand wildcard routes now that a new link is fully established.
                let all_nodes = self.0.db.list_nodes().await.unwrap_or_else(|e| {
                    tracing::error!("failed to list nodes for route expansion: {e}");
                    vec![]
                });
                let all_links = self.0.db.list_all_links().await.unwrap_or_else(|e| {
                    tracing::error!("failed to list links for route expansion: {e}");
                    vec![]
                });
                self.expand_all_wildcard_routes(&all_nodes, &all_links)
                    .await;

                // Enqueue both source and dest for reconciliation so routes
                // can be expanded over the now-established link.
                self.0.queue.add(claimed.source_node_id.clone());
                self.0.queue.add(node_id.to_string());
            }
            Ok(None) => {
                tracing::debug!(
                    "connection_received: link {link_id} already claimed or not found for {node_id}"
                );
            }
            Err(e) => {
                tracing::error!(
                    "connection_received: failed to claim link {link_id} for {node_id}: {e}"
                );
            }
        }
    }
}

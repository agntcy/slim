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

        // For wildcard routes, determine whether this is the first announcer
        // for this name (creates the full SPT) or a subsequent one (installs
        // downward path from root to new announcer).
        let existing_root = if source_node_id == ALL_NODES_ID {
            let n = route.name.as_ref().unwrap();
            let (c0, c1, c2) = n.str_components();
            let comp_id = if n.id() == NameId::NULL_COMPONENT {
                None
            } else {
                Some(n.string_id())
            };
            self.0
                .db
                .get_destination_node_id_for_name(c0, c1, c2, comp_id.as_deref())
                .await
                .unwrap_or(None)
        } else {
            None
        };

        let db_route = route.to_db_route(source_node_id, dest_node_id);
        let route_id = self.add_single_route(db_route).await?;

        if source_node_id == ALL_NODES_ID {
            let all_nodes = self.0.db.list_nodes().await?;
            let all_links = self.0.db.list_all_links().await?;
            let n = route.name.as_ref().unwrap();
            let (c0, c1, c2) = n.str_components();
            let comp_id = if n.id() == NameId::NULL_COMPONENT {
                None
            } else {
                Some(n.string_id())
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

        let (c0, c1, c2) = route.name.as_ref().unwrap().str_components();
        let comp_id = if route.name.as_ref().unwrap().id() == NameId::NULL_COMPONENT {
            None
        } else {
            Some(route.name.as_ref().unwrap().string_id())
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
                self.find_matching_link(source_node_id, dest_node_id)
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
}

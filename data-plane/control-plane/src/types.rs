// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use crate::api::proto::controller::proto::v1::Route as ProtoRoute;
use crate::db::RouteStatus;
use crate::error::{Error, Result};

pub const ALL_NODES_ID: &str = crate::db::ALL_NODES_ID;

pub fn validate_route_nodes(source_node_id: &str, dest_node_id: &str) -> Result<()> {
    if source_node_id.is_empty() {
        return Err(Error::EmptySourceNodeId);
    }
    if dest_node_id.is_empty() {
        return Err(Error::EmptyDestNodeId);
    }
    if source_node_id == dest_node_id {
        return Err(Error::SameSourceAndDest);
    }
    Ok(())
}

impl ProtoRoute {
    pub fn to_db_route(&self, source_node_id: &str, dest_node_id: &str) -> crate::db::Route {
        crate::db::Route {
            id: String::new(),
            source_node_id: source_node_id.to_string(),
            dest_node_id: dest_node_id.to_string(),
            link_id: None,
            name: self.name.clone(),
            status: if source_node_id != ALL_NODES_ID {
                RouteStatus::Pending
            } else {
                RouteStatus::Applied
            },
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        }
    }
}

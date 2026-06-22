// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use slim_datapath::api::NameId;

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

pub trait ProtoRouteExt {
    fn to_db_route(
        &self,
        source_node_id: &str,
        source_group: &str,
        dest_node_id: &str,
        dest_group: &str,
    ) -> crate::db::Route;
}

impl ProtoRouteExt for ProtoRoute {
    fn to_db_route(
        &self,
        source_node_id: &str,
        source_group: &str,
        dest_node_id: &str,
        dest_group: &str,
    ) -> crate::db::Route {
        let n = self.name.as_ref().unwrap();
        let (c0, c1, c2) = n.str_components();
        let comp_id = if n.id() == NameId::NULL_COMPONENT {
            None
        } else {
            Some(n.string_id())
        };

        crate::db::Route {
            id: String::new(),
            source_node_id: source_node_id.to_string(),
            source_group: source_group.to_string(),
            dest_node_id: dest_node_id.to_string(),
            dest_group: dest_group.to_string(),
            link_id: None,
            component0: c0.to_string(),
            component1: c1.to_string(),
            component2: c2.to_string(),
            component_id: comp_id,
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

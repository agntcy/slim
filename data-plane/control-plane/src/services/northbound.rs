// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use prost_types::{Struct, Value, value::Kind};
use tonic::{Request, Response, Status};

use crate::api::proto::controller::proto::v1::{
    Ack, AddParticipantRequest, ConnectionDetails, ConnectionListResponse, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListChannelsResponse, ListParticipantsRequest,
    ListParticipantsResponse, SubscriptionListResponse,
};
use crate::api::proto::controlplane::proto::v1::{
    AddRouteRequest, AddRouteResponse, CreateChannelRequest, CreateChannelResponse,
    DeleteRouteRequest, DeleteRouteResponse, LinkListRequest, LinkListResponse, LinkStatus,
    NodeListRequest, NodeListResponse, RouteListRequest, RouteListResponse, RouteStatus,
    control_plane_service_server::ControlPlaneService,
};
use crate::db::SharedDb;
use crate::node_control::{DefaultNodeCommandHandler, NodeStatus};
use crate::services::group::GroupService;
use crate::services::routes::RouteService;

pub struct NorthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
    group_service: GroupService,
}

impl NorthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
        group_service: GroupService,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
            group_service,
        }
    }
}

#[tonic::async_trait]
impl ControlPlaneService for NorthboundApiService {
    async fn list_nodes(
        &self,
        _request: Request<NodeListRequest>,
    ) -> Result<Response<NodeListResponse>, Status> {
        let nodes = self.db.list_nodes().await;
        let mut entries = Vec::with_capacity(nodes.len());
        for node in nodes {
            let node_status = self.cmd_handler.get_connection_status(&node.id).await;
            let status = match node_status {
                NodeStatus::Connected => {
                    crate::api::proto::controlplane::proto::v1::NodeStatus::Connected as i32
                }
                NodeStatus::NotConnected => {
                    crate::api::proto::controlplane::proto::v1::NodeStatus::NotConnected as i32
                }
                NodeStatus::Unknown => {
                    crate::api::proto::controlplane::proto::v1::NodeStatus::Unspecified as i32
                }
            };

            let connections = node
                .conn_details
                .iter()
                .map(|cd| {
                    let mut metadata = None;
                    if let Some(ref ee) = cd.external_endpoint
                        && !ee.is_empty()
                    {
                        let mut fields = std::collections::BTreeMap::new();
                        fields.insert(
                            "external_endpoint".to_string(),
                            Value {
                                kind: Some(Kind::StringValue(ee.clone())),
                            },
                        );
                        metadata = Some(Struct { fields });
                    }
                    ConnectionDetails {
                        endpoint: cd.endpoint.clone(),
                        mtls_required: cd.mtls_required,
                        metadata,
                        auth: None,
                        tls: None,
                    }
                })
                .collect();

            entries.push(crate::api::proto::controlplane::proto::v1::NodeEntry {
                id: node.id,
                connections,
                status,
            });
        }
        Ok(Response::new(NodeListResponse { entries }))
    }

    async fn list_subscriptions(
        &self,
        request: Request<crate::api::proto::controlplane::proto::v1::Node>,
    ) -> Result<Response<SubscriptionListResponse>, Status> {
        let node_id = request.into_inner().id;
        let node = self
            .db
            .get_node(&node_id)
            .await
            .ok_or_else(|| Status::not_found(format!("node {node_id} not found")))?;

        self.route_service
            .list_subscriptions(&node.id)
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn list_connections(
        &self,
        request: Request<crate::api::proto::controlplane::proto::v1::Node>,
    ) -> Result<Response<ConnectionListResponse>, Status> {
        let node_id = request.into_inner().id;
        let node = self
            .db
            .get_node(&node_id)
            .await
            .ok_or_else(|| Status::not_found(format!("node {node_id} not found")))?;

        self.route_service
            .list_connections(&node.id)
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn add_route(
        &self,
        request: Request<AddRouteRequest>,
    ) -> Result<Response<AddRouteResponse>, Status> {
        let req = request.into_inner();

        self.db
            .get_node(&req.node_id)
            .await
            .ok_or_else(|| Status::not_found(format!("invalid source nodeID: {}", req.node_id)))?;

        if req.dest_node_id.is_empty() {
            return Err(Status::invalid_argument("destNodeId must be provided"));
        }
        self.db.get_node(&req.dest_node_id).await.ok_or_else(|| {
            Status::not_found(format!("invalid destination nodeID: {}", req.dest_node_id))
        })?;

        let sub = req.subscription.unwrap_or_default();
        let route = crate::services::routes::Route {
            source_node_id: req.node_id,
            dest_node_id: req.dest_node_id,
            component0: sub.component_0,
            component1: sub.component_1,
            component2: sub.component_2,
            component_id: sub.id,
            link_id: String::new(),
        };

        let route_id = self
            .route_service
            .add_route(route)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(AddRouteResponse {
            success: true,
            route_id,
        }))
    }

    async fn delete_route(
        &self,
        request: Request<DeleteRouteRequest>,
    ) -> Result<Response<DeleteRouteResponse>, Status> {
        let req = request.into_inner();

        self.db
            .get_node(&req.node_id)
            .await
            .ok_or_else(|| Status::not_found(format!("invalid source nodeID: {}", req.node_id)))?;

        if req.dest_node_id.is_empty() {
            return Err(Status::invalid_argument("destNodeId must be provided"));
        }
        self.db.get_node(&req.dest_node_id).await.ok_or_else(|| {
            Status::not_found(format!("invalid destination nodeID: {}", req.dest_node_id))
        })?;

        let sub = req.subscription.unwrap_or_default();
        let route = crate::services::routes::Route {
            source_node_id: req.node_id,
            dest_node_id: req.dest_node_id,
            component0: sub.component_0,
            component1: sub.component_1,
            component2: sub.component_2,
            component_id: sub.id,
            link_id: String::new(),
        };

        self.route_service
            .delete_route(route)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(DeleteRouteResponse { success: true }))
    }

    async fn create_channel(
        &self,
        request: Request<CreateChannelRequest>,
    ) -> Result<Response<CreateChannelResponse>, Status> {
        self.group_service
            .create_channel(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn delete_channel(
        &self,
        request: Request<DeleteChannelRequest>,
    ) -> Result<Response<Ack>, Status> {
        self.group_service
            .delete_channel(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn add_participant(
        &self,
        request: Request<AddParticipantRequest>,
    ) -> Result<Response<Ack>, Status> {
        self.group_service
            .add_participant(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn delete_participant(
        &self,
        request: Request<DeleteParticipantRequest>,
    ) -> Result<Response<Ack>, Status> {
        self.group_service
            .delete_participant(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn list_channels(
        &self,
        request: Request<ListChannelsRequest>,
    ) -> Result<Response<ListChannelsResponse>, Status> {
        self.group_service
            .list_channels(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn list_participants(
        &self,
        request: Request<ListParticipantsRequest>,
    ) -> Result<Response<ListParticipantsResponse>, Status> {
        self.group_service
            .list_participants(request.into_inner())
            .await
            .map(Response::new)
            .map_err(|e| Status::internal(e.to_string()))
    }

    async fn list_routes(
        &self,
        request: Request<RouteListRequest>,
    ) -> Result<Response<RouteListResponse>, Status> {
        let req = request.into_inner();
        let routes = self
            .db
            .filter_routes_by_src_dest(&req.src_node_id, &req.dest_node_id)
            .await;

        let mut route_entries: Vec<crate::api::proto::controlplane::proto::v1::RouteEntry> = routes
            .iter()
            .map(|r| {
                let status = match r.status {
                    crate::db::RouteStatus::Applied => RouteStatus::Applied as i32,
                    crate::db::RouteStatus::Failed => RouteStatus::Failed as i32,
                    crate::db::RouteStatus::Pending => RouteStatus::Pending as i32,
                };
                let last_updated = r
                    .last_updated
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                crate::api::proto::controlplane::proto::v1::RouteEntry {
                    id: r.id as u64,
                    source_node_id: r.source_node_id.clone(),
                    dest_node_id: r.dest_node_id.clone(),
                    link_id: r.link_id.clone(),
                    component_0: r.component0.clone(),
                    component_1: r.component1.clone(),
                    component_2: r.component2.clone(),
                    component_id: r.component_id.map(|v| v as u64),
                    status,
                    status_msg: r.status_msg.clone(),
                    deleted: r.deleted,
                    last_updated,
                    dest_endpoint: String::new(),
                    conn_config_data: String::new(),
                }
            })
            .collect();

        // Sort: wildcard (*) sources first, then alphabetically.
        route_entries.sort_by(|a, b| {
            match (
                a.source_node_id == crate::db::ALL_NODES_ID,
                b.source_node_id == crate::db::ALL_NODES_ID,
            ) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.source_node_id.cmp(&b.source_node_id),
            }
        });

        Ok(Response::new(RouteListResponse {
            routes: route_entries,
        }))
    }

    async fn list_links(
        &self,
        request: Request<LinkListRequest>,
    ) -> Result<Response<LinkListResponse>, Status> {
        let req = request.into_inner();
        let nodes = self.db.list_nodes().await;
        let mut seen = std::collections::HashSet::new();
        let mut link_entries = Vec::new();

        for node in &nodes {
            let links = self.db.get_links_for_node(&node.id).await;
            for l in links {
                if !req.src_node_id.is_empty() && l.source_node_id != req.src_node_id {
                    continue;
                }
                if !req.dest_node_id.is_empty() && l.dest_node_id != req.dest_node_id {
                    continue;
                }
                let key = format!(
                    "{}|{}|{}|{}",
                    l.source_node_id, l.dest_node_id, l.dest_endpoint, l.link_id
                );
                if !seen.insert(key.clone()) {
                    continue;
                }
                let link_status = match l.status {
                    crate::db::LinkStatus::Pending => LinkStatus::Pending as i32,
                    crate::db::LinkStatus::Applied => LinkStatus::Applied as i32,
                    crate::db::LinkStatus::Failed => LinkStatus::Failed as i32,
                };
                let last_updated = l
                    .last_updated
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                link_entries.push(crate::api::proto::controlplane::proto::v1::LinkEntry {
                    id: key,
                    link_id: l.link_id,
                    source_node_id: l.source_node_id,
                    dest_node_id: l.dest_node_id,
                    dest_endpoint: l.dest_endpoint,
                    conn_config_data: l.conn_config_data,
                    status: link_status,
                    status_msg: l.status_msg,
                    deleted: l.deleted,
                    last_updated,
                });
            }
        }

        link_entries.sort_by(|a, b| {
            a.source_node_id
                .cmp(&b.source_node_id)
                .then(a.dest_node_id.cmp(&b.dest_node_id))
                .then(a.link_id.cmp(&b.link_id))
        });

        Ok(Response::new(LinkListResponse {
            links: link_entries,
        }))
    }
}

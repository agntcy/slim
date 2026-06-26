// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::api::{NameId, ProtoName};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::api::proto::controller::proto::v1::{
    ConnectionDetails, ConnectionListResponse, RouteListResponse as NodeRouteListResponse,
    connection_details::SpireMtls,
};
use crate::api::proto::controlplane::proto::v1::{
    AddSegmentRequest, AddSegmentResponse, AddTopologyLinkRequest, AddTopologyLinkResponse,
    LinkEntry, LinkListRequest, LinkStatus, NodeEntry, NodeListRequest, RemoveSegmentRequest,
    RemoveSegmentResponse, RemoveTopologyLinkRequest, RemoveTopologyLinkResponse, RouteEntry,
    RouteListRequest, RouteStatus, SegmentEdge, SegmentEntry, SegmentListRequest,
    SegmentListResponse, control_plane_service_server::ControlPlaneService,
};
use crate::db::SharedDb;
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus};
use crate::route_service::RouteService;
use crate::types::DEFAULT_SEGMENT;

pub struct NorthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
}

impl NorthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
        }
    }

    /// Build a map of link_id → qualified peer name (group/node) for a given node.
    fn build_link_peer_map<'a>(
        node_id: &str,
        links: &'a [crate::db::Link],
    ) -> std::collections::HashMap<&'a str, String> {
        links
            .iter()
            .map(|link| {
                let peer = if link.source_node_id == node_id {
                    if link
                        .dest_node_id
                        .starts_with(&format!("{}/", link.dest_group))
                    {
                        link.dest_node_id.clone()
                    } else {
                        format!("{}/{}", link.dest_group, link.dest_node_id)
                    }
                } else if link
                    .source_node_id
                    .starts_with(&format!("{}/", link.source_group))
                {
                    link.source_node_id.clone()
                } else {
                    format!("{}/{}", link.source_group, link.source_node_id)
                };
                (link.link_id.as_str(), peer)
            })
            .collect()
    }

    /// Enrich `peer_node_id` on connection entries using group information.
    fn enrich_peer_node_ids(
        entries: &mut [crate::api::proto::controller::proto::v1::ConnectionEntry],
        node_group: &str,
        link_peer_map: &std::collections::HashMap<String, String>,
    ) {
        use crate::api::proto::controller::proto::v1::ConnectionType;
        for entry in entries.iter_mut() {
            match entry.connection_type() {
                ConnectionType::Peer => {
                    if let Some(bare_id) = entry.peer_node_id.take() {
                        if bare_id.starts_with(&format!("{}/", node_group)) {
                            entry.peer_node_id = Some(bare_id);
                        } else {
                            entry.peer_node_id = Some(format!("{}/{}", node_group, bare_id));
                        }
                    }
                }
                ConnectionType::Remote => {
                    if let Some(ref link_id) = entry.link_id
                        && let Some(qualified) = link_peer_map.get(link_id.as_str())
                    {
                        entry.peer_node_id = Some(qualified.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Fetch link data for a node and build the enrichment context.
    /// Returns `(node_group, link_peer_map)`. Logs a warning on DB errors
    /// and continues with an empty map (enrichment is best-effort).
    async fn enrichment_context(
        &self,
        node: &crate::db::Node,
        caller: &str,
    ) -> (String, std::collections::HashMap<String, String>) {
        let node_group = node.group_name.clone().unwrap_or_default();
        let links = self
            .db
            .get_links_for_node(&node.id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("{caller}: failed to fetch links for enrichment: {e}");
                Vec::new()
            });
        let link_peer_map = Self::build_link_peer_map(&node.id, &links)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        (node_group, link_peer_map)
    }
}

#[tonic::async_trait]
impl ControlPlaneService for NorthboundApiService {
    type ListNodesStream = ReceiverStream<Result<NodeEntry, Status>>;
    type ListRoutesStream = ReceiverStream<Result<RouteEntry, Status>>;
    type ListLinksStream = ReceiverStream<Result<LinkEntry, Status>>;

    async fn list_nodes(
        &self,
        _request: Request<NodeListRequest>,
    ) -> Result<Response<Self::ListNodesStream>, Status> {
        let nodes = self.db.list_nodes().await.map_err(|e| {
            tracing::error!("list_nodes: {e}");
            Status::internal("internal error")
        })?;
        let status_futs: Vec<_> = nodes
            .iter()
            .map(|node| self.cmd_handler.get_connection_status(&node.id))
            .collect();
        let statuses = futures::future::join_all(status_futs).await;

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for (node, node_status) in nodes.into_iter().zip(statuses) {
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
                        let spire_mtls = cd.spire_mtls.as_ref().map(|s| SpireMtls {
                            socket_path: s.socket_path.clone(),
                            trust_domain: s.trust_domain.clone(),
                        });
                        ConnectionDetails {
                            endpoint: cd.endpoint.clone(),
                            external_endpoint: cd.external_endpoint.clone(),
                            spire_mtls,
                            metadata: None,
                        }
                    })
                    .collect();

                let group = node.group_name.unwrap_or_default();
                let entry = NodeEntry {
                    id: node.id,
                    connections,
                    status,
                    group,
                };
                if tx.send(Ok(entry)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn list_node_routes(
        &self,
        request: Request<crate::api::proto::controlplane::proto::v1::Node>,
    ) -> Result<Response<NodeRouteListResponse>, Status> {
        let node_id = request.into_inner().id;
        let node = self
            .db
            .get_node(&node_id)
            .await
            .map_err(|e| {
                tracing::error!("list_node_routes get_node: {e}");
                Status::internal("internal error")
            })?
            .ok_or_else(|| Status::not_found(format!("node {node_id} not found")))?;

        let mut resp = self
            .route_service
            .list_node_routes(&node.id)
            .await
            .map_err(|e| {
                tracing::error!("list_node_routes: {e}");
                Status::internal("internal error")
            })?;

        // Enrich peer_node_id with group prefix (same logic as list_connections).
        let (node_group, link_peer_map) = self.enrichment_context(&node, "list_node_routes").await;

        for entry in &mut resp.entries {
            Self::enrich_peer_node_ids(&mut entry.connections, &node_group, &link_peer_map);
        }

        Ok(Response::new(resp))
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
            .map_err(|e| {
                tracing::error!("list_connections get_node: {e}");
                Status::internal("internal error")
            })?
            .ok_or_else(|| Status::not_found(format!("node {node_id} not found")))?;

        let mut resp = self
            .route_service
            .list_connections(&node.id)
            .await
            .map_err(|e| {
                tracing::error!("list_connections: {e}");
                Status::internal("internal error")
            })?;

        // Enrich peer_node_id with group prefix using link information.
        let (node_group, link_peer_map) = self.enrichment_context(&node, "list_connections").await;
        Self::enrich_peer_node_ids(&mut resp.entries, &node_group, &link_peer_map);

        Ok(Response::new(resp))
    }

    async fn list_routes(
        &self,
        request: Request<RouteListRequest>,
    ) -> Result<Response<Self::ListRoutesStream>, Status> {
        let req = request.into_inner();
        let routes = self
            .db
            .filter_routes_by_src_dest(&req.src_node_id, &req.dest_node_id)
            .await
            .map_err(|e| {
                tracing::error!("list_routes: {e}");
                Status::internal("internal error")
            })?;

        let mut route_entries: Vec<RouteEntry> = routes
            .iter()
            .map(|r| {
                let status = match r.status {
                    crate::db::RouteStatus::Applied => RouteStatus::Applied as i32,
                    crate::db::RouteStatus::Failed => RouteStatus::Failed as i32,
                    crate::db::RouteStatus::Pending => RouteStatus::Pending as i32,
                    crate::db::RouteStatus::Deleted => RouteStatus::Deleted as i32,
                };
                let last_updated = r
                    .last_updated
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let name = ProtoName::from_strings([&r.component0, &r.component1, &r.component2])
                    .with_id(
                        r.component_id
                            .as_deref()
                            .map(|s| {
                                NameId::try_from(s.to_string())
                                    .map(u128::from)
                                    .unwrap_or(NameId::NULL_COMPONENT)
                            })
                            .unwrap_or(NameId::NULL_COMPONENT),
                    );
                RouteEntry {
                    id: r.id.clone(),
                    source_node_id: r.source_node_id.clone(),
                    dest_node_id: r.dest_node_id.clone(),
                    link_id: r.link_id.clone().unwrap_or_default(),
                    name: Some(name),
                    status,
                    status_msg: r.status_msg.clone(),
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

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for entry in route_entries {
                if tx.send(Ok(entry)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn list_links(
        &self,
        request: Request<LinkListRequest>,
    ) -> Result<Response<Self::ListLinksStream>, Status> {
        let req = request.into_inner();
        let links = self
            .db
            .filter_links_by_src_dest(&req.src_node_id, &req.dest_node_id)
            .await
            .map_err(|e| {
                tracing::error!("list_links: {e}");
                Status::internal("internal error")
            })?;

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for l in links {
                let key = format!(
                    "{}|{}|{}|{}",
                    l.source_node_id, l.dest_node_id, l.dest_endpoint, l.link_id
                );
                let link_status = match l.status {
                    crate::db::LinkStatus::Pending => LinkStatus::Pending as i32,
                    crate::db::LinkStatus::Connecting => LinkStatus::Pending as i32,
                    crate::db::LinkStatus::Applied => LinkStatus::Applied as i32,
                    crate::db::LinkStatus::Failed => LinkStatus::Failed as i32,
                    crate::db::LinkStatus::Deleted => LinkStatus::Unspecified as i32,
                };
                let last_updated = l
                    .last_updated
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let entry = LinkEntry {
                    id: key,
                    link_id: l.link_id,
                    source_node_id: l.source_node_id,
                    dest_node_id: l.dest_node_id,
                    dest_endpoint: l.dest_endpoint,
                    conn_config_data: serde_json::to_string(&l.conn_config_data)
                        .unwrap_or_default(),
                    status: link_status,
                    status_msg: l.status_msg,
                    deleted: l.status == crate::db::LinkStatus::Deleted,
                    last_updated,
                };
                if tx.send(Ok(entry)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn list_segments(
        &self,
        _request: Request<SegmentListRequest>,
    ) -> Result<Response<SegmentListResponse>, Status> {
        let segments = self.route_service.list_segments().await;
        let entries = segments
            .into_iter()
            .map(|(name, groups, edges)| SegmentEntry {
                name,
                groups,
                edges: edges
                    .into_iter()
                    .map(|(a, b)| SegmentEdge {
                        group_a: a,
                        group_b: b,
                    })
                    .collect(),
            })
            .collect();
        Ok(Response::new(SegmentListResponse { segments: entries }))
    }

    async fn add_segment(
        &self,
        request: Request<AddSegmentRequest>,
    ) -> Result<Response<AddSegmentResponse>, Status> {
        let name = &request.get_ref().name;
        if name.is_empty() {
            return Err(Status::invalid_argument("segment name must not be empty"));
        }
        self.route_service.add_segment(name).await?;
        Ok(Response::new(AddSegmentResponse {}))
    }

    async fn remove_segment(
        &self,
        request: Request<RemoveSegmentRequest>,
    ) -> Result<Response<RemoveSegmentResponse>, Status> {
        let name = &request.get_ref().name;
        if name.is_empty() {
            return Err(Status::invalid_argument("segment name must not be empty"));
        }
        self.route_service.remove_segment(name).await?;
        Ok(Response::new(RemoveSegmentResponse {}))
    }

    async fn add_topology_link(
        &self,
        request: Request<AddTopologyLinkRequest>,
    ) -> Result<Response<AddTopologyLinkResponse>, Status> {
        let req = request.get_ref();
        if req.group_a.is_empty() || req.group_b.is_empty() {
            return Err(Status::invalid_argument(
                "group_a and group_b must not be empty",
            ));
        }
        let segment = if req.segment.is_empty() {
            DEFAULT_SEGMENT
        } else {
            &req.segment
        };
        self.route_service
            .add_topology_link(&req.group_a, &req.group_b, segment)
            .await?;
        Ok(Response::new(AddTopologyLinkResponse {}))
    }

    async fn remove_topology_link(
        &self,
        request: Request<RemoveTopologyLinkRequest>,
    ) -> Result<Response<RemoveTopologyLinkResponse>, Status> {
        let req = request.get_ref();
        if req.group_a.is_empty() || req.group_b.is_empty() {
            return Err(Status::invalid_argument(
                "group_a and group_b must not be empty",
            ));
        }
        let segment = if req.segment.is_empty() {
            DEFAULT_SEGMENT
        } else {
            &req.segment
        };
        self.route_service
            .remove_topology_link(&req.group_a, &req.group_b, segment)
            .await?;
        Ok(Response::new(RemoveTopologyLinkResponse {}))
    }
}

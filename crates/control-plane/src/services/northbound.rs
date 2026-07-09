// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_datapath::api::{NameId, ProtoName};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::api::proto::controller::proto::v1::{
    AuthMethod as ProtoAuthMethod, ConnectionDetails, ConnectionListResponse,
    RouteListResponse as NodeRouteListResponse,
};
use crate::api::proto::controlplane::proto::v1::{
    AddGroupRequest, AddGroupResponse, AddSegmentRequest, AddSegmentResponse,
    AddTopologyLinkRequest, AddTopologyLinkResponse, GroupEntry, LinkEntry, LinkListRequest,
    LinkStatus, ListGroupsRequest, ListGroupsResponse, NodeEntry, NodeListRequest,
    RemoveGroupRequest, RemoveGroupResponse, RemoveSegmentRequest, RemoveSegmentResponse,
    RemoveTopologyLinkRequest, RemoveTopologyLinkResponse, RouteEntry, RouteListRequest,
    RouteStatus, SegmentEdge, SegmentEntry, SegmentListRequest, SegmentListResponse,
    control_plane_service_server::ControlPlaneService,
};
use crate::db::{SharedDb, model};
use crate::auth::GroupAuthenticator;
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus};
use crate::route_service::RouteService;
use crate::types::DEFAULT_SEGMENT;

pub struct NorthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
    authenticator: GroupAuthenticator,
}

impl NorthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
        authenticator: GroupAuthenticator,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
            authenticator,
        }
    }

    /// Build a map of link_id → qualified peer name (group/node) for a given node.
    fn build_link_peer_map(
        node_id: &str,
        links: &[crate::db::Link],
    ) -> std::collections::HashMap<String, String> {
        links
            .iter()
            .map(|link| {
                let peer = if link.source_node_id == node_id {
                    if link.dest_group.is_empty()
                        || link
                            .dest_node_id
                            .starts_with(&format!("{}/", link.dest_group))
                    {
                        link.dest_node_id.clone()
                    } else {
                        format!("{}/{}", link.dest_group, link.dest_node_id)
                    }
                } else if link.source_group.is_empty()
                    || link
                        .source_node_id
                        .starts_with(&format!("{}/", link.source_group))
                {
                    link.source_node_id.clone()
                } else {
                    format!("{}/{}", link.source_group, link.source_node_id)
                };
                (link.link_id.clone(), peer)
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
        let link_peer_map = Self::build_link_peer_map(&node.id, &links);
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
                    .map(|cd| ConnectionDetails {
                        endpoint: cd.endpoint.clone(),
                        external_endpoint: cd.external_endpoint.clone(),
                        tls_required: cd.tls_required,
                        auth_method: match cd.auth_method {
                            model::AuthMethod::Spire => ProtoAuthMethod::Spire as i32,
                            model::AuthMethod::Basic => ProtoAuthMethod::Basic as i32,
                            model::AuthMethod::Jwt => ProtoAuthMethod::Jwt as i32,
                            model::AuthMethod::None => ProtoAuthMethod::None as i32,
                        },
                        spire_trust_domain: cd.spire_trust_domain.clone(),
                        ..Default::default()
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

        // Collect topology link group pairs to identify pending ones.
        let segments = self.route_service.list_segments().await;
        let mut topology_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for (_name, _groups, edges) in &segments {
            for (src, dst) in edges {
                topology_pairs.insert((src.clone(), dst.clone()));
                topology_pairs.insert((dst.clone(), src.clone()));
            }
        }

        // Track which group pairs have physical links.
        let mut realized_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        for l in &links {
            if !l.source_group.is_empty() && !l.dest_group.is_empty() {
                realized_pairs.insert((l.source_group.clone(), l.dest_group.clone()));
                realized_pairs.insert((l.dest_group.clone(), l.source_group.clone()));
            }
        }

        // Pending topology links (configured but no physical link yet).
        let mut pending_entries = Vec::new();
        for (src, dst) in &topology_pairs {
            if !realized_pairs.contains(&(src.clone(), dst.clone())) {
                // Only emit one direction per pair.
                if !pending_entries
                    .iter()
                    .any(|(a, b): &(String, String)| a == dst && b == src)
                {
                    pending_entries.push((src.clone(), dst.clone()));
                }
            }
        }

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
                let conn_config_data =
                    serde_json::to_string(&l.conn_config_data).unwrap_or_else(|e| {
                        tracing::warn!(
                            "failed to serialize conn_config_data for link {}: {e}",
                            l.link_id
                        );
                        String::new()
                    });
                let entry = LinkEntry {
                    id: key,
                    link_id: l.link_id,
                    source_node_id: l.source_node_id,
                    dest_node_id: l.dest_node_id,
                    dest_endpoint: l.dest_endpoint,
                    conn_config_data,
                    status: link_status,
                    status_msg: l.status_msg,
                    deleted: l.status == crate::db::LinkStatus::Deleted,
                    last_updated,
                };
                if tx.send(Ok(entry)).await.is_err() {
                    return;
                }
            }
            // Emit pending topology links.
            // For pending entries, source_node_id/dest_node_id hold group names
            // (not node IDs) since no physical link exists yet.
            for (src, dst) in pending_entries {
                let entry = LinkEntry {
                    id: format!("pending|{}|{}", src, dst),
                    link_id: "-".to_string(),
                    source_node_id: src,
                    dest_node_id: dst,
                    dest_endpoint: "-".to_string(),
                    conn_config_data: String::new(),
                    status: LinkStatus::Pending as i32,
                    status_msg: String::new(),
                    deleted: false,
                    last_updated: 0,
                };
                if tx.send(Ok(entry)).await.is_err() {
                    return;
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
        let warnings = self
            .route_service
            .add_topology_link(&req.group_a, &req.group_b, segment)
            .await?;
        Ok(Response::new(AddTopologyLinkResponse { warnings }))
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

    // ── Registration auth group management ─────────────────────────────────

    async fn list_groups(
        &self,
        _request: Request<ListGroupsRequest>,
    ) -> Result<Response<ListGroupsResponse>, Status> {
        // Start with groups from the live authenticator (covers both config and API mode).
        let mut groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for g in self.authenticator.configured_groups() {
            groups.entry(g).or_default();
        }

        // Also include groups from DB secrets (API mode — may have groups not yet
        // loaded into the authenticator on error, or for consistency).
        let auth_groups = self
            .db
            .list_registration_secret_groups()
            .await
            .map_err(|e| Status::internal(format!("failed to list groups: {e}")))?;
        for g in auth_groups {
            groups.entry(g).or_default();
        }

        // Add connected nodes grouped by group_name.
        let nodes = self
            .db
            .list_nodes()
            .await
            .map_err(|e| Status::internal(format!("failed to list nodes: {e}")))?;
        for node in nodes {
            if let Some(group_name) = node.group_name {
                groups.entry(group_name).or_default().push(node.id);
            }
        }

        let entries = groups
            .into_iter()
            .map(|(group_name, nodes)| GroupEntry { group_name, nodes })
            .collect();
        Ok(Response::new(ListGroupsResponse { groups: entries }))
    }

    async fn add_group(
        &self,
        request: Request<AddGroupRequest>,
    ) -> Result<Response<AddGroupResponse>, Status> {
        self.route_service.ensure_api_mode()?;

        if !self.authenticator.is_shared_secret() {
            return Err(Status::unimplemented(
                "add_group is only supported in shared_secret auth mode",
            ));
        }

        let req = request.get_ref();
        if req.group_name.is_empty() {
            return Err(Status::invalid_argument("group_name must not be empty"));
        }
        if req.secret.is_empty() {
            return Err(Status::invalid_argument("secret must not be empty"));
        }

        // Build the verifier first (validates the secret is usable).
        self.authenticator
            .add_verifier(&req.group_name, &req.secret)?;

        // Persist to DB only after the verifier was successfully built.
        if let Err(e) = self
            .db
            .upsert_registration_secret(&req.group_name, &req.secret)
            .await
        {
            // Roll back the in-memory verifier so the caller's error is consistent.
            let _ = self.authenticator.remove_verifier(&req.group_name);
            return Err(Status::internal(format!("failed to store secret: {e}")));
        }

        tracing::info!("add_group: added group '{}'", req.group_name);
        Ok(Response::new(AddGroupResponse {}))
    }

    async fn remove_group(
        &self,
        request: Request<RemoveGroupRequest>,
    ) -> Result<Response<RemoveGroupResponse>, Status> {
        self.route_service.ensure_api_mode()?;

        let req = request.get_ref();
        if req.group_name.is_empty() {
            return Err(Status::invalid_argument("group_name must not be empty"));
        }

        if !self.authenticator.is_shared_secret() {
            return Err(Status::unimplemented(
                "remove_group is only supported in shared_secret auth mode",
            ));
        }

        // Remove the verifier first to prevent new registrations.
        self.authenticator.remove_verifier(&req.group_name)?;

        // Remove the secret from DB (won't survive restart).
        self.db
            .delete_registration_secret(&req.group_name)
            .await
            .map_err(|e| Status::internal(format!("failed to delete secret: {e}")))?;

        // Now list nodes — no new nodes can register for this group since the
        // verifier is already removed.
        let nodes = self
            .db
            .list_nodes()
            .await
            .map_err(|e| Status::internal(format!("failed to list nodes: {e}")))?;
        let group_nodes: Vec<_> = nodes
            .iter()
            .filter(|n| n.group_name.as_deref() == Some(&req.group_name))
            .collect();

        // Disconnect and deregister each existing node in the group.
        for node in &group_nodes {
            self.cmd_handler.force_remove_stream(&node.id).await;
            self.route_service.node_deregistered(&node.id).await;
        }

        tracing::info!(
            "remove_group: removed group '{}' ({} node(s) disconnected)",
            req.group_name,
            group_nodes.len()
        );
        Ok(Response::new(RemoveGroupResponse {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdjacencyEntry, ReconcilerConfig, TopologyConfig};
    use crate::db::inmemory::InMemoryDb;
    use crate::db::{Link, LinkStatus};
    use crate::node_transport::DefaultNodeCommandHandler;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn make_link(
        link_id: &str,
        src_node: &str,
        src_group: &str,
        dst_node: &str,
        dst_group: &str,
    ) -> Link {
        Link {
            link_id: link_id.to_string(),
            source_node_id: src_node.to_string(),
            source_group: src_group.to_string(),
            dest_node_id: dst_node.to_string(),
            dest_group: dst_group.to_string(),
            dest_endpoint: "http://127.0.0.1:9000".to_string(),
            conn_config_data: slim_config::client::ServerConnectionConfig::default(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        }
    }

    /// Create a NorthboundApiService with an in-memory DB and the given topology.
    fn make_nb_service(
        topology: TopologyConfig,
        authenticator: GroupAuthenticator,
    ) -> NorthboundApiService {
        let db = InMemoryDb::shared();
        let cmd_handler = DefaultNodeCommandHandler::new();
        let route_service = RouteService::new(
            db.clone(),
            cmd_handler.clone(),
            ReconcilerConfig {
                max_requeues: 3,
                ..Default::default()
            },
            topology,
        );
        NorthboundApiService::new(db, cmd_handler, route_service, authenticator)
    }

    fn shared_secret_authenticator() -> GroupAuthenticator {
        GroupAuthenticator::SharedSecret {
            verifiers: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    fn config_managed_topology() -> TopologyConfig {
        TopologyConfig::Links(vec![AdjacencyEntry {
            group: "*".to_string(),
            neighbors: vec!["*".to_string()],
        }])
    }

    #[test]
    fn build_link_peer_map_normal_prefix() {
        // node-a is source, peer should be "group-b/node-b"
        let links = vec![make_link("l1", "node-a", "group-a", "node-b", "group-b")];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "group-b/node-b");
    }

    #[test]
    fn build_link_peer_map_already_prefixed() {
        // dest_node_id already has "group-b/" prefix — should not double it
        let links = vec![make_link(
            "l1",
            "node-a",
            "group-a",
            "group-b/node-b",
            "group-b",
        )];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "group-b/node-b");
    }

    #[test]
    fn build_link_peer_map_empty_group() {
        // Empty dest_group — should return dest_node_id as-is
        let links = vec![make_link("l1", "node-a", "group-a", "node-b", "")];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "node-b");
    }

    #[test]
    fn build_link_peer_map_reverse_direction() {
        // node-b is the current node (dest in the link), peer should be source
        let links = vec![make_link("l1", "node-a", "group-a", "node-b", "group-b")];
        let map = NorthboundApiService::build_link_peer_map("node-b", &links);
        assert_eq!(map.get("l1").unwrap(), "group-a/node-a");
    }

    // ── Group management unit tests ───────────────────────────────────────────

    #[tokio::test]
    async fn add_group_rejects_in_config_mode() {
        let svc = make_nb_service(config_managed_topology(), shared_secret_authenticator());
        let err = svc
            .add_group(Request::new(AddGroupRequest {
                group_name: "test-group".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject in config mode");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn add_group_rejects_empty_group_name() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let err = svc
            .add_group(Request::new(AddGroupRequest {
                group_name: "".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject empty group name");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("group_name"));
    }

    #[tokio::test]
    async fn add_group_rejects_empty_secret() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let err = svc
            .add_group(Request::new(AddGroupRequest {
                group_name: "test-group".to_string(),
                secret: "".to_string(),
            }))
            .await
            .expect_err("should reject empty secret");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("secret"));
    }

    #[tokio::test]
    async fn add_group_succeeds_and_persists() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let secret = "my-secret-0123456789abcdefghijklmnopqrstuv";

        // Add the group.
        svc.add_group(Request::new(AddGroupRequest {
            group_name: "new-group".to_string(),
            secret: secret.to_string(),
        }))
        .await
        .expect("add_group should succeed");

        // Verify it persisted in DB.
        let groups = svc.db.list_registration_secret_groups().await.unwrap();
        assert!(groups.contains(&"new-group".to_string()));

        // Verify the verifier was added (the authenticator can verify for this group).
        assert!(svc.authenticator.is_shared_secret());
        if let GroupAuthenticator::SharedSecret { verifiers } = &svc.authenticator {
            assert!(verifiers.read().contains_key("new-group"));
        }
    }

    #[tokio::test]
    async fn remove_group_rejects_in_config_mode() {
        let svc = make_nb_service(config_managed_topology(), shared_secret_authenticator());
        let err = svc
            .remove_group(Request::new(RemoveGroupRequest {
                group_name: "test-group".to_string(),
            }))
            .await
            .expect_err("should reject in config mode");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn remove_group_nonexistent_succeeds() {
        // Removing a group that doesn't exist should still succeed (idempotent).
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        svc.remove_group(Request::new(RemoveGroupRequest {
            group_name: "nonexistent".to_string(),
        }))
        .await
        .expect("remove_group for nonexistent group should succeed");
    }

    #[tokio::test]
    async fn remove_group_disconnects_nodes() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());

        // Register a node in the group via the DB directly.
        let node = crate::db::Node {
            id: "my-group/node-1".to_string(),
            group_name: Some("my-group".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        svc.db.save_node(node).await.unwrap();

        // Add a secret so remove_group can clean up the verifier.
        svc.db
            .upsert_registration_secret("my-group", "secret-0123456789abcdefghijklmnopqrstuv")
            .await
            .unwrap();
        svc.authenticator
            .add_verifier("my-group", "secret-0123456789abcdefghijklmnopqrstuv")
            .unwrap();

        // Remove the group.
        svc.remove_group(Request::new(RemoveGroupRequest {
            group_name: "my-group".to_string(),
        }))
        .await
        .expect("remove_group should succeed");

        // Node should be deregistered (removed from DB).
        let nodes = svc.db.list_nodes().await.unwrap();
        assert!(
            !nodes.iter().any(|n| n.id == "my-group/node-1"),
            "node should have been deregistered"
        );

        // Verifier should be removed.
        if let GroupAuthenticator::SharedSecret { verifiers } = &svc.authenticator {
            assert!(!verifiers.read().contains_key("my-group"));
        }

        // Secret should be removed from DB.
        let groups = svc.db.list_registration_secret_groups().await.unwrap();
        assert!(!groups.contains(&"my-group".to_string()));
    }

    #[tokio::test]
    async fn list_groups_merges_auth_and_connected() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());

        // Add a group secret (no nodes connected yet).
        svc.db
            .upsert_registration_secret("empty-group", "secret-0123456789abcdefgh")
            .await
            .unwrap();

        // Add a node belonging to a different group (that also has a secret).
        svc.db
            .upsert_registration_secret("active-group", "secret-0123456789xyzxyzxyz")
            .await
            .unwrap();
        let node = crate::db::Node {
            id: "active-group/node-a".to_string(),
            group_name: Some("active-group".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        svc.db.save_node(node).await.unwrap();

        // Call list_groups.
        let resp = svc
            .list_groups(Request::new(ListGroupsRequest {}))
            .await
            .expect("list_groups should succeed")
            .into_inner();

        // Should have both groups.
        assert_eq!(resp.groups.len(), 2);

        let empty = resp
            .groups
            .iter()
            .find(|g| g.group_name == "empty-group")
            .unwrap();
        assert!(empty.nodes.is_empty(), "empty-group should have no nodes");

        let active = resp
            .groups
            .iter()
            .find(|g| g.group_name == "active-group")
            .unwrap();
        assert_eq!(active.nodes, vec!["active-group/node-a"]);
    }

    #[tokio::test]
    async fn add_group_rejects_noop_authenticator() {
        // When auth is Noop (no auth configured), add_group should fail.
        let svc = make_nb_service(TopologyConfig::ApiManaged, GroupAuthenticator::Noop);
        let err = svc
            .add_group(Request::new(AddGroupRequest {
                group_name: "test".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject when auth is Noop");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}

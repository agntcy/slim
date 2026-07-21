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
    AddDomainRequest, AddDomainResponse, AddSegmentRequest, AddSegmentResponse,
    AddTopologyLinkRequest, AddTopologyLinkResponse, DomainEntry, LinkEntry, LinkListRequest,
    LinkStatus, ListDomainsRequest, ListDomainsResponse, NodeEntry, NodeListRequest,
    RemoveDomainRequest, RemoveDomainResponse, RemoveSegmentRequest, RemoveSegmentResponse,
    RemoveTopologyLinkRequest, RemoveTopologyLinkResponse, RouteEntry, RouteListRequest,
    RouteStatus, SegmentEdge, SegmentEntry, SegmentListRequest, SegmentListResponse,
    control_plane_service_server::ControlPlaneService,
};
use crate::auth::DomainAuthenticator;
use crate::db::{SharedDb, model};
use crate::node_transport::{DefaultNodeCommandHandler, NodeStatus};
use crate::route_service::RouteService;
use crate::types::DEFAULT_SEGMENT;

pub struct NorthboundApiService {
    db: SharedDb,
    cmd_handler: DefaultNodeCommandHandler,
    route_service: RouteService,
    authenticator: DomainAuthenticator,
}

impl NorthboundApiService {
    pub fn new(
        db: SharedDb,
        cmd_handler: DefaultNodeCommandHandler,
        route_service: RouteService,
        authenticator: DomainAuthenticator,
    ) -> Self {
        Self {
            db,
            cmd_handler,
            route_service,
            authenticator,
        }
    }

    /// Build a map of link_id → qualified peer name (domain/node) for a given node.
    fn build_link_peer_map(
        node_id: &str,
        links: &[crate::db::Link],
    ) -> std::collections::HashMap<String, String> {
        links
            .iter()
            .map(|link| {
                let peer = if link.source_node_id == node_id {
                    if link.dest_domain.is_empty()
                        || link
                            .dest_node_id
                            .starts_with(&format!("{}/", link.dest_domain))
                    {
                        link.dest_node_id.clone()
                    } else {
                        format!("{}/{}", link.dest_domain, link.dest_node_id)
                    }
                } else if link.source_domain.is_empty()
                    || link
                        .source_node_id
                        .starts_with(&format!("{}/", link.source_domain))
                {
                    link.source_node_id.clone()
                } else {
                    format!("{}/{}", link.source_domain, link.source_node_id)
                };
                (link.link_id.clone(), peer)
            })
            .collect()
    }

    /// Enrich `peer_node_id` on connection entries using domain information.
    fn enrich_peer_node_ids(
        entries: &mut [crate::api::proto::controller::proto::v1::ConnectionEntry],
        node_domain: &str,
        link_peer_map: &std::collections::HashMap<String, String>,
    ) {
        use crate::api::proto::controller::proto::v1::ConnectionType;
        for entry in entries.iter_mut() {
            match entry.connection_type() {
                ConnectionType::Peer => {
                    if let Some(bare_id) = entry.peer_node_id.take() {
                        if bare_id.starts_with(&format!("{}/", node_domain)) {
                            entry.peer_node_id = Some(bare_id);
                        } else {
                            entry.peer_node_id = Some(format!("{}/{}", node_domain, bare_id));
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
    /// Returns `(node_domain, link_peer_map)`. Logs a warning on DB errors
    /// and continues with an empty map (enrichment is best-effort).
    async fn enrichment_context(
        &self,
        node: &crate::db::Node,
        caller: &str,
    ) -> (String, std::collections::HashMap<String, String>) {
        let node_domain = node.domain_name.clone().unwrap_or_default();
        let links = self
            .db
            .get_links_for_node(&node.id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("{caller}: failed to fetch links for enrichment: {e}");
                Vec::new()
            });
        let link_peer_map = Self::build_link_peer_map(&node.id, &links);
        (node_domain, link_peer_map)
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

                let domain = node.domain_name.unwrap_or_default();
                let entry = NodeEntry {
                    id: node.id,
                    connections,
                    status,
                    domain,
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

        // Enrich peer_node_id with domain prefix (same logic as list_connections).
        let (node_domain, link_peer_map) = self.enrichment_context(&node, "list_node_routes").await;

        for entry in &mut resp.entries {
            Self::enrich_peer_node_ids(&mut entry.connections, &node_domain, &link_peer_map);
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

        // Enrich peer_node_id with domain prefix using link information.
        let (node_domain, link_peer_map) = self.enrichment_context(&node, "list_connections").await;
        Self::enrich_peer_node_ids(&mut resp.entries, &node_domain, &link_peer_map);

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

        // Collect topology link domain pairs to identify pending ones.
        let segments = self.route_service.list_segments().await;
        let mut topology_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for (_name, _groups, edges) in &segments {
            for (src, dst) in edges {
                topology_pairs.insert((src.clone(), dst.clone()));
                topology_pairs.insert((dst.clone(), src.clone()));
            }
        }

        // Track which domain pairs have physical links.
        let mut realized_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        for l in &links {
            if !l.source_domain.is_empty() && !l.dest_domain.is_empty() {
                realized_pairs.insert((l.source_domain.clone(), l.dest_domain.clone()));
                realized_pairs.insert((l.dest_domain.clone(), l.source_domain.clone()));
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
            // For pending entries, source_node_id/dest_node_id hold domain names
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
            .map(|(name, domains, edges)| SegmentEntry {
                name,
                domains,
                edges: edges
                    .into_iter()
                    .map(|(a, b)| SegmentEdge {
                        domain_a: a,
                        domain_b: b,
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
        if req.domain_a.is_empty() || req.domain_b.is_empty() {
            return Err(Status::invalid_argument(
                "domain_a and domain_b must not be empty",
            ));
        }
        let segment = if req.segment.is_empty() {
            DEFAULT_SEGMENT
        } else {
            &req.segment
        };
        let warnings = self
            .route_service
            .add_topology_link(&req.domain_a, &req.domain_b, segment)
            .await?;
        Ok(Response::new(AddTopologyLinkResponse { warnings }))
    }

    async fn remove_topology_link(
        &self,
        request: Request<RemoveTopologyLinkRequest>,
    ) -> Result<Response<RemoveTopologyLinkResponse>, Status> {
        let req = request.get_ref();
        if req.domain_a.is_empty() || req.domain_b.is_empty() {
            return Err(Status::invalid_argument(
                "domain_a and domain_b must not be empty",
            ));
        }
        let segment = if req.segment.is_empty() {
            DEFAULT_SEGMENT
        } else {
            &req.segment
        };
        self.route_service
            .remove_topology_link(&req.domain_a, &req.domain_b, segment)
            .await?;
        Ok(Response::new(RemoveTopologyLinkResponse {}))
    }

    // ── Registration auth domain management ─────────────────────────────────

    async fn list_domains(
        &self,
        _request: Request<ListDomainsRequest>,
    ) -> Result<Response<ListDomainsResponse>, Status> {
        // Start with domains from the live authenticator (covers both config and API mode).
        let mut domains: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for g in self.authenticator.configured_domains() {
            domains.entry(g).or_default();
        }

        // Also include domains from DB secrets (API mode — may have domains not yet
        // loaded into the authenticator on error, or for consistency).
        let auth_groups = self
            .db
            .list_registration_secret_groups()
            .await
            .map_err(|e| Status::internal(format!("failed to list domains: {e}")))?;
        for g in auth_groups {
            domains.entry(g).or_default();
        }

        // Add connected nodes grouped by domain_name.
        let nodes = self
            .db
            .list_nodes()
            .await
            .map_err(|e| Status::internal(format!("failed to list nodes: {e}")))?;
        for node in nodes {
            if let Some(domain_name) = node.domain_name {
                domains.entry(domain_name).or_default().push(node.id);
            }
        }

        let entries = domains
            .into_iter()
            .map(|(domain_name, nodes)| DomainEntry { domain_name, nodes })
            .collect();
        Ok(Response::new(ListDomainsResponse { domains: entries }))
    }

    async fn add_domain(
        &self,
        request: Request<AddDomainRequest>,
    ) -> Result<Response<AddDomainResponse>, Status> {
        self.route_service.ensure_api_mode()?;

        if !self.authenticator.is_shared_secret() {
            return Err(Status::unimplemented(
                "add_domain is only supported in shared_secret auth mode",
            ));
        }

        let req = request.get_ref();
        if req.domain_name.is_empty() {
            return Err(Status::invalid_argument("domain_name must not be empty"));
        }
        if req.secret.is_empty() {
            return Err(Status::invalid_argument("secret must not be empty"));
        }

        // Build the verifier first (validates the secret is usable).
        self.authenticator
            .add_verifier(&req.domain_name, &req.secret)?;

        // Persist to DB only after the verifier was successfully built.
        if let Err(e) = self
            .db
            .upsert_registration_secret(&req.domain_name, &req.secret)
            .await
        {
            // Roll back the in-memory verifier so the caller's error is consistent.
            let _ = self.authenticator.remove_verifier(&req.domain_name);
            return Err(Status::internal(format!("failed to store secret: {e}")));
        }

        tracing::info!("add_domain: added domain '{}'", req.domain_name);
        Ok(Response::new(AddDomainResponse {}))
    }

    async fn remove_domain(
        &self,
        request: Request<RemoveDomainRequest>,
    ) -> Result<Response<RemoveDomainResponse>, Status> {
        self.route_service.ensure_api_mode()?;

        let req = request.get_ref();
        if req.domain_name.is_empty() {
            return Err(Status::invalid_argument("domain_name must not be empty"));
        }

        if !self.authenticator.is_shared_secret() {
            return Err(Status::unimplemented(
                "remove_domain is only supported in shared_secret auth mode",
            ));
        }

        // Remove the verifier first to prevent new registrations.
        self.authenticator.remove_verifier(&req.domain_name)?;

        // Remove the secret from DB (won't survive restart).
        self.db
            .delete_registration_secret(&req.domain_name)
            .await
            .map_err(|e| Status::internal(format!("failed to delete secret: {e}")))?;

        // Now list nodes — no new nodes can register for this domain since the
        // verifier is already removed.
        let nodes = self
            .db
            .list_nodes()
            .await
            .map_err(|e| Status::internal(format!("failed to list nodes: {e}")))?;
        let group_nodes: Vec<_> = nodes
            .iter()
            .filter(|n| n.domain_name.as_deref() == Some(&req.domain_name))
            .collect();

        // Disconnect and deregister each existing node in the domain.
        for node in &group_nodes {
            self.cmd_handler.force_remove_stream(&node.id).await;
            self.route_service.node_deregistered(&node.id).await;
        }

        tracing::info!(
            "remove_domain: removed domain '{}' ({} node(s) disconnected)",
            req.domain_name,
            group_nodes.len()
        );
        Ok(Response::new(RemoveDomainResponse {}))
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
        src_domain: &str,
        dst_node: &str,
        dst_domain: &str,
    ) -> Link {
        Link {
            link_id: link_id.to_string(),
            source_node_id: src_node.to_string(),
            source_domain: src_domain.to_string(),
            dest_node_id: dst_node.to_string(),
            dest_domain: dst_domain.to_string(),
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
        authenticator: DomainAuthenticator,
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

    fn shared_secret_authenticator() -> DomainAuthenticator {
        DomainAuthenticator::SharedSecret {
            verifiers: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    fn config_managed_topology() -> TopologyConfig {
        TopologyConfig::Links(vec![AdjacencyEntry {
            domain: "*".to_string(),
            neighbors: vec!["*".to_string()],
        }])
    }

    #[test]
    fn build_link_peer_map_normal_prefix() {
        // node-a is source, peer should be "domain-b/node-b"
        let links = vec![make_link("l1", "node-a", "domain-a", "node-b", "domain-b")];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "domain-b/node-b");
    }

    #[test]
    fn build_link_peer_map_already_prefixed() {
        // dest_node_id already has "domain-b/" prefix — should not double it
        let links = vec![make_link(
            "l1",
            "node-a",
            "domain-a",
            "domain-b/node-b",
            "domain-b",
        )];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "domain-b/node-b");
    }

    #[test]
    fn build_link_peer_map_empty_group() {
        // Empty dest_domain — should return dest_node_id as-is
        let links = vec![make_link("l1", "node-a", "domain-a", "node-b", "")];
        let map = NorthboundApiService::build_link_peer_map("node-a", &links);
        assert_eq!(map.get("l1").unwrap(), "node-b");
    }

    #[test]
    fn build_link_peer_map_reverse_direction() {
        // node-b is the current node (dest in the link), peer should be source
        let links = vec![make_link("l1", "node-a", "domain-a", "node-b", "domain-b")];
        let map = NorthboundApiService::build_link_peer_map("node-b", &links);
        assert_eq!(map.get("l1").unwrap(), "domain-a/node-a");
    }

    // ── Group management unit tests ───────────────────────────────────────────

    #[tokio::test]
    async fn add_domain_rejects_in_config_mode() {
        let svc = make_nb_service(config_managed_topology(), shared_secret_authenticator());
        let err = svc
            .add_domain(Request::new(AddDomainRequest {
                domain_name: "test-domain".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject in config mode");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn add_domain_rejects_empty_domain_name() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let err = svc
            .add_domain(Request::new(AddDomainRequest {
                domain_name: "".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject empty domain name");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("domain_name"));
    }

    #[tokio::test]
    async fn add_domain_rejects_empty_secret() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let err = svc
            .add_domain(Request::new(AddDomainRequest {
                domain_name: "test-domain".to_string(),
                secret: "".to_string(),
            }))
            .await
            .expect_err("should reject empty secret");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("secret"));
    }

    #[tokio::test]
    async fn add_domain_succeeds_and_persists() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        let secret = "my-secret-0123456789abcdefghijklmnopqrstuv";

        // Add the domain.
        svc.add_domain(Request::new(AddDomainRequest {
            domain_name: "new-domain".to_string(),
            secret: secret.to_string(),
        }))
        .await
        .expect("add_domain should succeed");

        // Verify it persisted in DB.
        let domains = svc.db.list_registration_secret_groups().await.unwrap();
        assert!(domains.contains(&"new-domain".to_string()));

        // Verify the verifier was added (the authenticator can verify for this domain).
        assert!(svc.authenticator.is_shared_secret());
        if let DomainAuthenticator::SharedSecret { verifiers } = &svc.authenticator {
            assert!(verifiers.read().contains_key("new-domain"));
        }
    }

    #[tokio::test]
    async fn remove_domain_rejects_in_config_mode() {
        let svc = make_nb_service(config_managed_topology(), shared_secret_authenticator());
        let err = svc
            .remove_domain(Request::new(RemoveDomainRequest {
                domain_name: "test-domain".to_string(),
            }))
            .await
            .expect_err("should reject in config mode");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn remove_domain_nonexistent_succeeds() {
        // Removing a domain that doesn't exist should still succeed (idempotent).
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());
        svc.remove_domain(Request::new(RemoveDomainRequest {
            domain_name: "nonexistent".to_string(),
        }))
        .await
        .expect("remove_domain for nonexistent domain should succeed");
    }

    #[tokio::test]
    async fn remove_domain_disconnects_nodes() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());

        // Register a node in the domain via the DB directly.
        let node = crate::db::Node {
            id: "my-domain/node-1".to_string(),
            domain_name: Some("my-domain".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        svc.db.save_node(node).await.unwrap();

        // Add a secret so remove_domain can clean up the verifier.
        svc.db
            .upsert_registration_secret("my-domain", "secret-0123456789abcdefghijklmnopqrstuv")
            .await
            .unwrap();
        svc.authenticator
            .add_verifier("my-domain", "secret-0123456789abcdefghijklmnopqrstuv")
            .unwrap();

        // Remove the domain.
        svc.remove_domain(Request::new(RemoveDomainRequest {
            domain_name: "my-domain".to_string(),
        }))
        .await
        .expect("remove_domain should succeed");

        // Node should be deregistered (removed from DB).
        let nodes = svc.db.list_nodes().await.unwrap();
        assert!(
            !nodes.iter().any(|n| n.id == "my-domain/node-1"),
            "node should have been deregistered"
        );

        // Verifier should be removed.
        if let DomainAuthenticator::SharedSecret { verifiers } = &svc.authenticator {
            assert!(!verifiers.read().contains_key("my-domain"));
        }

        // Secret should be removed from DB.
        let domains = svc.db.list_registration_secret_groups().await.unwrap();
        assert!(!domains.contains(&"my-domain".to_string()));
    }

    #[tokio::test]
    async fn list_domains_merges_auth_and_connected() {
        let svc = make_nb_service(TopologyConfig::ApiManaged, shared_secret_authenticator());

        // Add a domain secret (no nodes connected yet).
        svc.db
            .upsert_registration_secret("empty-domain", "secret-0123456789abcdefgh")
            .await
            .unwrap();

        // Add a node belonging to a different domain (that also has a secret).
        svc.db
            .upsert_registration_secret("active-domain", "secret-0123456789xyzxyzxyz")
            .await
            .unwrap();
        let node = crate::db::Node {
            id: "active-domain/node-a".to_string(),
            domain_name: Some("active-domain".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        svc.db.save_node(node).await.unwrap();

        // Call list_domains.
        let resp = svc
            .list_domains(Request::new(ListDomainsRequest {}))
            .await
            .expect("list_domains should succeed")
            .into_inner();

        // Should have both domains.
        assert_eq!(resp.domains.len(), 2);

        let empty = resp
            .domains
            .iter()
            .find(|g| g.domain_name == "empty-domain")
            .unwrap();
        assert!(empty.nodes.is_empty(), "empty-domain should have no nodes");

        let active = resp
            .domains
            .iter()
            .find(|g| g.domain_name == "active-domain")
            .unwrap();
        assert_eq!(active.nodes, vec!["active-domain/node-a"]);
    }

    #[tokio::test]
    async fn add_domain_rejects_noop_authenticator() {
        // When auth is Noop (no auth configured), add_domain should fail.
        let svc = make_nb_service(TopologyConfig::ApiManaged, DomainAuthenticator::Noop);
        let err = svc
            .add_domain(Request::new(AddDomainRequest {
                domain_name: "test".to_string(),
                secret: "secret-0123456789abcdefghijk".to_string(),
            }))
            .await
            .expect_err("should reject when auth is Noop");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}

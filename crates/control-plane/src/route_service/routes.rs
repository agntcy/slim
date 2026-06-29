// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use petgraph::visit::EdgeRef;

#[cfg(test)]
use crate::api::proto::controller::proto::v1::Route as ProtoRoute;
use crate::db::{LinkStatus, Route, RouteStatus};
use crate::error::{Error, Result};

use super::spt;
use super::*;

/// Build a route struct for a gateway node in SPT expansion.
#[allow(clippy::too_many_arguments)]
fn build_route_for_gateway(
    source_node_id: &str,
    source_group: &str,
    dest_node_id: &str,
    dest_group: &str,
    link_id: String,
    component0: &str,
    component1: &str,
    component2: &str,
    component_id: Option<&str>,
) -> Route {
    Route {
        id: String::new(),
        source_node_id: source_node_id.to_string(),
        source_group: source_group.to_string(),
        dest_node_id: dest_node_id.to_string(),
        dest_group: dest_group.to_string(),
        link_id: Some(link_id),
        component0: component0.to_string(),
        component1: component1.to_string(),
        component2: component2.to_string(),
        component_id: component_id.map(|s| s.to_string()),
        status: RouteStatus::Pending,
        status_msg: String::new(),
        created_at: SystemTime::now(),
        last_updated: SystemTime::now(),
    }
}

impl super::RouteService {
    /// Rebuild the runtime segment graphs from the config-defined topology.
    ///
    /// **Config mode only.** Extracts distinct group names from registered nodes,
    /// expands `$group` templates in segment configs, and rebuilds one graph per
    /// segment. Called on every node register/deregister so that dynamic `$group`
    /// expansion picks up new groups.
    ///
    /// Returns `true` if the set of groups changed (a group was added or removed),
    /// `false` if unchanged or if running in API mode.
    ///
    /// **API mode:** returns `false` immediately — segment graphs are managed
    /// exclusively via topology mutation APIs (`add_link_to_segment`, etc.) and
    /// loaded from DB on startup. Node registration does not alter topology.
    pub(super) async fn rebuild_link_graph(&self, nodes: &[crate::db::Node]) -> bool {
        if self.0.topology.is_api_managed() {
            return false;
        }

        let new_groups: HashSet<&str> = nodes
            .iter()
            .map(|n| n.group_name.as_deref().unwrap_or(""))
            .collect();

        let mut current_segments = self.0.segment_graphs.write().await;
        let current_groups: HashSet<&str> = current_segments
            .iter()
            .flat_map(|(_, g)| g.node_indices().map(|idx| g[idx].as_str()))
            .collect();
        let groups_changed = new_groups != current_groups;

        if groups_changed {
            let group_vec: Vec<&str> = new_groups.into_iter().collect();
            *current_segments = self.0.topology.build_graph(&group_vec);

            // Persist the expanded topology to DB so that API mode can pick it
            // up if the operator later switches modes.
            self.persist_segments_to_db(&current_segments).await;
        }

        groups_changed
    }

    /// Write the current segment graphs to the DB (topology_segments +
    /// topology_segment_links). Called after config-mode graph expansion so
    /// the DB always reflects the latest resolved topology.
    async fn persist_segments_to_db(
        &self,
        segments: &[(String, petgraph::graph::UnGraph<String, u32>)],
    ) {
        for (seg_name, graph) in segments {
            let seg = match self.0.db.get_segment_by_name(seg_name).await {
                Ok(Some(s)) => s,
                Ok(None) => match self.0.db.create_segment(seg_name).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(%seg_name, error = %e, "failed to persist segment");
                        continue;
                    }
                },
                Err(e) => {
                    tracing::error!(%seg_name, error = %e, "failed to check segment");
                    continue;
                }
            };

            for edge in graph.edge_references() {
                let group_a = &graph[edge.source()];
                let group_b = &graph[edge.target()];
                if let Err(e) = self
                    .0
                    .db
                    .add_link_to_segment(&seg.id, group_a, group_b)
                    .await
                {
                    tracing::debug!(
                        %seg_name, %group_a, %group_b, error = %e,
                        "failed to persist topology link (may already exist)"
                    );
                }
            }
        }
    }

    /// Compute the set of allowed group pairs from the runtime segment graphs.
    /// Two groups can link if they share an edge in any segment.
    pub(super) async fn allowed_link_pairs(&self) -> HashSet<(String, String)> {
        let segments = self.0.segment_graphs.read().await;
        let mut pairs = HashSet::new();
        for (_, graph) in segments.iter() {
            for edge in graph.edge_references() {
                let a = &graph[edge.source()];
                let b = &graph[edge.target()];
                // Store both directions for O(1) lookup
                pairs.insert((a.clone(), b.clone()));
                pairs.insert((b.clone(), a.clone()));
            }
        }
        pairs
    }

    /// Return the current segment graphs as (name, groups, edges) tuples.
    /// Only groups that participate in at least one edge are included.
    pub async fn list_segments(&self) -> Vec<(String, Vec<String>, Vec<(String, String)>)> {
        let segments = self.0.segment_graphs.read().await;
        segments
            .iter()
            .map(|(name, graph)| {
                let mut edges: Vec<(String, String)> = graph
                    .edge_references()
                    .map(|e| {
                        let a = graph[e.source()].clone();
                        let b = graph[e.target()].clone();
                        if a <= b { (a, b) } else { (b, a) }
                    })
                    .collect();
                edges.sort();
                edges.dedup();
                let mut groups: Vec<String> = {
                    let mut connected = std::collections::HashSet::new();
                    for (a, b) in &edges {
                        connected.insert(a.clone());
                        connected.insert(b.clone());
                    }
                    connected.into_iter().collect()
                };
                groups.sort();
                (name.clone(), groups, edges)
            })
            .collect()
    }

    /// Find the inter-group link between two groups using pre-loaded links.
    ///
    /// Searches for an existing (non-deleted) link between any node in `group_a`
    /// and any node in `group_b`. Returns the node_id in `group_a` (the gateway)
    /// and the link_id connecting them.
    pub(super) fn find_inter_group_link_from_cache(
        group_a: &str,
        group_b: &str,
        all_nodes: &[crate::db::Node],
        all_links: &[crate::db::Link],
    ) -> Option<(String, String)> {
        let nodes_a: HashSet<&str> = all_nodes
            .iter()
            .filter(|n| n.group_name.as_deref() == Some(group_a))
            .map(|n| n.id.as_str())
            .collect();
        let nodes_b: HashSet<&str> = all_nodes
            .iter()
            .filter(|n| n.group_name.as_deref() == Some(group_b))
            .map(|n| n.id.as_str())
            .collect();

        // Find an established link connecting a node in group_a to a node in group_b.
        // Only Applied links with a known dest_node_id are usable for routing.
        for link in all_links {
            if link.status != LinkStatus::Applied || link.dest_node_id.is_empty() {
                continue;
            }
            if nodes_a.contains(link.source_node_id.as_str())
                && nodes_b.contains(link.dest_node_id.as_str())
            {
                return Some((link.source_node_id.clone(), link.link_id.clone()));
            }
            if nodes_b.contains(link.source_node_id.as_str())
                && nodes_a.contains(link.dest_node_id.as_str())
            {
                // Link goes B→A; the gateway in group_a is the dest side.
                return Some((link.dest_node_id.clone(), link.link_id.clone()));
            }
        }
        None
    }

    /// Expand a wildcard route using the Shortest Path Tree.
    ///
    /// Given a route template (dest_node_id + name components), computes the SPT
    /// rooted at the destination's group. For each non-root group in the tree,
    /// selects a gateway node and installs a route pointing toward the parent group.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn expand_route_via_spt(
        &self,
        dest_node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
        all_nodes: &[crate::db::Node],
        all_links: &[crate::db::Link],
    ) {
        // Resolve the destination node's group — this is the root of the SPT.
        let dest_group = all_nodes
            .iter()
            .find(|n| n.id == dest_node_id)
            .and_then(|n| n.group_name.as_deref())
            .unwrap_or("");

        // Clone the segment graphs to release the read lock before the async loop,
        // preventing potential deadlocks with concurrent write lock acquisitions.
        // The snapshot may become stale if a concurrent registration from a different
        // group triggers rebuild_link_graph. This is safe: missing routes will be
        // installed when the other group's handler runs expand_all_wildcard_routes,
        // and stale routes are cleaned up by handle_disconnect.
        let segment_graphs = self.0.segment_graphs.read().await.clone();

        for (_seg_name, graph) in &segment_graphs {
            // If the link graph has no inter-group edges, there is nothing for the
            // control plane to expand (same-group routing is handled by the data plane).
            if graph.node_count() <= 1 {
                continue;
            }

            let root_idx = match graph.node_indices().find(|&idx| graph[idx] == dest_group) {
                Some(idx) => idx,
                None => continue,
            };

            // Compute the SPT rooted at the destination group.
            let spt = match spt::compute_spt(root_idx, graph) {
                Some(t) => t,
                None => continue,
            };

            // For each non-root group in the SPT, install a route on the gateway
            // node pointing toward the parent group.
            for (&orig_idx, &tree_idx) in &spt.index_map {
                if orig_idx == root_idx {
                    continue;
                }

                let child_group = &graph[orig_idx];

                // Find the parent group in the directed tree (incoming edge = from parent).
                let parent_tree_idx = match spt
                    .tree
                    .edges_directed(tree_idx, petgraph::Direction::Incoming)
                    .next()
                {
                    Some(edge) => edge.source(),
                    None => continue,
                };
                let parent_group = &spt.tree[parent_tree_idx];

                // Find the inter-group link from pre-loaded links (O(n) scan, no DB query).
                let (source_node_id, link_id) = match Self::find_inter_group_link_from_cache(
                    child_group,
                    parent_group,
                    all_nodes,
                    all_links,
                ) {
                    Some(pair) => pair,
                    None => {
                        tracing::debug!(
                            "expand_route_via_spt: no link between '{child_group}' and '{parent_group}', skipping"
                        );
                        continue;
                    }
                };

                // Create the per-gateway route pointing toward the parent group.
                let per_node = build_route_for_gateway(
                    &source_node_id,
                    child_group,
                    dest_node_id,
                    dest_group,
                    link_id,
                    component0,
                    component1,
                    component2,
                    component_id,
                );
                if let Err(e) = self.add_single_route(per_node).await {
                    tracing::warn!("expand_route_via_spt: route for {source_node_id} skipped: {e}");
                }
            }
        }
    }

    /// Install downward routes from the SPT root toward a new announcer.
    ///
    /// When a name already has an SPT (first announcer = root), subsequent
    /// announcers need routes along the path from root down to their group.
    /// Walks from the new announcer's group up to the root in the SPT and
    /// installs a route on each intermediate parent pointing toward the child.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn install_downward_path(
        &self,
        root_node_id: &str,
        new_announcer_node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
        all_nodes: &[crate::db::Node],
        all_links: &[crate::db::Link],
    ) {
        let root_group = all_nodes
            .iter()
            .find(|n| n.id == root_node_id)
            .and_then(|n| n.group_name.as_deref())
            .unwrap_or("");

        let announcer_group = all_nodes
            .iter()
            .find(|n| n.id == new_announcer_node_id)
            .and_then(|n| n.group_name.as_deref())
            .unwrap_or("");

        // Clone the segment graphs to release the read lock before the async loop.
        // See expand_route_via_spt for why staleness from concurrent registrations is safe.
        let segment_graphs = self.0.segment_graphs.read().await.clone();

        // If root and announcer are in the same group, the data plane handles
        // routing within that group — nothing for the control plane to do.
        if root_group == announcer_group {
            return;
        }

        for (_seg_name, graph) in &segment_graphs {
            if graph.node_count() <= 1 {
                continue;
            }

            let root_idx = match graph.node_indices().find(|&idx| graph[idx] == root_group) {
                Some(idx) => idx,
                None => continue,
            };
            let announcer_idx = match graph
                .node_indices()
                .find(|&idx| graph[idx] == announcer_group)
            {
                Some(idx) => idx,
                None => continue,
            };

            let spt = match spt::compute_spt(root_idx, graph) {
                Some(t) => t,
                None => continue,
            };

            // Walk from announcer up to root in the tree, collecting (parent, child) pairs.
            let mut current = match spt.index_map.get(&announcer_idx) {
                Some(&idx) => idx,
                None => continue,
            };
            let tree_root = spt.index_map[&root_idx];

            while current != tree_root {
                // Find parent (incoming edge source).
                let parent_tree_idx = match spt
                    .tree
                    .edges_directed(current, petgraph::Direction::Incoming)
                    .next()
                {
                    Some(edge) => edge.source(),
                    None => break,
                };

                let parent_group = &spt.tree[parent_tree_idx];
                let child_group = &spt.tree[current];

                // Install route on the parent group's gateway pointing toward child.
                if let Some((gateway_node_id, link_id)) = Self::find_inter_group_link_from_cache(
                    parent_group,
                    child_group,
                    all_nodes,
                    all_links,
                ) {
                    let per_node = build_route_for_gateway(
                        &gateway_node_id,
                        parent_group,
                        new_announcer_node_id,
                        announcer_group,
                        link_id,
                        component0,
                        component1,
                        component2,
                        component_id,
                    );
                    if let Err(e) = self.add_single_route(per_node).await {
                        tracing::warn!(
                            "install_downward_path: route for {gateway_node_id} skipped: {e}"
                        );
                    }
                } else {
                    tracing::debug!(
                        "install_downward_path: no link between '{parent_group}' and '{child_group}', skipping"
                    );
                }

                current = parent_tree_idx;
            }
        }
    }

    pub(super) async fn add_single_route(&self, mut db_route: Route) -> Result<String> {
        // If link_id was not pre-resolved by the caller (e.g. expand_route_via_spt),
        // try to find a direct link between source and dest. If none exists yet,
        // store with link_id=None — the reconciler will resolve it later.
        if db_route.source_node_id != ALL_NODES_ID && db_route.link_id.is_none() {
            db_route.link_id = self
                .find_matching_link(
                    &db_route.source_node_id,
                    &db_route.source_group,
                    &db_route.dest_node_id,
                    &db_route.dest_group,
                )
                .await
                .ok();
        }

        // Retry once if a stale soft-deleted route blocks insertion. The
        // get_route_by_id + delete_route + add_route sequence is not atomic, so
        // a concurrent caller can race; bound retries to 2 attempts.
        for attempt in 0..2 {
            match self.0.db.add_route(db_route.clone()).await {
                Ok(r) => {
                    let route_str = r.to_string();
                    tracing::info!("route added: {route_str}");
                    if db_route.source_node_id != ALL_NODES_ID {
                        self.0.queue.add(db_route.source_node_id);
                    }
                    return Ok(route_str);
                }
                Err(e) => {
                    let unique_id = db_route.compute_id();
                    match self.0.db.get_route_by_id(&unique_id).await? {
                        Some(existing) if existing.status == RouteStatus::Deleted => {
                            tracing::warn!(
                                "removing stale deleted route {} to allow re-add (attempt {})",
                                existing,
                                attempt + 1
                            );
                            match self.0.db.delete_route(&existing.id).await {
                                Ok(()) | Err(Error::RouteNotFound { .. }) => {}
                                Err(e) => return Err(e),
                            }
                            continue;
                        }
                        Some(existing) => {
                            // Route already exists with non-Deleted status — a concurrent
                            // caller won the race. Treat as success since the route is in place.
                            tracing::debug!("route already exists (concurrent add): {existing}");
                            return Ok(existing.to_string());
                        }
                        None => {
                            return Err(Error::InvalidInput(format!("failed to add route: {e}")));
                        }
                    }
                }
            }
        }
        Err(Error::InvalidInput(
            "failed to add route after retries".into(),
        ))
    }

    pub(super) async fn delete_single_route(
        &self,
        node_id: &str,
        route_id: &str,
        route_key: &str,
    ) -> Result<()> {
        self.0.db.mark_route_deleted(route_id).await?;
        tracing::info!("route marked for delete: {route_key}");
        if node_id != ALL_NODES_ID {
            self.0.queue.add(node_id.to_string());
        }
        Ok(())
    }

    /// Expand every wildcard route template via SPT.
    /// Called when the group topology changes (group added/removed) or when a
    /// node registers and needs its routes populated.
    /// `add_single_route` rejects duplicates so this is idempotent.
    pub(super) async fn expand_all_wildcard_routes(
        &self,
        all_nodes: &[crate::db::Node],
        all_links: &[crate::db::Link],
    ) {
        let wildcard_routes = match self.0.db.get_routes_for_node(ALL_NODES_ID).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("expand_all_wildcard_routes: failed to fetch wildcard routes: {e}");
                return;
            }
        };

        // Group wildcard routes by name. The first (oldest by created_at)
        // route for each name owns the SPT; subsequent ones get downward paths.
        // Only consider non-deleted routes for expansion.
        let mut by_name: HashMap<(String, String, String, Option<String>), Vec<&Route>> =
            HashMap::new();
        for r in wildcard_routes
            .iter()
            .filter(|r| r.status != crate::db::RouteStatus::Deleted)
        {
            let key = (
                r.component0.clone(),
                r.component1.clone(),
                r.component2.clone(),
                r.component_id.clone(),
            );
            by_name.entry(key).or_default().push(r);
        }

        for (_name, mut routes) in by_name {
            // Sort by creation time, with dest_node_id as tiebreaker for determinism
            // when timestamps have the same resolution.
            routes.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.dest_node_id.cmp(&b.dest_node_id))
            });
            let root_route = routes[0];

            // First announcer: full SPT expansion (upward routes).
            self.expand_route_via_spt(
                &root_route.dest_node_id,
                &root_route.component0,
                &root_route.component1,
                &root_route.component2,
                root_route.component_id.as_deref(),
                all_nodes,
                all_links,
            )
            .await;

            // Subsequent announcers: install downward paths from root.
            for r in &routes[1..] {
                self.install_downward_path(
                    &root_route.dest_node_id,
                    &r.dest_node_id,
                    &r.component0,
                    &r.component1,
                    &r.component2,
                    r.component_id.as_deref(),
                    all_nodes,
                    all_links,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use slim_config::conn_type::ConnType;
    use slim_config::grpc::client::ClientConfig;
    use slim_datapath::api::ProtoName;

    use super::super::test_utils::{
        make_node, make_route_service, make_route_service_with_topology, star_topology,
    };
    use super::*;
    use crate::config::{AdjacencyEntry, SegmentConfig};
    use crate::db::inmemory::InMemoryDb;

    #[tokio::test]
    async fn add_route_empty_source_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("", "dst", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("source"));
    }

    #[tokio::test]
    async fn add_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("src", "", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("destination"));
    }

    #[tokio::test]
    async fn add_route_same_src_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.add_route("node1", "node1", &route).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("same"));
    }

    // ── delete_route validation ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_route_empty_dest_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.delete_route("src", "", &route).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("destination node ID")
        );
    }

    #[tokio::test]
    async fn delete_route_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["o", "n", "t"])),
            ..Default::default()
        };
        let result = svc.delete_route("src", "dst", &route).await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn add_route_wildcard_expands_via_spt() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke_a = make_node("spoke-a", Some("customer-a"), vec![]);
        let spoke_b = make_node("spoke-b", Some("customer-b"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke_a).await.unwrap();
        db.save_node(spoke_b).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());

        // Build the link graph (normally done in node_registered).
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create inter-group links (star: hub↔spoke-a, hub↔spoke-b).
        let link_hub_a = crate::db::Link {
            link_id: "link-hub-a".to_string(),
            source_node_id: "hub-node".to_string(),
            source_group: String::new(),
            dest_node_id: "spoke-a".to_string(),
            dest_group: String::new(),
            dest_endpoint: "spoke-a:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        let link_hub_b = crate::db::Link {
            link_id: "link-hub-b".to_string(),
            source_node_id: "hub-node".to_string(),
            source_group: String::new(),
            dest_node_id: "spoke-b".to_string(),
            dest_group: String::new(),
            dest_endpoint: "spoke-b:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_hub_a).await.unwrap();
        db.add_link(link_hub_b).await.unwrap();

        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "name", "type"])),
            ..Default::default()
        };

        // Wildcard route: all nodes → spoke-b.
        // SPT rooted at "customer-b": customer-b → platform → customer-a.
        // Hub (platform) gets route via link-hub-b, spoke-a (customer-a) via link to hub.
        svc.add_route(ALL_NODES_ID, "spoke-b", &route)
            .await
            .unwrap();

        let hub_routes = db.get_routes_for_node("hub-node").await.unwrap();
        let spoke_a_routes = db.get_routes_for_node("spoke-a").await.unwrap();

        // Hub should have a route to spoke-b (parent of hub in SPT is spoke-b itself,
        // so hub is a direct child of root and gets a route via link-hub-b).
        assert!(
            hub_routes
                .iter()
                .any(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "hub-node"),
            "hub should have a route to spoke-b"
        );

        // Spoke-a should have a route to spoke-b pointing toward hub (its parent in SPT).
        assert!(
            spoke_a_routes.iter().any(|r| r.dest_node_id == "spoke-b"),
            "spoke-a should have a route to spoke-b via hub"
        );
    }

    // ── topology: SPT-based route expansion ──────────────────────────────

    #[tokio::test]
    async fn ensure_routes_spt_creates_route_via_parent() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke_a = make_node("spoke-a", Some("customer-a"), vec![]);
        let spoke_b = make_node("spoke-b", Some("customer-b"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke_a).await.unwrap();
        db.save_node(spoke_b).await.unwrap();

        // Star topology: platform links to all, spokes link only to platform.
        let topology = TopologyConfig::Links(vec![
            AdjacencyEntry {
                group: "platform".to_string(),
                neighbors: vec!["*".to_string()],
            },
            AdjacencyEntry {
                group: "*".to_string(),
                neighbors: vec!["platform".to_string()],
            },
        ]);

        let svc = make_route_service_with_topology(db.clone(), topology);

        // Add a wildcard route targeting spoke-b
        let wildcard_route = crate::db::Route {
            id: "wildcard-1".to_string(),
            source_node_id: ALL_NODES_ID.to_string(),
            source_group: String::new(),
            dest_node_id: "spoke-b".to_string(),
            dest_group: String::new(),
            link_id: None,
            component0: "org".to_string(),
            component1: "name".to_string(),
            component2: "type".to_string(),
            component_id: None,
            status: RouteStatus::Pending,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_route(wildcard_route).await.unwrap();

        // spoke-a has a link to hub-node (star topology)
        let link_a_hub = crate::db::Link {
            link_id: "link-a-hub".to_string(),
            source_node_id: "spoke-a".to_string(),
            source_group: String::new(),
            dest_node_id: "hub-node".to_string(),
            dest_group: String::new(),
            dest_endpoint: "hub:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_a_hub.clone()).await.unwrap();

        // hub-node has a link to spoke-b
        let link_hub_b = crate::db::Link {
            link_id: "link-hub-b".to_string(),
            source_node_id: "hub-node".to_string(),
            source_group: String::new(),
            dest_node_id: "spoke-b".to_string(),
            dest_group: String::new(),
            dest_endpoint: "spoke-b:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_hub_b).await.unwrap();

        // Build the link graph (normally done in node_registered).
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Expand wildcard routes via SPT.
        // SPT rooted at "customer-b": customer-b → platform → customer-a
        // So spoke-a should get a route pointing to its parent (platform/hub-node).
        let all_links = db.list_all_links().await.unwrap();
        svc.expand_all_wildcard_routes(&all_nodes, &all_links).await;

        // spoke-a should have a route to spoke-b via the hub link (toward parent).
        let spoke_a_routes = db.get_routes_for_node("spoke-a").await.unwrap();
        let transit_route = spoke_a_routes
            .iter()
            .find(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "spoke-a");
        assert!(
            transit_route.is_some(),
            "spoke-a should have a route to spoke-b"
        );
        assert_eq!(
            transit_route.unwrap().link_id.as_deref(),
            Some("link-a-hub"),
            "route should point to the hub link (parent in SPT)"
        );

        // hub-node should also have a route to spoke-b via the direct link.
        let hub_routes = db.get_routes_for_node("hub-node").await.unwrap();
        let hub_route = hub_routes
            .iter()
            .find(|r| r.dest_node_id == "spoke-b" && r.source_node_id == "hub-node");
        assert!(
            hub_route.is_some(),
            "hub-node should have a route to spoke-b"
        );
        assert_eq!(
            hub_route.unwrap().link_id.as_deref(),
            Some("link-hub-b"),
            "hub route should point directly to spoke-b"
        );
    }

    #[tokio::test]
    async fn rebuild_link_graph_returns_false_when_groups_unchanged() {
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();

        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();

        // First call: groups change (empty → {group-a, group-b}).
        assert!(svc.rebuild_link_graph(&all_nodes).await);

        // Second call with same nodes: no change.
        assert!(!svc.rebuild_link_graph(&all_nodes).await);
    }

    #[tokio::test]
    async fn reexpand_is_idempotent() {
        let db = InMemoryDb::shared();
        let hub = make_node("hub-node", Some("platform"), vec![]);
        let spoke = make_node("spoke-a", Some("customer-a"), vec![]);
        db.save_node(hub).await.unwrap();
        db.save_node(spoke).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create a link hub↔spoke.
        let link = crate::db::Link {
            link_id: "link-1".to_string(),
            source_node_id: "hub-node".to_string(),
            source_group: String::new(),
            dest_node_id: "spoke-a".to_string(),
            dest_group: String::new(),
            dest_endpoint: "spoke-a:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link).await.unwrap();

        // Add a wildcard route.
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "name", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "hub-node", &route)
            .await
            .unwrap();

        let count_before = db.get_routes_for_node("spoke-a").await.unwrap().len();

        // Expand again — should not create duplicates.
        let all_links = db.list_all_links().await.unwrap();
        svc.expand_all_wildcard_routes(&all_nodes, &all_links).await;

        let count_after = db.get_routes_for_node("spoke-a").await.unwrap().len();
        assert_eq!(
            count_before, count_after,
            "re-expansion should be idempotent"
        );
    }

    #[tokio::test]
    async fn spt_expansion_full_mesh_all_nodes_get_routes() {
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        let node_c = make_node("node-c", Some("group-c"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();
        db.save_node(node_c).await.unwrap();

        // Full mesh topology (wildcard links).
        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Create all inter-group links (full mesh).
        for (src, dst, lid) in [
            ("node-a", "node-b", "link-ab"),
            ("node-a", "node-c", "link-ac"),
            ("node-b", "node-c", "link-bc"),
        ] {
            let link = crate::db::Link {
                link_id: lid.to_string(),
                source_node_id: src.to_string(),
                source_group: String::new(),
                dest_node_id: dst.to_string(),
                dest_group: String::new(),
                dest_endpoint: format!("{dst}:8080"),
                conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
                status: LinkStatus::Applied,
                status_msg: String::new(),
                created_at: SystemTime::now(),
                last_updated: SystemTime::now(),
            };
            db.add_link(link).await.unwrap();
        }

        // Add wildcard route to node-a (root = group-a).
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "svc", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "node-a", &route).await.unwrap();

        // In full mesh SPT rooted at group-a, both group-b and group-c are
        // direct children. So node-b and node-c should each get a route.
        let routes_b = db.get_routes_for_node("node-b").await.unwrap();
        let routes_c = db.get_routes_for_node("node-c").await.unwrap();

        assert!(
            routes_b.iter().any(|r| r.dest_node_id == "node-a"),
            "node-b should have route to node-a"
        );
        assert!(
            routes_c.iter().any(|r| r.dest_node_id == "node-a"),
            "node-c should have route to node-a"
        );
    }

    #[tokio::test]
    async fn spt_expansion_chain_routes_through_intermediate() {
        // Chain: group-a — group-b — group-c
        // Route to node-a (root=group-a). node-c should route via node-b.
        let db = InMemoryDb::shared();
        let node_a = make_node("node-a", Some("group-a"), vec![]);
        let node_b = make_node("node-b", Some("group-b"), vec![]);
        let node_c = make_node("node-c", Some("group-c"), vec![]);
        db.save_node(node_a).await.unwrap();
        db.save_node(node_b).await.unwrap();
        db.save_node(node_c).await.unwrap();

        // Chain topology: a↔b, b↔c (no direct a↔c).
        let topology = TopologyConfig::Links(vec![
            AdjacencyEntry {
                group: "group-a".to_string(),
                neighbors: vec!["group-b".to_string()],
            },
            AdjacencyEntry {
                group: "group-b".to_string(),
                neighbors: vec!["group-a".to_string(), "group-c".to_string()],
            },
            AdjacencyEntry {
                group: "group-c".to_string(),
                neighbors: vec!["group-b".to_string()],
            },
        ]);
        let svc = make_route_service_with_topology(db.clone(), topology);
        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Links: a↔b and b↔c.
        let link_ab = crate::db::Link {
            link_id: "link-ab".to_string(),
            source_node_id: "node-a".to_string(),
            source_group: String::new(),
            dest_node_id: "node-b".to_string(),
            dest_group: String::new(),
            dest_endpoint: "node-b:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        let link_bc = crate::db::Link {
            link_id: "link-bc".to_string(),
            source_node_id: "node-b".to_string(),
            source_group: String::new(),
            dest_node_id: "node-c".to_string(),
            dest_group: String::new(),
            dest_endpoint: "node-c:8080".to_string(),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.add_link(link_ab).await.unwrap();
        db.add_link(link_bc).await.unwrap();

        // Add wildcard route to node-a.
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "svc", "type"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "node-a", &route).await.unwrap();

        // node-b (child of root group-a) should route via link-ab.
        let routes_b = db.get_routes_for_node("node-b").await.unwrap();
        let route_b = routes_b.iter().find(|r| r.dest_node_id == "node-a");
        assert!(route_b.is_some(), "node-b should have route to node-a");
        assert_eq!(route_b.unwrap().link_id.as_deref(), Some("link-ab"));

        // node-c (child of group-b in the chain) should route via link-bc
        // toward its parent (group-b), NOT directly to group-a.
        let routes_c = db.get_routes_for_node("node-c").await.unwrap();
        let route_c = routes_c.iter().find(|r| r.dest_node_id == "node-a");
        assert!(route_c.is_some(), "node-c should have route to node-a");
        assert_eq!(
            route_c.unwrap().link_id.as_deref(),
            Some("link-bc"),
            "node-c should route toward group-b (its parent in the SPT)"
        );
    }

    // ── topology: segmented route isolation ──────────────────────────────

    /// Helper: create an Applied link record.
    fn make_link(id: &str, src: &str, dst: &str) -> crate::db::Link {
        crate::db::Link {
            link_id: id.to_string(),
            source_node_id: src.to_string(),
            source_group: String::new(),
            dest_node_id: dst.to_string(),
            dest_group: String::new(),
            dest_endpoint: format!("{dst}:8080"),
            conn_config_data: ClientConfig::default().with_connection_type(ConnType::Remote),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn segments_isolate_spokes_but_hub_reaches_all() {
        // Two segments: platform↔customer-a and platform↔customer-b.
        // - Route on spoke-a → hub gets it, spoke-b does NOT (isolation).
        // - Route on hub → both spokes get it (hub bridges segments).
        let db = InMemoryDb::shared();
        db.save_node(make_node("hub", Some("platform"), vec![]))
            .await
            .unwrap();
        db.save_node(make_node("spoke-a", Some("customer-a"), vec![]))
            .await
            .unwrap();
        db.save_node(make_node("spoke-b", Some("customer-b"), vec![]))
            .await
            .unwrap();

        let topology = TopologyConfig::Segments(vec![
            SegmentConfig {
                name: "seg-a".to_string(),
                links: vec![AdjacencyEntry {
                    group: "platform".to_string(),
                    neighbors: vec!["customer-a".to_string()],
                }],
            },
            SegmentConfig {
                name: "seg-b".to_string(),
                links: vec![AdjacencyEntry {
                    group: "platform".to_string(),
                    neighbors: vec!["customer-b".to_string()],
                }],
            },
        ]);
        let svc = make_route_service_with_topology(db.clone(), topology);

        db.add_link(make_link("link-hub-a", "hub", "spoke-a"))
            .await
            .unwrap();
        db.add_link(make_link("link-hub-b", "hub", "spoke-b"))
            .await
            .unwrap();

        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Route 1: all → spoke-a (spoke-a's service)
        let route_a = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "ns", "svc-a"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "spoke-a", &route_a)
            .await
            .unwrap();

        // Hub should reach spoke-a (same segment seg-a)
        let hub_routes = db.get_routes_for_node("hub").await.unwrap();
        assert!(
            hub_routes.iter().any(|r| r.dest_node_id == "spoke-a"),
            "hub should have a route to spoke-a"
        );

        // Spoke-b should NOT reach spoke-a (different segments, no overlap)
        let spoke_b_routes = db.get_routes_for_node("spoke-b").await.unwrap();
        assert!(
            !spoke_b_routes.iter().any(|r| r.dest_node_id == "spoke-a"),
            "spoke-b should NOT have a route to spoke-a (isolated segments)"
        );

        // Route 2: all → hub (hub's service)
        let route_hub = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "ns", "svc-hub"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "hub", &route_hub)
            .await
            .unwrap();

        // Both spokes should reach hub (hub is in both segments)
        let spoke_a_routes = db.get_routes_for_node("spoke-a").await.unwrap();
        assert!(
            spoke_a_routes.iter().any(|r| r.dest_node_id == "hub"),
            "spoke-a should have a route to hub"
        );

        let spoke_b_routes = db.get_routes_for_node("spoke-b").await.unwrap();
        assert!(
            spoke_b_routes.iter().any(|r| r.dest_node_id == "hub"),
            "spoke-b should have a route to hub"
        );
    }

    #[tokio::test]
    async fn segments_group_template_expands_and_isolates() {
        // Same isolation test but using $group template config.
        let db = InMemoryDb::shared();
        db.save_node(make_node("hub", Some("platform"), vec![]))
            .await
            .unwrap();
        db.save_node(make_node("spoke-a", Some("customer-a"), vec![]))
            .await
            .unwrap();
        db.save_node(make_node("spoke-b", Some("customer-b"), vec![]))
            .await
            .unwrap();

        let topology = TopologyConfig::Segments(vec![SegmentConfig {
            name: "seg-$group".to_string(),
            links: vec![AdjacencyEntry {
                group: "platform".to_string(),
                neighbors: vec!["$group".to_string()],
            }],
        }]);
        let svc = make_route_service_with_topology(db.clone(), topology);

        db.add_link(make_link("link-hub-a", "hub", "spoke-a"))
            .await
            .unwrap();
        db.add_link(make_link("link-hub-b", "hub", "spoke-b"))
            .await
            .unwrap();

        let all_nodes = db.list_nodes().await.unwrap();
        svc.rebuild_link_graph(&all_nodes).await;

        // Route: all → spoke-a
        let route = ProtoRoute {
            name: Some(ProtoName::from_strings(["org", "ns", "svc-a"])),
            ..Default::default()
        };
        svc.add_route(ALL_NODES_ID, "spoke-a", &route)
            .await
            .unwrap();

        // Hub should have a route to spoke-a
        let hub_routes = db.get_routes_for_node("hub").await.unwrap();
        assert!(
            hub_routes.iter().any(|r| r.dest_node_id == "spoke-a"),
            "hub should have a route to spoke-a ($group template)"
        );

        // Spoke-b should NOT have a route to spoke-a
        let spoke_b_routes = db.get_routes_for_node("spoke-b").await.unwrap();
        assert!(
            !spoke_b_routes.iter().any(|r| r.dest_node_id == "spoke-a"),
            "spoke-b should NOT have a route to spoke-a ($group isolation)"
        );
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::time::SystemTime;

use rand::seq::SliceRandom;
use uuid::Uuid;

use crate::db::LinkStatus;
use crate::error::{Error, Result};

use super::connection_config::{
    ReportedConnection, compute_client_config, find_reported_connection_for_dest,
};
impl super::RouteService {
    /// Ensure inter-group links exist between `node_id` and nodes in other
    /// groups allowed by the topology link policy. Same-group connectivity
    /// is handled by the data plane.
    /// Returns (affected_node_ids, newly_created_links).
    pub(super) async fn ensure_links_for_node(
        &self,
        node_id: &str,
        existing_links: &[crate::db::Link],
        all_nodes: &[crate::db::Node],
        all_links: &[crate::db::Link],
        reported: &[ReportedConnection],
    ) -> (Vec<String>, Vec<crate::db::Link>) {
        let src_node = match all_nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n.clone(),
            None => {
                tracing::error!("ensure_links: node {node_id} not found");
                return (vec![node_id.to_string()], vec![]);
            }
        };
        let mut affected: HashSet<String> = [node_id.to_string()].into_iter().collect();
        let mut new_links: Vec<crate::db::Link> = Vec::new();

        let connected_peers: HashSet<String> = existing_links
            .iter()
            .filter(|l| l.status != LinkStatus::Deleted)
            .map(|l| {
                if l.source_node_id == node_id {
                    l.dest_node_id.clone()
                } else {
                    l.source_node_id.clone()
                }
            })
            .collect();

        // Track which destination groups already have an active inter-group link
        // from or to the source group (across ALL nodes in the group, not just this one).
        let src_group = src_node.group_name.as_deref().unwrap_or("");
        let mut linked_groups: HashSet<String> = all_links
            .iter()
            .filter(|l| l.status != LinkStatus::Deleted)
            .filter_map(|l| {
                let dg = l.dest_group.as_str();
                let src_grp = l.source_group.as_str();
                // Outgoing link from any node in src_group to another group.
                if src_grp == src_group && !dg.is_empty() && dg != src_group {
                    Some(dg.to_string())
                } else if dg == src_group && !src_grp.is_empty() && src_grp != src_group {
                    // Incoming link targeting src_group from another group.
                    Some(src_grp.to_string())
                } else {
                    None
                }
            })
            .collect();

        let has_src_external = src_node.conn_details.iter().any(|d| {
            d.external_endpoint
                .as_deref()
                .map(|e| !e.is_empty())
                .unwrap_or(false)
        });

        // Shuffle nodes to randomly distribute the gateway role across group members.
        let mut candidates: Vec<_> = all_nodes.to_vec();
        candidates.shuffle(&mut rand::rng());

        for other in &candidates {
            if other.id == node_id {
                continue;
            }
            if connected_peers.contains(&other.id) {
                continue;
            }
            // Same-group links are handled by the data plane automatically.
            let dst_group = other.group_name.as_deref().unwrap_or("");
            if src_group == dst_group {
                continue;
            }
            // Only one link per group pair.
            if linked_groups.contains(dst_group) {
                continue;
            }
            // Topology policy: skip link creation if the groups are not allowed to link.
            if !self.can_link_runtime(src_group, dst_group).await {
                tracing::debug!(
                    "topology policy: skipping link between {node_id} ({src_group}) and {} ({dst_group})",
                    other.id
                );
                continue;
            }
            let has_dst_external = other.conn_details.iter().any(|d| {
                d.external_endpoint
                    .as_deref()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
            });
            if has_dst_external {
                if let Some((src, link)) = self
                    .ensure_link_internal(&src_node, other, reported, true)
                    .await
                {
                    affected.insert(src);
                    new_links.extend(link);
                    linked_groups.insert(dst_group.to_string());
                }
                continue;
            }
            if has_src_external {
                if let Some((src, link)) = self
                    .ensure_link_internal(other, &src_node, reported, true)
                    .await
                {
                    affected.insert(src);
                    new_links.extend(link);
                    linked_groups.insert(dst_group.to_string());
                }
                continue;
            }
            tracing::error!(
                "cannot create link between {node_id} and {}: no external endpoint available",
                other.id
            );
        }
        (affected.into_iter().collect(), new_links)
    }

    /// Ensure an inter-group link exists between src_node and a node in the
    /// destination group. The `dest_node_id` is left empty because the actual
    /// destination node is unknown until the remote side claims the link.
    /// `dst_node` is used only to derive the dest_group and endpoint.
    async fn ensure_link_internal(
        &self,
        src_node: &crate::db::Node,
        dst_node: &crate::db::Node,
        reported: &[ReportedConnection],
        reuse_existing_link_id: bool,
    ) -> Option<(String, Option<crate::db::Link>)> {
        let dest_group = dst_node.group_name.clone().unwrap_or_default();

        let (endpoint, config_data, link_id, status) =
            if let Some(rc) = find_reported_connection_for_dest(reported, dst_node) {
                (
                    rc.endpoint.clone(),
                    rc.config_data.clone(),
                    rc.link_id.clone(),
                    // Source already confirmed the connection; mark Connecting
                    // until the destination claims the link.
                    LinkStatus::Connecting,
                )
            } else {
                let (ep, cd) = match compute_client_config(src_node, dst_node) {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::error!(
                            "ensure_link: failed to get connection details {}->{}:  {e}",
                            src_node.id,
                            dst_node.id
                        );
                        return None;
                    }
                };
                let lid = if reuse_existing_link_id {
                    // Reuse an existing link_id only when its destination matches dst_node.
                    self.0
                        .db
                        .get_link_for_source_and_endpoint(&src_node.id, &ep)
                        .await
                        .ok()
                        .flatten()
                        .filter(|l| l.dest_group == dest_group)
                        .map(|l| l.link_id)
                        .unwrap_or_else(|| Uuid::new_v4().to_string())
                } else {
                    Uuid::new_v4().to_string()
                };
                (ep, cd, lid, LinkStatus::Pending)
            };

        let link = crate::db::Link {
            link_id,
            source_node_id: src_node.id.clone(),
            source_group: src_node.group_name.clone().unwrap_or_default(),
            // dest_node_id is left empty — resolved when the remote node claims the link.
            dest_node_id: String::new(),
            dest_group,
            dest_endpoint: endpoint,
            conn_config_data: config_data,
            status,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };

        match self.0.db.find_or_create_link(link).await {
            Ok((link, created)) => {
                if created {
                    Some((src_node.id.clone(), Some(link)))
                } else {
                    Some((src_node.id.clone(), None))
                }
            }
            Err(e) => {
                tracing::error!(
                    "ensure_link: failed to ensure link {}->dest_group({}):  {e}",
                    src_node.id,
                    dst_node.group_name.as_deref().unwrap_or("")
                );
                None
            }
        }
    }

    pub(super) async fn find_matching_link(
        &self,
        source: &str,
        source_group: &str,
        dest: &str,
        dest_group: &str,
    ) -> Result<String> {
        // First try exact node-to-node match (legacy / direct links).
        if let Some(l) = self.0.db.find_link_between_nodes(source, dest).await?
            && l.status != LinkStatus::Deleted
        {
            return Ok(l.link_id);
        }

        // For inter-group links: find a link between the groups of source and dest.
        if source_group != dest_group
            && let Some(link) = self
                .0
                .db
                .find_link_between_groups(source_group, dest_group)
                .await?
        {
            return Ok(link.link_id);
        }

        Err(Error::InvalidInput(format!(
            "no matching link found for source={source} destination={dest}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_utils::{
        make_conn_details, make_node, make_route_service, make_route_service_with_topology,
        star_topology,
    };
    use crate::db::inmemory::InMemoryDb;

    #[tokio::test]
    async fn ensure_links_star_skips_spoke_to_spoke() {
        let db = InMemoryDb::shared();
        let hub = make_node(
            "hub-node",
            Some("platform"),
            vec![make_conn_details("hub:8080", Some("hub-ext:9090"))],
        );
        let spoke_a = make_node(
            "spoke-a",
            Some("customer-a"),
            vec![make_conn_details("a:8080", Some("a-ext:9090"))],
        );
        let spoke_b = make_node(
            "spoke-b",
            Some("customer-b"),
            vec![make_conn_details("b:8080", Some("b-ext:9090"))],
        );
        db.save_node(hub.clone()).await.unwrap();
        db.save_node(spoke_a.clone()).await.unwrap();
        db.save_node(spoke_b.clone()).await.unwrap();

        let svc = make_route_service_with_topology(db.clone(), star_topology());
        let all_nodes = db.list_nodes().await.unwrap();
        let (affected, new_links) = svc
            .ensure_links_for_node("spoke-a", &[], &all_nodes, &[], &[])
            .await;

        // spoke-a should link to hub's group (platform) but NOT to spoke-b's group
        assert!(affected.contains(&"spoke-a".to_string()));
        assert!(
            new_links
                .iter()
                .any(|l| l.dest_group == "platform" || l.source_node_id == "hub-node")
        );
        assert!(
            !new_links
                .iter()
                .any(|l| l.dest_group == "customer-b" || l.source_node_id == "spoke-b")
        );
    }

    #[tokio::test]
    async fn ensure_links_full_mesh_connects_all() {
        let db = InMemoryDb::shared();
        let node_a = make_node(
            "node-a",
            Some("group-a"),
            vec![make_conn_details("a:8080", Some("a-ext:9090"))],
        );
        let node_b = make_node(
            "node-b",
            Some("group-b"),
            vec![make_conn_details("b:8080", Some("b-ext:9090"))],
        );
        let node_c = make_node(
            "node-c",
            Some("group-c"),
            vec![make_conn_details("c:8080", Some("c-ext:9090"))],
        );
        db.save_node(node_a.clone()).await.unwrap();
        db.save_node(node_b.clone()).await.unwrap();
        db.save_node(node_c.clone()).await.unwrap();

        // Default topology = full mesh
        let svc = make_route_service(db.clone());
        let all_nodes = db.list_nodes().await.unwrap();
        let (_, new_links) = svc
            .ensure_links_for_node("node-a", &[], &all_nodes, &[], &[])
            .await;

        // node-a should link to both node-b and node-c
        assert_eq!(new_links.len(), 2);
    }

    #[tokio::test]
    async fn ensure_links_segments_isolates_spokes() {
        use crate::config::{AdjacencyEntry, SegmentConfig, TopologyConfig};

        let db = InMemoryDb::shared();
        let hub = make_node(
            "hub-node",
            Some("platform"),
            vec![make_conn_details("hub:8080", Some("hub-ext:9090"))],
        );
        let spoke_a = make_node(
            "spoke-a",
            Some("customer-a"),
            vec![make_conn_details("a:8080", Some("a-ext:9090"))],
        );
        let spoke_b = make_node(
            "spoke-b",
            Some("customer-b"),
            vec![make_conn_details("b:8080", Some("b-ext:9090"))],
        );
        db.save_node(hub.clone()).await.unwrap();
        db.save_node(spoke_a.clone()).await.unwrap();
        db.save_node(spoke_b.clone()).await.unwrap();

        // Two segments: platform↔customer-a and platform↔customer-b
        let topology = TopologyConfig::Segments(vec![
            SegmentConfig {
                name: "seg-a".to_string(),
                links: vec![AdjacencyEntry {
                    name: "platform".to_string(),
                    neighbors: vec!["customer-a".to_string()],
                }],
            },
            SegmentConfig {
                name: "seg-b".to_string(),
                links: vec![AdjacencyEntry {
                    name: "platform".to_string(),
                    neighbors: vec!["customer-b".to_string()],
                }],
            },
        ]);
        let svc = make_route_service_with_topology(db.clone(), topology);
        let all_nodes = db.list_nodes().await.unwrap();

        // Hub should link to both spokes (union of segments)
        let (_, hub_links) = svc
            .ensure_links_for_node("hub-node", &[], &all_nodes, &[], &[])
            .await;
        assert_eq!(
            hub_links.len(),
            2,
            "hub should create 2 links (one per spoke)"
        );

        // Spoke-a should link to hub only, NOT to spoke-b
        let (_, spoke_a_links) = svc
            .ensure_links_for_node("spoke-a", &[], &all_nodes, &[], &[])
            .await;
        assert_eq!(
            spoke_a_links.len(),
            1,
            "spoke-a should create 1 link (to hub)"
        );
        assert!(
            spoke_a_links
                .iter()
                .all(|l| l.dest_group == "platform" || l.source_node_id == "hub-node"),
            "spoke-a link should be to platform group"
        );
        assert!(
            !spoke_a_links
                .iter()
                .any(|l| l.dest_group == "customer-b" || l.source_node_id == "spoke-b"),
            "spoke-a should NOT link to customer-b"
        );
    }
}

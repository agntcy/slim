// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::time::SystemTime;

use uuid::Uuid;

use crate::db::LinkStatus;
use crate::error::{Error, Result};

use super::connections::{
    ReportedConnection, compute_connection_details, find_reported_connection_for_dest,
};
impl super::RouteService {
    /// Ensure direct or group links exist between `node_id` and every other
    /// node allowed by the topology link policy.
    /// Returns (affected_node_ids, newly_created_links).
    pub(super) async fn ensure_links_for_node(
        &self,
        node_id: &str,
        existing_links: &[crate::db::Link],
        all_nodes: &[crate::db::Node],
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

        let has_src_external = src_node.conn_details.iter().any(|d| {
            d.external_endpoint
                .as_deref()
                .map(|e| !e.is_empty())
                .unwrap_or(false)
        });

        for other in all_nodes {
            if other.id == node_id {
                continue;
            }
            if connected_peers.contains(&other.id) {
                continue;
            }
            // Topology policy: skip link creation if the groups are not allowed to link.
            let src_group = src_node.group_name.as_deref().unwrap_or("");
            let dst_group = other.group_name.as_deref().unwrap_or("");
            if !self.0.topology.can_link(src_group, dst_group) {
                tracing::debug!(
                    "topology policy: skipping link between {node_id} ({src_group}) and {} ({dst_group})",
                    other.id
                );
                continue;
            }
            let same_group = src_node.group_name == other.group_name;
            if same_group {
                if let Some((src, link)) = self
                    .ensure_link_internal(&src_node, other, reported, false)
                    .await
                {
                    affected.insert(src);
                    new_links.extend(link);
                }
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

    /// Ensure a link exists between src_node and dst_node.
    /// When `reuse_existing_link_id` is true (cross-group links), tries to reuse
    /// an existing link_id for the same source+endpoint before generating a new one.
    async fn ensure_link_internal(
        &self,
        src_node: &crate::db::Node,
        dst_node: &crate::db::Node,
        reported: &[ReportedConnection],
        reuse_existing_link_id: bool,
    ) -> Option<(String, Option<crate::db::Link>)> {
        let (endpoint, config_data, link_id, status) =
            if let Some(rc) = find_reported_connection_for_dest(reported, dst_node) {
                (
                    rc.endpoint.clone(),
                    rc.config_data.clone(),
                    rc.link_id.clone(),
                    LinkStatus::Applied,
                )
            } else {
                let (ep, cd) = match compute_connection_details(src_node, dst_node) {
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
                        .filter(|l| l.dest_node_id == dst_node.id)
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
            dest_node_id: dst_node.id.clone(),
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
                    "ensure_link: failed to ensure link {}->{}:  {e}",
                    src_node.id,
                    dst_node.id
                );
                None
            }
        }
    }

    pub(super) async fn find_matching_link(&self, source: &str, dest: &str) -> Result<String> {
        match self.0.db.find_link_between_nodes(source, dest).await? {
            Some(l) if l.status != LinkStatus::Deleted => Ok(l.link_id),
            _ => Err(Error::InvalidInput(format!(
                "no matching link found for source={source} destination={dest}"
            ))),
        }
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
            .ensure_links_for_node("spoke-a", &[], &all_nodes, &[])
            .await;

        // spoke-a should link to hub but NOT to spoke-b
        assert!(affected.contains(&"spoke-a".to_string()));
        assert!(
            new_links
                .iter()
                .any(|l| l.dest_node_id == "hub-node" || l.source_node_id == "hub-node")
        );
        assert!(
            !new_links
                .iter()
                .any(|l| l.dest_node_id == "spoke-b" || l.source_node_id == "spoke-b")
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
            .ensure_links_for_node("node-a", &[], &all_nodes, &[])
            .await;

        // node-a should link to both node-b and node-c
        assert_eq!(new_links.len(), 2);
    }
}

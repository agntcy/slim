// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::SystemTime;

use parking_lot::RwLock;
use uuid::Uuid;

use async_trait::async_trait;

use super::model::{
    ALL_NODES_ID, Channel, Link, Node, Route, RouteStatus, SubscriptionName,
    has_connection_details_changed,
};
use super::{DataAccess, SharedDb};

pub struct InMemoryDb {
    nodes: RwLock<HashMap<String, Node>>,
    routes: RwLock<HashMap<i64, Route>>,
    links: RwLock<HashMap<String, Link>>,
    channels: RwLock<HashMap<String, Channel>>,
}

impl InMemoryDb {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            routes: RwLock::new(HashMap::new()),
            links: RwLock::new(HashMap::new()),
            channels: RwLock::new(HashMap::new()),
        }
    }

    pub fn shared() -> SharedDb {
        std::sync::Arc::new(Self::new())
    }
}

impl Default for InMemoryDb {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataAccess for InMemoryDb {
    // ── Nodes ──────────────────────────────────────────────────────────────

    async fn list_nodes(&self) -> Vec<Node> {
        self.nodes.read().values().cloned().collect()
    }

    async fn get_node(&self, id: &str) -> Option<Node> {
        self.nodes.read().get(id).cloned()
    }

    async fn save_node(&self, mut node: Node) -> Result<(String, bool), String> {
        let mut nodes = self.nodes.write();
        let conn_details_changed;
        if node.id.is_empty() {
            node.id = Uuid::new_v4().to_string();
            conn_details_changed = false;
        } else if let Some(existing) = nodes.get(&node.id) {
            conn_details_changed =
                has_connection_details_changed(&existing.conn_details, &node.conn_details);
        } else {
            conn_details_changed = false;
        }
        node.last_updated = SystemTime::now();
        let id = node.id.clone();
        nodes.insert(id.clone(), node);
        Ok((id, conn_details_changed))
    }

    async fn delete_node(&self, id: &str) -> Result<(), String> {
        let mut nodes = self.nodes.write();
        if nodes.remove(id).is_none() {
            return Err(format!("node {id} not found"));
        }
        Ok(())
    }

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, mut route: Route) -> Result<Route, String> {
        let route_id = route.compute_id();
        let mut routes = self.routes.write();
        if routes.contains_key(&route_id) {
            return Err(format!("route {} already exists", route));
        }
        route.id = route_id;
        route.last_updated = SystemTime::now();
        routes.insert(route_id, route.clone());
        Ok(route)
    }

    async fn get_route_by_id(&self, route_id: i64) -> Option<Route> {
        self.routes.read().get(&route_id).cloned()
    }

    async fn get_routes_for_node_id(&self, node_id: &str) -> Vec<Route> {
        self.routes
            .read()
            .values()
            .filter(|r| r.source_node_id == node_id)
            .cloned()
            .collect()
    }

    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Vec<Route> {
        self.routes
            .read()
            .values()
            .filter(|r| r.dest_node_id == node_id && r.source_node_id != ALL_NODES_ID)
            .cloned()
            .collect()
    }

    async fn get_routes_for_dest_node_id_and_name(
        &self,
        node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Vec<Route> {
        self.routes
            .read()
            .values()
            .filter(|r| {
                r.dest_node_id == node_id
                    && r.component0 == component0
                    && r.component1 == component1
                    && r.component2 == component2
                    && r.component_id == component_id
            })
            .cloned()
            .collect()
    }

    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &SubscriptionName<'_>,
        dest_node_id: &str,
        link_id: &str,
    ) -> Option<Route> {
        self.routes
            .read()
            .values()
            .find(|r| {
                r.source_node_id == src_node_id
                    && (dest_node_id.is_empty() || r.dest_node_id == dest_node_id)
                    && (link_id.is_empty() || r.link_id == link_id)
                    && r.component0 == name.component0
                    && r.component1 == name.component1
                    && r.component2 == name.component2
                    && r.component_id == name.component_id
            })
            .cloned()
    }

    async fn filter_routes_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Vec<Route> {
        self.routes
            .read()
            .values()
            .filter(|r| {
                (source_node_id.is_empty() || r.source_node_id == source_node_id)
                    && (dest_node_id.is_empty() || r.dest_node_id == dest_node_id)
            })
            .cloned()
            .collect()
    }

    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Option<String> {
        let routes = self.routes.read();
        let mut matching: Vec<&Route> = routes
            .values()
            .filter(|r| {
                r.source_node_id == ALL_NODES_ID
                    && r.component0 == component0
                    && r.component1 == component1
                    && r.component2 == component2
                    && r.component_id == component_id
            })
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        Some(matching[0].dest_node_id.clone())
    }

    async fn get_routes_by_link_id(&self, link_id: &str) -> Vec<Route> {
        self.routes
            .read()
            .values()
            .filter(|r| r.link_id == link_id)
            .cloned()
            .collect()
    }

    async fn delete_route(&self, route_id: i64) -> Result<(), String> {
        let mut routes = self.routes.write();
        if routes.remove(&route_id).is_none() {
            return Err(format!("route {route_id} not found"));
        }
        Ok(())
    }

    async fn mark_route_deleted(&self, route_id: i64) -> Result<(), String> {
        let mut routes = self.routes.write();
        let route = routes
            .get_mut(&route_id)
            .ok_or_else(|| format!("route {route_id} not found"))?;
        route.deleted = true;
        route.last_updated = SystemTime::now();
        Ok(())
    }

    async fn mark_route_applied(&self, route_id: i64) -> Result<(), String> {
        let mut routes = self.routes.write();
        let route = routes
            .get_mut(&route_id)
            .ok_or_else(|| format!("route {route_id} not found"))?;
        route.status = RouteStatus::Applied;
        route.status_msg.clear();
        route.last_updated = SystemTime::now();
        Ok(())
    }

    async fn mark_route_failed(&self, route_id: i64, msg: &str) -> Result<(), String> {
        let mut routes = self.routes.write();
        let route = routes
            .get_mut(&route_id)
            .ok_or_else(|| format!("route {route_id} not found"))?;
        route.status = RouteStatus::Failed;
        route.status_msg = msg.to_string();
        route.last_updated = SystemTime::now();
        Ok(())
    }

    async fn repoint_route(
        &self,
        route_id: i64,
        link_id: &str,
        status: RouteStatus,
        msg: &str,
    ) -> Result<(), String> {
        let mut routes = self.routes.write();
        let route = routes
            .get_mut(&route_id)
            .ok_or_else(|| format!("route {route_id} not found"))?;
        route.link_id = link_id.to_string();
        route.status = status;
        route.status_msg = msg.to_string();
        route.last_updated = SystemTime::now();
        Ok(())
    }

    // ── Links ──────────────────────────────────────────────────────────────

    async fn add_link(&self, mut link: Link) -> Result<Link, String> {
        if link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err("sourceNodeID, destNodeID and destEndpoint are required".to_string());
        }
        let mut links = self.links.write();

        // Reuse an existing non-deleted link with the same source + endpoint.
        if link.link_id.is_empty() {
            for existing in links.values() {
                if !existing.deleted
                    && existing.source_node_id == link.source_node_id
                    && existing.dest_endpoint == link.dest_endpoint
                {
                    link.link_id = existing.link_id.clone();
                    break;
                }
            }
            if link.link_id.is_empty() {
                link.link_id = Uuid::new_v4().to_string();
            }
        }
        link.last_updated = SystemTime::now();
        let key = link.storage_key();
        links.insert(key, link.clone());
        Ok(link)
    }

    async fn update_link(&self, mut link: Link) -> Result<(), String> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err("link identity fields cannot be empty".to_string());
        }
        let mut links = self.links.write();
        let key = link.storage_key();
        if !links.contains_key(&key) {
            return Err("link not found".to_string());
        }
        link.last_updated = SystemTime::now();
        links.insert(key, link);
        Ok(())
    }

    async fn delete_link(&self, link: &Link) -> Result<(), String> {
        let mut links = self.links.write();
        let key = link.storage_key();
        if links.remove(&key).is_none() {
            return Err("link not found".to_string());
        }
        Ok(())
    }

    async fn get_link(
        &self,
        link_id: &str,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link> {
        let links = self.links.read();
        let mut latest: Option<&Link> = None;
        for link in links.values() {
            if link.deleted || link.link_id != link_id {
                continue;
            }
            if !source_node_id.is_empty() && !dest_node_id.is_empty() {
                let direct =
                    link.source_node_id == source_node_id && link.dest_node_id == dest_node_id;
                let reverse =
                    link.source_node_id == dest_node_id && link.dest_node_id == source_node_id;
                if !direct && !reverse {
                    continue;
                }
            }
            match latest {
                None => latest = Some(link),
                Some(prev) => {
                    if link.last_updated > prev.last_updated {
                        latest = Some(link);
                    }
                }
            }
        }
        latest.cloned()
    }

    async fn find_link_between_nodes(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link> {
        let links = self.links.read();
        let mut latest: Option<&Link> = None;
        for link in links.values() {
            if link.deleted {
                continue;
            }
            let direct = link.source_node_id == source_node_id && link.dest_node_id == dest_node_id;
            let reverse =
                link.source_node_id == dest_node_id && link.dest_node_id == source_node_id;
            if !direct && !reverse {
                continue;
            }
            match latest {
                None => latest = Some(link),
                Some(prev) => {
                    if link.last_updated > prev.last_updated {
                        latest = Some(link);
                    }
                }
            }
        }
        latest.cloned()
    }

    async fn get_link_for_source_and_endpoint(
        &self,
        source_node_id: &str,
        dest_endpoint: &str,
    ) -> Option<Link> {
        let links = self.links.read();
        let mut latest: Option<&Link> = None;
        for link in links.values() {
            if link.deleted
                || link.source_node_id != source_node_id
                || link.dest_endpoint != dest_endpoint
            {
                continue;
            }
            match latest {
                None => latest = Some(link),
                Some(prev) => {
                    if link.last_updated > prev.last_updated {
                        latest = Some(link);
                    }
                }
            }
        }
        latest.cloned()
    }

    async fn get_links_for_node(&self, node_id: &str) -> Vec<Link> {
        self.links
            .read()
            .values()
            .filter(|l| l.source_node_id == node_id || l.dest_node_id == node_id)
            .cloned()
            .collect()
    }

    // ── Channels ───────────────────────────────────────────────────────────

    async fn save_channel(&self, channel_id: &str, moderators: Vec<String>) -> Result<(), String> {
        let mut channels = self.channels.write();
        if channels.contains_key(channel_id) {
            return Err(format!("channel {channel_id} already exists"));
        }
        channels.insert(
            channel_id.to_string(),
            Channel {
                id: channel_id.to_string(),
                moderators,
                participants: vec![],
            },
        );
        Ok(())
    }

    async fn delete_channel(&self, channel_id: &str) -> Result<(), String> {
        let mut channels = self.channels.write();
        if channels.remove(channel_id).is_none() {
            return Err(format!("channel {channel_id} not found"));
        }
        Ok(())
    }

    async fn get_channel(&self, channel_id: &str) -> Option<Channel> {
        self.channels.read().get(channel_id).cloned()
    }

    async fn update_channel(&self, channel: Channel) -> Result<(), String> {
        if channel.id.is_empty() {
            return Err("channel ID cannot be empty".to_string());
        }
        let mut channels = self.channels.write();
        if !channels.contains_key(&channel.id) {
            return Err(format!("channel {} not found", channel.id));
        }
        channels.insert(channel.id.clone(), channel);
        Ok(())
    }

    async fn list_channels(&self) -> Vec<Channel> {
        self.channels.read().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ConnectionDetails, LinkStatus, RouteStatus, SubscriptionName};
    use std::time::SystemTime;

    fn db() -> InMemoryDb {
        InMemoryDb::new()
    }

    fn make_node(id: &str, group: Option<&str>) -> Node {
        Node {
            id: id.to_string(),
            group_name: group.map(|s| s.to_string()),
            conn_details: vec![ConnectionDetails {
                endpoint: format!("{id}:8080"),
                external_endpoint: None,
                trust_domain: None,
                mtls_required: false,
                client_config: serde_json::Value::Null,
            }],
            last_updated: SystemTime::now(),
        }
    }

    fn make_route(src: &str, dst: &str, link: &str) -> Route {
        Route {
            id: 0,
            source_node_id: src.to_string(),
            dest_node_id: dst.to_string(),
            link_id: link.to_string(),
            component0: "org".to_string(),
            component1: "ns".to_string(),
            component2: "svc".to_string(),
            component_id: Some(1),
            status: RouteStatus::Pending,
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        }
    }

    fn make_link(src: &str, dst: &str, ep: &str, lid: &str) -> Link {
        Link {
            link_id: lid.to_string(),
            source_node_id: src.to_string(),
            dest_node_id: dst.to_string(),
            dest_endpoint: ep.to_string(),
            conn_config_data: String::new(),
            status: LinkStatus::Pending,
            status_msg: String::new(),
            deleted: false,
            last_updated: SystemTime::now(),
        }
    }

    // ── Node CRUD ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_get_node() {
        let db = db();
        let node = make_node("n1", Some("grp"));
        let (id, changed) = db.save_node(node).await.unwrap();
        assert_eq!(id, "n1");
        assert!(!changed);
        let got = db.get_node("n1").await.unwrap();
        assert_eq!(got.id, "n1");
        assert_eq!(got.group_name.as_deref(), Some("grp"));
    }

    #[tokio::test]
    async fn save_node_without_id_generates_uuid() {
        let db = db();
        let node = Node {
            id: String::new(),
            group_name: None,
            conn_details: vec![],
            last_updated: SystemTime::now(),
        };
        let (id, _) = db.save_node(node).await.unwrap();
        assert!(!id.is_empty());
    }

    #[tokio::test]
    async fn save_node_conn_details_changed() {
        let db = db();
        let node = make_node("n1", None);
        db.save_node(node).await.unwrap();
        // Update with different connection details.
        let mut updated = make_node("n1", None);
        updated.conn_details[0].endpoint = "n1:9999".to_string();
        let (_, changed) = db.save_node(updated).await.unwrap();
        assert!(changed);
    }

    #[tokio::test]
    async fn list_nodes_returns_all() {
        let db = db();
        db.save_node(make_node("n1", None)).await.unwrap();
        db.save_node(make_node("n2", None)).await.unwrap();
        let nodes = db.list_nodes().await;
        assert_eq!(nodes.len(), 2);
    }

    #[tokio::test]
    async fn delete_node_removes_it() {
        let db = db();
        db.save_node(make_node("n1", None)).await.unwrap();
        db.delete_node("n1").await.unwrap();
        assert!(db.get_node("n1").await.is_none());
    }

    #[tokio::test]
    async fn delete_node_not_found_returns_error() {
        let db = db();
        assert!(db.delete_node("missing").await.is_err());
    }

    // ── Route CRUD ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_get_route() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert_ne!(r.id, 0);
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.source_node_id, "src");
    }

    #[tokio::test]
    async fn add_duplicate_route_returns_error() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert!(db.add_route(make_route("src", "dst", "lnk")).await.is_err());
    }

    #[tokio::test]
    async fn get_routes_for_node_id() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.add_route(make_route("other", "dst", "lnk2"))
            .await
            .unwrap();
        let routes = db.get_routes_for_node_id("src").await;
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source_node_id, "src");
    }

    #[tokio::test]
    async fn get_routes_for_dest_node_id() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        // Wildcard source should be excluded.
        let mut wildcard = make_route(ALL_NODES_ID, "dst", "lnk2");
        wildcard.component1 = "ns2".to_string();
        db.add_route(wildcard).await.unwrap();
        let routes = db.get_routes_for_dest_node_id("dst").await;
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source_node_id, "src");
    }

    #[tokio::test]
    async fn get_routes_for_dest_node_id_and_name() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let found = db
            .get_routes_for_dest_node_id_and_name("dst", "org", "ns", "svc", Some(1))
            .await;
        assert_eq!(found.len(), 1);
        let not_found = db
            .get_routes_for_dest_node_id_and_name("dst", "org", "ns", "svc", None)
            .await;
        assert!(not_found.is_empty());
    }

    #[tokio::test]
    async fn get_route_for_src_dest_name() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let name = SubscriptionName {
            component0: "org",
            component1: "ns",
            component2: "svc",
            component_id: Some(1),
        };
        let found = db
            .get_route_for_src_dest_name("src", &name, "dst", "lnk")
            .await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, r.id);
    }

    #[tokio::test]
    async fn get_route_for_src_dest_name_empty_filters() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let name = SubscriptionName {
            component0: "org",
            component1: "ns",
            component2: "svc",
            component_id: Some(1),
        };
        // Empty dest_node_id and link_id should match any.
        let found = db.get_route_for_src_dest_name("src", &name, "", "").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, r.id);
    }

    #[tokio::test]
    async fn filter_routes_by_src_dest() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let all = db.filter_routes_by_src_dest("", "").await;
        assert_eq!(all.len(), 1);
        let by_src = db.filter_routes_by_src_dest("src", "").await;
        assert_eq!(by_src.len(), 1);
        let by_dest = db.filter_routes_by_src_dest("", "dst").await;
        assert_eq!(by_dest.len(), 1);
        let mismatch = db.filter_routes_by_src_dest("other", "").await;
        assert!(mismatch.is_empty());
    }

    #[tokio::test]
    async fn get_destination_node_id_for_name() {
        let db = db();
        let r = make_route(ALL_NODES_ID, "dst_node", "lnk");
        db.add_route(r).await.unwrap();
        let result = db
            .get_destination_node_id_for_name("org", "ns", "svc", Some(1))
            .await;
        assert_eq!(result.as_deref(), Some("dst_node"));
    }

    #[tokio::test]
    async fn get_routes_by_link_id() {
        let db = db();
        db.add_route(make_route("src", "dst", "link-abc"))
            .await
            .unwrap();
        let routes = db.get_routes_by_link_id("link-abc").await;
        assert_eq!(routes.len(), 1);
        let none = db.get_routes_by_link_id("link-xyz").await;
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn delete_route() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.delete_route(r.id).await.unwrap();
        assert!(db.get_route_by_id(r.id).await.is_none());
    }

    #[tokio::test]
    async fn delete_route_not_found() {
        let db = db();
        assert!(db.delete_route(9999).await.is_err());
    }

    #[tokio::test]
    async fn mark_route_deleted() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_deleted(r.id).await.unwrap();
        assert!(db.get_route_by_id(r.id).await.unwrap().deleted);
    }

    #[tokio::test]
    async fn mark_route_applied() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_applied(r.id).await.unwrap();
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.status, RouteStatus::Applied);
        assert!(got.status_msg.is_empty());
    }

    #[tokio::test]
    async fn mark_route_failed() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_failed(r.id, "oops").await.unwrap();
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.status, RouteStatus::Failed);
        assert_eq!(got.status_msg, "oops");
    }

    #[tokio::test]
    async fn repoint_route() {
        let db = db();
        let r = db
            .add_route(make_route("src", "dst", "old_lnk"))
            .await
            .unwrap();
        db.repoint_route(r.id, "new_lnk", RouteStatus::Pending, "")
            .await
            .unwrap();
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.link_id, "new_lnk");
        assert_eq!(got.status, RouteStatus::Pending);
    }

    // ── Link CRUD ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_get_link() {
        let db = db();
        let l = db
            .add_link(make_link("src", "dst", "ep:8080", "lid1"))
            .await
            .unwrap();
        assert_eq!(l.link_id, "lid1");
        let found = db.get_link("lid1", "src", "dst").await;
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn add_link_missing_fields_returns_error() {
        let db = db();
        let mut l = make_link("src", "dst", "ep:8080", "lid");
        l.source_node_id = String::new();
        assert!(db.add_link(l).await.is_err());
    }

    #[tokio::test]
    async fn add_link_reuses_existing_link_id_for_same_src_endpoint() {
        let db = db();
        let l1 = db
            .add_link(make_link("src", "dst1", "ep:8080", "lid1"))
            .await
            .unwrap();
        // Add without explicit link_id — should reuse lid1.
        let l2 = make_link("src", "dst2", "ep:8080", "");
        let added = db.add_link(l2).await.unwrap();
        assert_eq!(added.link_id, l1.link_id);
    }

    #[tokio::test]
    async fn update_link() {
        let db = db();
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        let mut updated = make_link("src", "dst", "ep:8080", "lid");
        updated.status = LinkStatus::Applied;
        db.update_link(updated).await.unwrap();
        let got = db.get_link("lid", "src", "dst").await.unwrap();
        assert_eq!(got.status, LinkStatus::Applied);
    }

    #[tokio::test]
    async fn update_link_missing_fields_returns_error() {
        let db = db();
        let l = make_link("src", "dst", "ep:8080", "");
        assert!(db.update_link(l).await.is_err());
    }

    #[tokio::test]
    async fn delete_link() {
        let db = db();
        let l = db
            .add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        db.delete_link(&l).await.unwrap();
        assert!(db.get_link("lid", "src", "dst").await.is_none());
    }

    #[tokio::test]
    async fn delete_link_not_found_returns_error() {
        let db = db();
        let l = make_link("src", "dst", "ep:8080", "lid");
        assert!(db.delete_link(&l).await.is_err());
    }

    #[tokio::test]
    async fn get_link_reverse_lookup() {
        let db = db();
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        // Should find link even when src/dst are swapped.
        let found = db.get_link("lid", "dst", "src").await;
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn get_link_ignores_deleted() {
        let db = db();
        let l = db
            .add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        let mut deleted = l.clone();
        deleted.deleted = true;
        db.update_link(deleted).await.unwrap();
        assert!(db.get_link("lid", "src", "dst").await.is_none());
    }

    #[tokio::test]
    async fn find_link_between_nodes() {
        let db = db();
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(db.find_link_between_nodes("src", "dst").await.is_some());
        assert!(db.find_link_between_nodes("dst", "src").await.is_some());
        assert!(db.find_link_between_nodes("a", "b").await.is_none());
    }

    #[tokio::test]
    async fn get_link_for_source_and_endpoint() {
        let db = db();
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(
            db.get_link_for_source_and_endpoint("src", "ep:8080")
                .await
                .is_some()
        );
        assert!(
            db.get_link_for_source_and_endpoint("src", "other")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn get_links_for_node() {
        let db = db();
        db.add_link(make_link("src", "dst", "ep:8080", "lid1"))
            .await
            .unwrap();
        let l2 = make_link("other", "src", "ep:9090", "lid2");
        db.add_link(l2).await.unwrap();
        let links = db.get_links_for_node("src").await;
        assert_eq!(links.len(), 2);
    }

    // ── Channel CRUD ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_get_channel() {
        let db = db();
        db.save_channel("chan1", vec!["mod1".to_string()])
            .await
            .unwrap();
        let ch = db.get_channel("chan1").await.unwrap();
        assert_eq!(ch.id, "chan1");
        assert_eq!(ch.moderators, vec!["mod1"]);
        assert!(ch.participants.is_empty());
    }

    #[tokio::test]
    async fn save_channel_duplicate_returns_error() {
        let db = db();
        db.save_channel("chan1", vec![]).await.unwrap();
        assert!(db.save_channel("chan1", vec![]).await.is_err());
    }

    #[tokio::test]
    async fn delete_channel() {
        let db = db();
        db.save_channel("chan1", vec![]).await.unwrap();
        db.delete_channel("chan1").await.unwrap();
        assert!(db.get_channel("chan1").await.is_none());
    }

    #[tokio::test]
    async fn delete_channel_not_found_returns_error() {
        let db = db();
        assert!(db.delete_channel("missing").await.is_err());
    }

    #[tokio::test]
    async fn update_channel() {
        let db = db();
        db.save_channel("chan1", vec!["mod1".to_string()])
            .await
            .unwrap();
        db.update_channel(Channel {
            id: "chan1".to_string(),
            moderators: vec!["mod1".to_string()],
            participants: vec!["p1".to_string()],
        })
        .await
        .unwrap();
        let ch = db.get_channel("chan1").await.unwrap();
        assert_eq!(ch.participants, vec!["p1"]);
    }

    #[tokio::test]
    async fn update_channel_empty_id_returns_error() {
        let db = db();
        assert!(
            db.update_channel(Channel {
                id: String::new(),
                moderators: vec![],
                participants: vec![],
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn update_channel_not_found_returns_error() {
        let db = db();
        assert!(
            db.update_channel(Channel {
                id: "missing".to_string(),
                moderators: vec![],
                participants: vec![],
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn list_channels() {
        let db = db();
        db.save_channel("c1", vec![]).await.unwrap();
        db.save_channel("c2", vec![]).await.unwrap();
        assert_eq!(db.list_channels().await.len(), 2);
    }
}

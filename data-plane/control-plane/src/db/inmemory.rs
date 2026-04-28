// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use parking_lot::RwLock;
use uuid::Uuid;

use async_trait::async_trait;

use super::model::{
    ALL_NODES_ID, Channel, Link, Node, Route, RouteStatus, SubscriptionName,
    has_connection_details_changed,
};
use super::{DataAccess, SharedDb};
use crate::error::{Error, Result};

// ── Route store ────────────────────────────────────────────────────────────────

struct RouteStore {
    /// Primary map: route id → Route.
    primary: HashMap<i64, Route>,
    /// Secondary index: source_node_id → set of route IDs.
    by_src: HashMap<String, HashSet<i64>>,
    /// Secondary index: dest_node_id → set of route IDs.
    by_dest: HashMap<String, HashSet<i64>>,
    /// Secondary index: link_id → set of route IDs (only for non-empty link_ids).
    by_link: HashMap<String, HashSet<i64>>,
}

impl RouteStore {
    fn new() -> Self {
        Self {
            primary: HashMap::new(),
            by_src: HashMap::new(),
            by_dest: HashMap::new(),
            by_link: HashMap::new(),
        }
    }

    fn index_add(&mut self, route: &Route) {
        self.by_src
            .entry(route.source_node_id.clone())
            .or_default()
            .insert(route.id);
        self.by_dest
            .entry(route.dest_node_id.clone())
            .or_default()
            .insert(route.id);
        if !route.link_id.is_empty() {
            self.by_link
                .entry(route.link_id.clone())
                .or_default()
                .insert(route.id);
        }
    }

    fn index_remove(&mut self, route: &Route) {
        if let Some(set) = self.by_src.get_mut(&route.source_node_id) {
            set.remove(&route.id);
        }
        if let Some(set) = self.by_dest.get_mut(&route.dest_node_id) {
            set.remove(&route.id);
        }
        if !route.link_id.is_empty()
            && let Some(set) = self.by_link.get_mut(&route.link_id)
        {
            set.remove(&route.id);
        }
    }
}

// ── Link store ─────────────────────────────────────────────────────────────────

struct LinkStore {
    /// Primary map: Link::storage_key() → Link.
    primary: HashMap<String, Link>,
    /// Secondary index: source_node_id → set of storage keys.
    by_src: HashMap<String, HashSet<String>>,
    /// Secondary index: dest_node_id → set of storage keys.
    by_dest: HashMap<String, HashSet<String>>,
    /// Secondary index: link_id → set of storage keys.
    by_link_id: HashMap<String, HashSet<String>>,
}

impl LinkStore {
    fn new() -> Self {
        Self {
            primary: HashMap::new(),
            by_src: HashMap::new(),
            by_dest: HashMap::new(),
            by_link_id: HashMap::new(),
        }
    }

    fn index_add(&mut self, link: &Link) {
        let key = link.storage_key();
        self.by_src
            .entry(link.source_node_id.clone())
            .or_default()
            .insert(key.clone());
        self.by_dest
            .entry(link.dest_node_id.clone())
            .or_default()
            .insert(key.clone());
        self.by_link_id
            .entry(link.link_id.clone())
            .or_default()
            .insert(key);
    }

    fn index_remove(&mut self, link: &Link) {
        let key = link.storage_key();
        if let Some(set) = self.by_src.get_mut(&link.source_node_id) {
            set.remove(&key);
        }
        if let Some(set) = self.by_dest.get_mut(&link.dest_node_id) {
            set.remove(&key);
        }
        if let Some(set) = self.by_link_id.get_mut(&link.link_id) {
            set.remove(&key);
        }
    }
}

// ── InMemoryDb ─────────────────────────────────────────────────────────────────

pub struct InMemoryDb {
    nodes: RwLock<HashMap<String, Node>>,
    routes: RwLock<RouteStore>,
    links: RwLock<LinkStore>,
    channels: RwLock<HashMap<String, Channel>>,
}

impl InMemoryDb {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            routes: RwLock::new(RouteStore::new()),
            links: RwLock::new(LinkStore::new()),
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

    async fn save_node(&self, mut node: Node) -> Result<(String, bool)> {
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

    async fn delete_node(&self, id: &str) -> Result<()> {
        let mut nodes = self.nodes.write();
        if nodes.remove(id).is_none() {
            return Err(Error::NodeNotFound { id: id.to_string() });
        }
        Ok(())
    }

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, mut route: Route) -> Result<Route> {
        let route_id = route.compute_id();
        let mut store = self.routes.write();
        if store.primary.contains_key(&route_id) {
            return Err(Error::RouteAlreadyExists {
                id: route.to_string(),
            });
        }
        route.id = route_id;
        route.last_updated = SystemTime::now();
        store.index_add(&route);
        store.primary.insert(route_id, route.clone());
        Ok(route)
    }

    async fn get_route_by_id(&self, route_id: i64) -> Option<Route> {
        self.routes.read().primary.get(&route_id).cloned()
    }

    async fn get_routes_for_node_id(&self, node_id: &str) -> Vec<Route> {
        let store = self.routes.read();
        store
            .by_src
            .get(node_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| store.primary.get(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Vec<Route> {
        let store = self.routes.read();
        store
            .by_dest
            .get(node_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| store.primary.get(id))
                    .filter(|r| r.source_node_id != ALL_NODES_ID)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn get_routes_for_dest_node_id_and_name(
        &self,
        node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Vec<Route> {
        let store = self.routes.read();
        store
            .by_dest
            .get(node_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| store.primary.get(id))
                    .filter(|r| {
                        r.component0 == component0
                            && r.component1 == component1
                            && r.component2 == component2
                            && r.component_id == component_id
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &SubscriptionName<'_>,
        dest_node_id: &str,
        link_id: &str,
    ) -> Option<Route> {
        let store = self.routes.read();
        store
            .by_src
            .get(src_node_id)?
            .iter()
            .filter_map(|id| store.primary.get(id))
            .find(|r| {
                (dest_node_id.is_empty() || r.dest_node_id == dest_node_id)
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
        let store = self.routes.read();
        match (source_node_id.is_empty(), dest_node_id.is_empty()) {
            (true, true) => store.primary.values().cloned().collect(),
            (false, true) => store
                .by_src
                .get(source_node_id)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| store.primary.get(id))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            (true, false) => store
                .by_dest
                .get(dest_node_id)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| store.primary.get(id))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            (false, false) => {
                let src_ids = store.by_src.get(source_node_id);
                let dest_ids = store.by_dest.get(dest_node_id);
                match (src_ids, dest_ids) {
                    (Some(src_ids), Some(dest_ids)) => {
                        // Iterate the smaller set and filter by the other dimension.
                        if src_ids.len() <= dest_ids.len() {
                            src_ids
                                .iter()
                                .filter_map(|id| store.primary.get(id))
                                .filter(|r| r.dest_node_id == dest_node_id)
                                .cloned()
                                .collect()
                        } else {
                            dest_ids
                                .iter()
                                .filter_map(|id| store.primary.get(id))
                                .filter(|r| r.source_node_id == source_node_id)
                                .cloned()
                                .collect()
                        }
                    }
                    _ => vec![],
                }
            }
        }
    }

    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Option<String> {
        let store = self.routes.read();
        let ids = store.by_src.get(ALL_NODES_ID)?;
        let matching: Vec<&Route> = ids
            .iter()
            .filter_map(|id| store.primary.get(id))
            .filter(|r| {
                r.component0 == component0
                    && r.component1 == component1
                    && r.component2 == component2
                    && r.component_id == component_id
            })
            .collect();
        matching
            .into_iter()
            .max_by_key(|r| r.last_updated)
            .map(|r| r.dest_node_id.clone())
    }

    async fn get_routes_by_link_id(&self, link_id: &str) -> Vec<Route> {
        let store = self.routes.read();
        store
            .by_link
            .get(link_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| store.primary.get(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn delete_route(&self, route_id: i64) -> Result<()> {
        let mut store = self.routes.write();
        let route = store
            .primary
            .remove(&route_id)
            .ok_or(Error::RouteNotFound { id: route_id })?;
        store.index_remove(&route);
        Ok(())
    }

    async fn mark_route_deleted(&self, route_id: i64) -> Result<()> {
        let mut store = self.routes.write();
        let route = store
            .primary
            .get_mut(&route_id)
            .ok_or(Error::RouteNotFound { id: route_id })?;
        route.deleted = true;
        route.last_updated = SystemTime::now();
        Ok(())
    }

    async fn mark_route_applied(&self, route_id: i64) -> Result<()> {
        let mut store = self.routes.write();
        let route = store
            .primary
            .get_mut(&route_id)
            .ok_or(Error::RouteNotFound { id: route_id })?;
        route.status = RouteStatus::Applied;
        route.status_msg.clear();
        route.last_updated = SystemTime::now();
        Ok(())
    }

    async fn mark_route_failed(&self, route_id: i64, msg: &str) -> Result<()> {
        let mut store = self.routes.write();
        let route = store
            .primary
            .get_mut(&route_id)
            .ok_or(Error::RouteNotFound { id: route_id })?;
        route.status = RouteStatus::Failed;
        route.status_msg = msg.to_string();
        route.last_updated = SystemTime::now();
        Ok(())
    }

    // ── Links ──────────────────────────────────────────────────────────────

    async fn add_link(&self, mut link: Link) -> Result<Link> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
        }
        let mut store = self.links.write();
        link.last_updated = SystemTime::now();
        let key = link.storage_key();
        // Only update indices for new keys; overwriting an existing key leaves
        // the index entry intact (HashSet::insert is idempotent anyway).
        if !store.primary.contains_key(&key) {
            store.index_add(&link);
        }
        store.primary.insert(key, link.clone());
        Ok(link)
    }

    async fn update_link(&self, mut link: Link) -> Result<()> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
        }
        let mut store = self.links.write();
        let key = link.storage_key();
        if !store.primary.contains_key(&key) {
            return Err(Error::LinkNotFound {
                id: link.link_id.clone(),
            });
        }
        // The key fields (link_id, source, dest, endpoint) are immutable in an
        // update — only status/deleted/etc. change — so no index update needed.
        link.last_updated = SystemTime::now();
        store.primary.insert(key, link);
        Ok(())
    }

    async fn delete_link(&self, link: &Link) -> Result<()> {
        let mut store = self.links.write();
        let key = link.storage_key();
        let removed = store
            .primary
            .remove(&key)
            .ok_or_else(|| Error::LinkNotFound {
                id: link.link_id.clone(),
            })?;
        store.index_remove(&removed);
        Ok(())
    }

    async fn get_link(
        &self,
        link_id: &str,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link> {
        let store = self.links.read();
        let keys = store.by_link_id.get(link_id)?;
        let mut latest: Option<&Link> = None;
        for key in keys {
            if let Some(link) = store.primary.get(key) {
                if link.deleted {
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
                    Some(prev) if link.last_updated > prev.last_updated => latest = Some(link),
                    _ => {}
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
        let store = self.links.read();
        let mut latest: Option<&Link> = None;

        // Forward: source_node_id → dest_node_id.
        for key in store.by_src.get(source_node_id).into_iter().flatten() {
            if let Some(link) = store.primary.get(key)
                && !link.deleted
                && link.dest_node_id == dest_node_id
            {
                match latest {
                    None => latest = Some(link),
                    Some(prev) if link.last_updated > prev.last_updated => latest = Some(link),
                    _ => {}
                }
            }
        }
        // Reverse: dest_node_id → source_node_id.
        for key in store.by_src.get(dest_node_id).into_iter().flatten() {
            if let Some(link) = store.primary.get(key)
                && !link.deleted
                && link.dest_node_id == source_node_id
            {
                match latest {
                    None => latest = Some(link),
                    Some(prev) if link.last_updated > prev.last_updated => latest = Some(link),
                    _ => {}
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
        let store = self.links.read();
        let keys = store.by_src.get(source_node_id)?;
        let mut latest: Option<&Link> = None;
        for key in keys {
            if let Some(link) = store.primary.get(key)
                && !link.deleted
                && link.dest_endpoint == dest_endpoint
            {
                match latest {
                    None => latest = Some(link),
                    Some(prev) if link.last_updated > prev.last_updated => latest = Some(link),
                    _ => {}
                }
            }
        }
        latest.cloned()
    }

    async fn get_links_for_node(&self, node_id: &str) -> Vec<Link> {
        let store = self.links.read();
        let mut keys: HashSet<String> = HashSet::new();
        if let Some(src_keys) = store.by_src.get(node_id) {
            keys.extend(src_keys.iter().cloned());
        }
        if let Some(dst_keys) = store.by_dest.get(node_id) {
            keys.extend(dst_keys.iter().cloned());
        }
        keys.iter()
            .filter_map(|k| store.primary.get(k))
            .cloned()
            .collect()
    }

    async fn filter_links_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Vec<Link> {
        let store = self.links.read();
        match (source_node_id.is_empty(), dest_node_id.is_empty()) {
            (true, true) => store.primary.values().cloned().collect(),
            (false, true) => store
                .by_src
                .get(source_node_id)
                .map(|keys| {
                    keys.iter()
                        .filter_map(|k| store.primary.get(k))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            (true, false) => store
                .by_dest
                .get(dest_node_id)
                .map(|keys| {
                    keys.iter()
                        .filter_map(|k| store.primary.get(k))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            (false, false) => store
                .by_src
                .get(source_node_id)
                .map(|keys| {
                    keys.iter()
                        .filter_map(|k| store.primary.get(k))
                        .filter(|l| l.dest_node_id == dest_node_id)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    async fn list_all_links(&self) -> Vec<Link> {
        self.links.read().primary.values().cloned().collect()
    }

    // ── Channels ───────────────────────────────────────────────────────────

    async fn save_channel(&self, channel_id: &str, moderators: Vec<String>) -> Result<()> {
        let mut channels = self.channels.write();
        if channels.contains_key(channel_id) {
            return Err(Error::ChannelAlreadyExists {
                id: channel_id.to_string(),
            });
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

    async fn delete_channel(&self, channel_id: &str) -> Result<()> {
        let mut channels = self.channels.write();
        if channels.remove(channel_id).is_none() {
            return Err(Error::ChannelNotFound {
                id: channel_id.to_string(),
            });
        }
        Ok(())
    }

    async fn get_channel(&self, channel_id: &str) -> Option<Channel> {
        self.channels.read().get(channel_id).cloned()
    }

    async fn update_channel(&self, channel: Channel) -> Result<()> {
        if channel.id.is_empty() {
            return Err(Error::EmptyChannelId);
        }
        let mut channels = self.channels.write();
        if !channels.contains_key(&channel.id) {
            return Err(Error::ChannelNotFound {
                id: channel.id.clone(),
            });
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
    async fn add_link_empty_link_id_returns_error() {
        // Callers must supply a non-empty link_id; auto-generation is not permitted.
        let db = db();
        let l = make_link("src", "dst", "ep:8080", "");
        assert!(matches!(
            db.add_link(l).await,
            Err(Error::LinkMissingFields)
        ));
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

    // ── Index consistency tests ────────────────────────────────────────────

    #[tokio::test]
    async fn delete_route_removes_from_index() {
        let db = db();
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.delete_route(r.id).await.unwrap();
        // Index should no longer return this route.
        assert!(db.get_routes_for_node_id("src").await.is_empty());
        assert!(db.get_routes_for_dest_node_id("dst").await.is_empty());
        assert!(db.get_routes_by_link_id("lnk").await.is_empty());
    }

    #[tokio::test]
    async fn delete_link_removes_from_index() {
        let db = db();
        let l = db
            .add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        db.delete_link(&l).await.unwrap();
        assert!(db.get_links_for_node("src").await.is_empty());
        assert!(db.get_links_for_node("dst").await.is_empty());
        assert!(db.get_link("lid", "src", "dst").await.is_none());
    }

    #[tokio::test]
    async fn filter_routes_both_src_and_dest() {
        let db = db();
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        // Different dest, same src.
        let mut r2 = make_route("src", "other", "lnk2");
        r2.component1 = "ns2".to_string();
        db.add_route(r2).await.unwrap();

        let found = db.filter_routes_by_src_dest("src", "dst").await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dest_node_id, "dst");
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

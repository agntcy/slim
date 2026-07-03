// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod inmemory;
pub mod model;
pub mod schema;
pub mod sqlite;

pub use model::*;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;

/// Trait for all storage backends used by the control plane.
#[async_trait]
pub trait DataAccess: Send + Sync {
    // ── Nodes ──────────────────────────────────────────────────────────────

    async fn list_nodes(&self) -> Result<Vec<Node>>;
    async fn get_node(&self, id: &str) -> Result<Option<Node>>;
    /// Returns (node_id, conn_details_changed).
    async fn save_node(&self, node: Node) -> Result<(String, bool)>;
    async fn delete_node(&self, id: &str) -> Result<()>;

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, route: Route) -> Result<Route>;
    async fn get_route_by_id(&self, route_id: &str) -> Result<Option<Route>>;
    async fn get_routes_for_node(&self, node_id: &str) -> Result<Vec<Route>>;
    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Result<Vec<Route>>;
    async fn get_routes_for_dest_node_id_and_name(
        &self,
        node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
    ) -> Result<Vec<Route>>;
    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &RouteName<'_>,
        dest_node_id: &str,
        link_id: Option<&str>,
    ) -> Result<Option<Route>>;
    async fn filter_routes_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Vec<Route>>;
    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
    ) -> Result<Option<String>>;
    async fn get_routes_by_link_id(&self, link_id: &str) -> Result<Vec<Route>>;
    async fn delete_route(&self, route_id: &str) -> Result<()>;
    async fn mark_route_deleted(&self, route_id: &str) -> Result<()>;
    async fn mark_route_applied(&self, route_id: &str) -> Result<()>;
    async fn mark_route_failed(&self, route_id: &str, msg: &str) -> Result<()>;
    /// Update the link_id on an existing route and reset status to Pending.
    async fn update_route_link_id(&self, route_id: &str, link_id: &str) -> Result<()>;

    // ── Links ──────────────────────────────────────────────────────────────

    /// Persist a new link record.
    ///
    /// **Contract:** `link.link_id` MUST be non-empty; callers are responsible
    /// for generating the ID (e.g. via `Uuid::new_v4()`) before calling this
    /// method.  Passing an empty `link_id` is an error and implementations MUST
    /// return [`Error::LinkMissingFields`].  Auto-generating an ID inside the
    /// implementation is not permitted — it would silently diverge across
    /// backends and make idempotency impossible for callers.
    async fn add_link(&self, link: Link) -> Result<Link>;
    async fn update_link(&self, link: Link) -> Result<()>;
    async fn delete_link(&self, link: &Link) -> Result<()>;
    async fn get_link(
        &self,
        link_id: &str,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Option<Link>>;
    async fn find_link_between_nodes(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Option<Link>>;
    /// Find a non-deleted link between two domains (in either direction).
    async fn find_link_between_domains(
        &self,
        domain_a: &str,
        domain_b: &str,
    ) -> Result<Option<Link>>;

    /// Atomically check for an existing non-deleted link between the two nodes
    /// (in either direction) and insert `link` only if none exists.
    ///
    /// Returns `Ok((link, true))` if a new link was created, or
    /// `Ok((existing, false))` if a link already existed.
    async fn find_or_create_link(&self, link: Link) -> Result<(Link, bool)>;
    async fn get_link_for_source_and_endpoint(
        &self,
        source_node_id: &str,
        dest_endpoint: &str,
    ) -> Result<Option<Link>>;
    async fn get_links_for_node(&self, node_id: &str) -> Result<Vec<Link>>;
    /// Return links filtered by source and/or destination node.
    ///
    /// An empty string for either parameter is treated as "any" (no filter on
    /// that dimension).  Passing both empty is equivalent to [`list_all_links`].
    async fn filter_links_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Vec<Link>>;
    /// Return every link record in the store (no filtering).
    async fn list_all_links(&self) -> Result<Vec<Link>>;

    /// Atomically claim an unclaimed link for a destination node.
    ///
    /// Succeeds only if the link exists with `dest_node_id == ""` and
    /// `dest_domain == dest_domain`. On success, sets `dest_node_id` to
    /// `claimant_node_id` and `status` to `Applied`.
    ///
    /// Returns `Ok(Some(link))` if claimed, `Ok(None)` if already claimed or
    /// not found.
    async fn claim_link(
        &self,
        link_id: &str,
        dest_domain: &str,
        claimant_node_id: &str,
    ) -> Result<Option<Link>>;

    // ── Topology (API-managed mode) ───────────────────────────────────────

    async fn create_segment(&self, name: &str) -> Result<TopologySegment>;
    async fn delete_segment(&self, id: &str) -> Result<()>;
    async fn get_segment_by_name(&self, name: &str) -> Result<Option<TopologySegment>>;
    async fn list_topology_segments(&self) -> Result<Vec<TopologySegment>>;
    async fn add_link_to_segment(
        &self,
        segment_id: &str,
        source_domain: &str,
        dest_domain: &str,
    ) -> Result<()>;
    async fn delete_link_from_segment(
        &self,
        segment_id: &str,
        source_domain: &str,
        dest_domain: &str,
    ) -> Result<()>;
    async fn get_links_for_segment(&self, segment_id: &str) -> Result<Vec<(String, String)>>;
    /// Wipe runtime state (nodes, links, routes) but keep topology config.
    /// Used on startup in API-managed mode.
    async fn clear_runtime_state(&self) -> Result<()>;
    /// Wipe all state: runtime + topology config.
    /// Used on startup in config-managed mode.
    async fn clear_all_state(&self) -> Result<()>;
}

pub type SharedDb = Arc<dyn DataAccess>;

/// Open the database backend specified by the configuration.
pub async fn open(cfg: &crate::config::DatabaseConfig) -> anyhow::Result<SharedDb> {
    match cfg {
        crate::config::DatabaseConfig::InMemory => {
            tracing::info!("using in-memory database");
            Ok(inmemory::InMemoryDb::shared())
        }
        crate::config::DatabaseConfig::Sqlite { path } => {
            tracing::info!("using sqlite database at {path}");
            sqlite::SqliteDb::shared(path)
                .await
                .map_err(|e| anyhow::anyhow!(e))
        }
    }
}

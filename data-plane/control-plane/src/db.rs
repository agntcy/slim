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

    async fn list_nodes(&self) -> Vec<Node>;
    async fn get_node(&self, id: &str) -> Option<Node>;
    /// Returns (node_id, conn_details_changed).
    async fn save_node(&self, node: Node) -> Result<(String, bool)>;
    async fn delete_node(&self, id: &str) -> Result<()>;

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, route: Route) -> Result<Route>;
    async fn get_route_by_id(&self, route_id: i64) -> Option<Route>;
    async fn get_routes_for_node_id(&self, node_id: &str) -> Vec<Route>;
    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Vec<Route>;
    async fn get_routes_for_dest_node_id_and_name(
        &self,
        node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Vec<Route>;
    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &SubscriptionName<'_>,
        dest_node_id: &str,
        link_id: &str,
    ) -> Option<Route>;
    async fn filter_routes_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Vec<Route>;
    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Option<String>;
    async fn get_routes_by_link_id(&self, link_id: &str) -> Vec<Route>;
    async fn delete_route(&self, route_id: i64) -> Result<()>;
    async fn mark_route_deleted(&self, route_id: i64) -> Result<()>;
    async fn mark_route_applied(&self, route_id: i64) -> Result<()>;
    async fn mark_route_failed(&self, route_id: i64, msg: &str) -> Result<()>;
    /// Update the link_id on an existing route and reset status to Pending.
    async fn update_route_link_id(&self, route_id: i64, link_id: &str) -> Result<()>;
    /// Restore a soft-deleted route: set deleted=false, status=Pending,
    /// and update the link_id to the current link between the nodes.
    async fn restore_route(&self, route_id: i64, link_id: &str) -> Result<()>;

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
    ) -> Option<Link>;
    async fn find_link_between_nodes(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link>;

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
    ) -> Option<Link>;
    async fn get_links_for_node(&self, node_id: &str) -> Vec<Link>;
    /// Return links filtered by source and/or destination node.
    ///
    /// An empty string for either parameter is treated as "any" (no filter on
    /// that dimension).  Passing both empty is equivalent to [`list_all_links`].
    async fn filter_links_by_src_dest(&self, source_node_id: &str, dest_node_id: &str)
    -> Vec<Link>;
    /// Return every link record in the store (no filtering).
    async fn list_all_links(&self) -> Vec<Link>;
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

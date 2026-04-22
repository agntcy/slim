// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod inmemory;
pub mod model;
pub mod schema;
pub mod sqlite;

pub use model::*;

use std::sync::Arc;

use async_trait::async_trait;

/// Trait for all storage backends used by the control plane.
#[async_trait]
pub trait DataAccess: Send + Sync {
    // ── Nodes ──────────────────────────────────────────────────────────────

    async fn list_nodes(&self) -> Vec<Node>;
    async fn get_node(&self, id: &str) -> Option<Node>;
    /// Returns (node_id, conn_details_changed, err).
    async fn save_node(&self, node: Node) -> Result<(String, bool), String>;
    async fn delete_node(&self, id: &str) -> Result<(), String>;

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, route: Route) -> Result<Route, String>;
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
    async fn delete_route(&self, route_id: i64) -> Result<(), String>;
    async fn mark_route_deleted(&self, route_id: i64) -> Result<(), String>;
    async fn mark_route_applied(&self, route_id: i64) -> Result<(), String>;
    async fn mark_route_failed(&self, route_id: i64, msg: &str) -> Result<(), String>;
    async fn repoint_route(
        &self,
        route_id: i64,
        link_id: &str,
        status: RouteStatus,
        msg: &str,
    ) -> Result<(), String>;

    // ── Links ──────────────────────────────────────────────────────────────

    async fn add_link(&self, link: Link) -> Result<Link, String>;
    async fn update_link(&self, link: Link) -> Result<(), String>;
    async fn delete_link(&self, link: &Link) -> Result<(), String>;
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
    async fn get_link_for_source_and_endpoint(
        &self,
        source_node_id: &str,
        dest_endpoint: &str,
    ) -> Option<Link>;
    async fn get_links_for_node(&self, node_id: &str) -> Vec<Link>;

    // ── Channels ───────────────────────────────────────────────────────────

    async fn save_channel(&self, channel_id: &str, moderators: Vec<String>) -> Result<(), String>;
    async fn delete_channel(&self, channel_id: &str) -> Result<(), String>;
    async fn get_channel(&self, channel_id: &str) -> Option<Channel>;
    async fn update_channel(&self, channel: Channel) -> Result<(), String>;
    async fn list_channels(&self) -> Vec<Channel>;
}

pub type SharedDb = Arc<dyn DataAccess>;

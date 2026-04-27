// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use diesel::OptionalExtension;
use diesel::prelude::*;
use diesel::serialize::{self, IsNull, Output, ToSql};
use diesel::sql_types::{Integer, Text};
use diesel::sqlite::Sqlite;
use diesel::sqlite::SqliteConnection;
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use diesel_async::SimpleAsyncConnection;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::sync_connection_wrapper::SyncConnectionWrapper;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness as _, embed_migrations};

use super::model::{
    ALL_NODES_ID, Channel, ConnDetailsJson, DbTimestamp, JsonStrings, Link, LinkStatus, Node,
    Route, RouteStatus, SubscriptionName, has_connection_details_changed,
};
use super::schema::{channels, links, nodes, routes};
use super::{DataAccess, SharedDb};
use crate::error::{Error, Result};

// ─── Embedded migrations ──────────────────────────────────────────────────────

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("src/db/sqlite/migrations");

// ─── Connection type alias ────────────────────────────────────────────────────

type AsyncSqliteConn = SyncConnectionWrapper<SqliteConnection>;

// ─── SQLite-specific ToSql impls ──────────────────────────────────────────────

impl ToSql<Integer, Sqlite> for RouteStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let v: i32 = match self {
            RouteStatus::Pending => 0,
            RouteStatus::Applied => 1,
            RouteStatus::Failed => 2,
        };
        out.set_value(v);
        Ok(IsNull::No)
    }
}

impl ToSql<Integer, Sqlite> for LinkStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let v: i32 = match self {
            LinkStatus::Pending => 0,
            LinkStatus::Applied => 1,
            LinkStatus::Failed => 2,
        };
        out.set_value(v);
        Ok(IsNull::No)
    }
}

impl ToSql<Text, Sqlite> for ConnDetailsJson {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let s = serde_json::to_string(&self.0).map_err(|e| format!("{e}"))?;
        out.set_value(s);
        Ok(IsNull::No)
    }
}

impl ToSql<Text, Sqlite> for JsonStrings {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let s = serde_json::to_string(&self.0).map_err(|e| format!("{e}"))?;
        out.set_value(s);
        Ok(IsNull::No)
    }
}

// ─── SqliteDb ─────────────────────────────────────────────────────────────────

pub struct SqliteDb {
    pool: Pool<AsyncSqliteConn>,
}

impl SqliteDb {
    pub async fn shared(path: &str) -> Result<SharedDb, String> {
        // Run embedded migrations on a dedicated sync connection via spawn_blocking.
        // AsyncMigrationHarness::block_in_place requires the multi-threaded runtime,
        // which we cannot guarantee in tests; spawn_blocking works on any runtime.
        let path_owned = path.to_string();
        tokio::task::spawn_blocking(move || {
            SqliteConnection::establish(&path_owned)
                .map_err(|e| format!("failed to open db: {e}"))
                .and_then(|mut conn| {
                    conn.run_pending_migrations(MIGRATIONS)
                        .map(|_| ())
                        .map_err(|e| format!("migration failed: {e}"))
                })
        })
        .await
        .map_err(|e| format!("migration task panicked: {e}"))??;

        // Build async pool.
        // Each connection enables WAL mode (one writer + concurrent readers) and a
        // 5-second busy timeout so that concurrent writers retry instead of
        // immediately returning "database is locked".
        let mut manager_config = ManagerConfig::<AsyncSqliteConn>::default();
        manager_config.custom_setup = Box::new(|url| {
            let url = url.to_string();
            Box::pin(async move {
                let mut conn: AsyncSqliteConn = AsyncSqliteConn::establish(&url).await?;
                conn.batch_execute("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                    .await
                    .map_err(|e| diesel::ConnectionError::BadConnection(e.to_string()))?;
                Ok(conn)
            })
        });
        let manager =
            AsyncDieselConnectionManager::<AsyncSqliteConn>::new_with_config(path, manager_config);
        let pool = Pool::builder(manager)
            .max_size(16)
            .build()
            .map_err(|e| format!("failed to build connection pool: {e}"))?;

        Ok(Arc::new(Self { pool }))
    }
}

// ─── DataAccess impl ──────────────────────────────────────────────────────────

#[async_trait]
impl DataAccess for SqliteDb {
    // ── Nodes ──────────────────────────────────────────────────────────────

    async fn list_nodes(&self) -> Vec<Node> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        nodes::table
            .load::<Node>(&mut conn)
            .await
            .unwrap_or_default()
    }

    async fn get_node(&self, id: &str) -> Option<Node> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        nodes::table
            .find(id)
            .first::<Node>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn save_node(&self, node: Node) -> Result<(String, bool)> {
        let existing = self.get_node(&node.id).await;
        let conn_details_changed = existing
            .as_ref()
            .map(|e| has_connection_details_changed(&e.conn_details, &node.conn_details))
            .unwrap_or(false);

        let node_id = node.id.clone();
        let group_name = node.group_name.clone();
        let conn_json = ConnDetailsJson(node.conn_details.clone());
        let ts = DbTimestamp::from(SystemTime::now());

        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "save_node pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(nodes::table)
            .values(node)
            .on_conflict(nodes::id)
            .do_update()
            .set((
                nodes::group_name.eq(group_name),
                nodes::conn_details.eq(conn_json),
                nodes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "save_node",
                msg: e.to_string(),
            })?;

        Ok((node_id, conn_details_changed))
    }

    async fn delete_node(&self, id: &str) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_node pool",
            msg: e.to_string(),
        })?;
        let n = diesel::delete(nodes::table.find(id))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "delete_node",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::NodeNotFound { id: id.to_string() });
        }
        Ok(())
    }

    // ── Routes ─────────────────────────────────────────────────────────────

    async fn add_route(&self, mut route: Route) -> Result<Route> {
        route.id = route.compute_id();
        route.last_updated = SystemTime::now();
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "add_route pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(routes::table)
            .values(route.clone())
            .execute(&mut conn)
            .await
            .map_err(|e| {
                // UNIQUE constraint violation on the primary key means the route
                // already exists — surface the typed error so callers can match on it.
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint failed") || msg.contains("AlreadyExists") {
                    Error::RouteAlreadyExists {
                        id: route.to_string(),
                    }
                } else {
                    Error::DbError {
                        context: "add_route",
                        msg,
                    }
                }
            })?;
        Ok(route)
    }

    async fn get_route_by_id(&self, route_id: i64) -> Option<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        routes::table
            .find(route_id)
            .first::<Route>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn get_routes_for_node_id(&self, node_id: &str) -> Vec<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        routes::table
            .filter(routes::source_node_id.eq(node_id))
            .load::<Route>(&mut conn)
            .await
            .unwrap_or_default()
    }

    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Vec<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        routes::table
            .filter(routes::dest_node_id.eq(node_id))
            .filter(routes::source_node_id.ne(ALL_NODES_ID))
            .load::<Route>(&mut conn)
            .await
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
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        let mut q = routes::table
            .filter(routes::dest_node_id.eq(node_id))
            .filter(routes::component0.eq(component0))
            .filter(routes::component1.eq(component1))
            .filter(routes::component2.eq(component2))
            .into_boxed();
        if let Some(id) = component_id {
            q = q.filter(routes::component_id.eq(id));
        } else {
            q = q.filter(routes::component_id.is_null());
        }
        q.load::<Route>(&mut conn).await.unwrap_or_default()
    }

    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &SubscriptionName<'_>,
        dest_node_id: &str,
        link_id: &str,
    ) -> Option<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        let mut q = routes::table
            .filter(routes::source_node_id.eq(src_node_id))
            .filter(routes::component0.eq(name.component0))
            .filter(routes::component1.eq(name.component1))
            .filter(routes::component2.eq(name.component2))
            .into_boxed();
        if !dest_node_id.is_empty() {
            q = q.filter(routes::dest_node_id.eq(dest_node_id));
        }
        if !link_id.is_empty() {
            q = q.filter(routes::link_id.eq(link_id));
        }
        if let Some(id) = name.component_id {
            q = q.filter(routes::component_id.eq(id));
        } else {
            q = q.filter(routes::component_id.is_null());
        }
        q.limit(1).first::<Route>(&mut conn).await.optional().ok()?
    }

    async fn filter_routes_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Vec<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        let mut q = routes::table.into_boxed();
        if !source_node_id.is_empty() {
            q = q.filter(routes::source_node_id.eq(source_node_id));
        }
        if !dest_node_id.is_empty() {
            q = q.filter(routes::dest_node_id.eq(dest_node_id));
        }
        q.load::<Route>(&mut conn).await.unwrap_or_default()
    }

    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
    ) -> Option<String> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        let mut q = routes::table
            .filter(routes::source_node_id.eq(ALL_NODES_ID))
            .filter(routes::component0.eq(component0))
            .filter(routes::component1.eq(component1))
            .filter(routes::component2.eq(component2))
            .order(routes::last_updated.desc())
            .select(routes::dest_node_id)
            .into_boxed();
        if let Some(id) = component_id {
            q = q.filter(routes::component_id.eq(id));
        } else {
            q = q.filter(routes::component_id.is_null());
        }
        q.first::<String>(&mut conn).await.optional().ok()?
    }

    async fn get_routes_by_link_id(&self, link_id: &str) -> Vec<Route> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        routes::table
            .filter(routes::link_id.eq(link_id))
            .load::<Route>(&mut conn)
            .await
            .unwrap_or_default()
    }

    async fn delete_route(&self, route_id: i64) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_route pool",
            msg: e.to_string(),
        })?;
        let n = diesel::delete(routes::table.find(route_id))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "delete_route",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound { id: route_id });
        }
        Ok(())
    }

    async fn mark_route_deleted(&self, route_id: i64) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "mark_route_deleted pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((routes::deleted.eq(true), routes::last_updated.eq(ts)))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "mark_route_deleted",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound { id: route_id });
        }
        Ok(())
    }

    async fn mark_route_applied(&self, route_id: i64) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "mark_route_applied pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((
                routes::status.eq(RouteStatus::Applied),
                routes::status_msg.eq(""),
                routes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "mark_route_applied",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound { id: route_id });
        }
        Ok(())
    }

    async fn mark_route_failed(&self, route_id: i64, msg: &str) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "mark_route_failed pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((
                routes::status.eq(RouteStatus::Failed),
                routes::status_msg.eq(msg),
                routes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "mark_route_failed",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound { id: route_id });
        }
        Ok(())
    }

    async fn repoint_route(
        &self,
        route_id: i64,
        link_id: &str,
        status: RouteStatus,
        msg: &str,
    ) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "repoint_route pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((
                routes::link_id.eq(link_id),
                routes::status.eq(status),
                routes::status_msg.eq(msg),
                routes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "repoint_route",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound { id: route_id });
        }
        Ok(())
    }

    // ── Links ──────────────────────────────────────────────────────────────

    async fn add_link(&self, mut link: Link) -> Result<Link> {
        if link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
        }
        if link.link_id.is_empty() {
            if let Some(existing) = self
                .get_link_for_source_and_endpoint(&link.source_node_id, &link.dest_endpoint)
                .await
            {
                link.link_id = existing.link_id;
            } else {
                link.link_id = uuid::Uuid::new_v4().to_string();
            }
        }
        link.last_updated = SystemTime::now();
        let ts = DbTimestamp::from(link.last_updated);
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "add_link pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(links::table)
            .values(link.clone())
            .on_conflict((
                links::link_id,
                links::source_node_id,
                links::dest_node_id,
                links::dest_endpoint,
            ))
            .do_update()
            .set((
                links::conn_config_data.eq(&link.conn_config_data),
                links::status.eq(link.status),
                links::status_msg.eq(&link.status_msg),
                links::deleted.eq(link.deleted),
                links::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "add_link",
                msg: e.to_string(),
            })?;
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
        link.last_updated = SystemTime::now();
        let ts = DbTimestamp::from(link.last_updated);
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "update_link pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(
            links::table
                .filter(links::link_id.eq(&link.link_id))
                .filter(links::source_node_id.eq(&link.source_node_id))
                .filter(links::dest_node_id.eq(&link.dest_node_id))
                .filter(links::dest_endpoint.eq(&link.dest_endpoint)),
        )
        .set((
            links::conn_config_data.eq(&link.conn_config_data),
            links::status.eq(link.status),
            links::status_msg.eq(&link.status_msg),
            links::deleted.eq(link.deleted),
            links::last_updated.eq(ts),
        ))
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "update_link",
            msg: e.to_string(),
        })?;
        if n == 0 {
            return Err(Error::LinkNotFound {
                id: link.link_id.clone(),
            });
        }
        Ok(())
    }

    async fn delete_link(&self, link: &Link) -> Result<()> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
        }
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_link pool",
            msg: e.to_string(),
        })?;
        let n = diesel::delete(
            links::table
                .filter(links::link_id.eq(&link.link_id))
                .filter(links::source_node_id.eq(&link.source_node_id))
                .filter(links::dest_node_id.eq(&link.dest_node_id))
                .filter(links::dest_endpoint.eq(&link.dest_endpoint)),
        )
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "delete_link",
            msg: e.to_string(),
        })?;
        if n == 0 {
            return Err(Error::LinkNotFound {
                id: link.link_id.clone(),
            });
        }
        Ok(())
    }

    async fn get_link(
        &self,
        link_id: &str,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        links::table
            .filter(links::link_id.eq(link_id))
            .filter(links::deleted.eq(false))
            .filter(
                links::source_node_id
                    .eq(source_node_id)
                    .and(links::dest_node_id.eq(dest_node_id))
                    .or(links::source_node_id
                        .eq(dest_node_id)
                        .and(links::dest_node_id.eq(source_node_id))),
            )
            .order(links::last_updated.desc())
            .first::<Link>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn find_link_between_nodes(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Option<Link> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        links::table
            .filter(links::deleted.eq(false))
            .filter(
                links::source_node_id
                    .eq(source_node_id)
                    .and(links::dest_node_id.eq(dest_node_id))
                    .or(links::source_node_id
                        .eq(dest_node_id)
                        .and(links::dest_node_id.eq(source_node_id))),
            )
            .order(links::last_updated.desc())
            .first::<Link>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn get_link_for_source_and_endpoint(
        &self,
        source_node_id: &str,
        dest_endpoint: &str,
    ) -> Option<Link> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        links::table
            .filter(links::source_node_id.eq(source_node_id))
            .filter(links::dest_endpoint.eq(dest_endpoint))
            .filter(links::deleted.eq(false))
            .order(links::last_updated.desc())
            .first::<Link>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn get_links_for_node(&self, node_id: &str) -> Vec<Link> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        links::table
            .filter(
                links::source_node_id
                    .eq(node_id)
                    .or(links::dest_node_id.eq(node_id)),
            )
            .load::<Link>(&mut conn)
            .await
            .unwrap_or_default()
    }

    // ── Channels ───────────────────────────────────────────────────────────

    async fn save_channel(&self, channel_id: &str, moderators: Vec<String>) -> Result<()> {
        let channel = Channel {
            id: channel_id.to_string(),
            moderators,
            participants: vec![],
        };
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "save_channel pool",
            msg: e.to_string(),
        })?;
        let n = diesel::insert_into(channels::table)
            .values(channel)
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "save_channel",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::ChannelAlreadyExists {
                id: channel_id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_channel(&self, channel_id: &str) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_channel pool",
            msg: e.to_string(),
        })?;
        let n = diesel::delete(channels::table.find(channel_id))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "delete_channel",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::ChannelNotFound {
                id: channel_id.to_string(),
            });
        }
        Ok(())
    }

    async fn get_channel(&self, channel_id: &str) -> Option<Channel> {
        let Ok(mut conn) = self.pool.get().await else {
            return None;
        };
        channels::table
            .find(channel_id)
            .first::<Channel>(&mut conn)
            .await
            .optional()
            .ok()?
    }

    async fn update_channel(&self, channel: Channel) -> Result<()> {
        if channel.id.is_empty() {
            return Err(Error::EmptyChannelId);
        }
        let mods = JsonStrings(channel.moderators.clone());
        let parts = JsonStrings(channel.participants.clone());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "update_channel pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(channels::table.find(&channel.id))
            .set((
                channels::moderators.eq(mods),
                channels::participants.eq(parts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "update_channel",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::ChannelNotFound {
                id: channel.id.clone(),
            });
        }
        Ok(())
    }

    async fn list_channels(&self) -> Vec<Channel> {
        let Ok(mut conn) = self.pool.get().await else {
            return vec![];
        };
        channels::table
            .load::<Channel>(&mut conn)
            .await
            .unwrap_or_default()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DataAccess;
    use crate::db::model::ConnectionDetails;
    use std::time::SystemTime;
    use tempfile::NamedTempFile;

    async fn tmp_db() -> (NamedTempFile, Arc<dyn DataAccess>) {
        let f = NamedTempFile::new().unwrap();
        let db = SqliteDb::shared(f.path().to_str().unwrap()).await.unwrap();
        (f, db)
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
        let (_f, db) = tmp_db().await;
        let node = make_node("n1", Some("grp"));
        let (id, changed) = db.save_node(node).await.unwrap();
        assert_eq!(id, "n1");
        assert!(!changed);
        let got = db.get_node("n1").await.unwrap();
        assert_eq!(got.id, "n1");
        assert_eq!(got.group_name.as_deref(), Some("grp"));
    }

    #[tokio::test]
    async fn list_nodes_returns_all() {
        let (_f, db) = tmp_db().await;
        db.save_node(make_node("n1", None)).await.unwrap();
        db.save_node(make_node("n2", None)).await.unwrap();
        assert_eq!(db.list_nodes().await.len(), 2);
    }

    #[tokio::test]
    async fn delete_node() {
        let (_f, db) = tmp_db().await;
        db.save_node(make_node("n1", None)).await.unwrap();
        db.delete_node("n1").await.unwrap();
        assert!(db.get_node("n1").await.is_none());
    }

    #[tokio::test]
    async fn delete_node_not_found() {
        let (_f, db) = tmp_db().await;
        assert!(db.delete_node("missing").await.is_err());
    }

    #[tokio::test]
    async fn save_node_detects_conn_details_change() {
        let (_f, db) = tmp_db().await;
        db.save_node(make_node("n1", None)).await.unwrap();
        let mut updated = make_node("n1", None);
        updated.conn_details[0].endpoint = "n1:9999".to_string();
        let (_, changed) = db.save_node(updated).await.unwrap();
        assert!(changed);
    }

    // ── Route CRUD ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_get_route() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert_ne!(r.id, 0);
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.source_node_id, "src");
    }

    #[tokio::test]
    async fn add_duplicate_route_returns_error() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert!(db.add_route(make_route("src", "dst", "lnk")).await.is_err());
    }

    #[tokio::test]
    async fn get_routes_for_node_id() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.add_route(make_route("other", "dst", "lnk2"))
            .await
            .unwrap();
        let routes = db.get_routes_for_node_id("src").await;
        assert_eq!(routes.len(), 1);
    }

    #[tokio::test]
    async fn get_routes_for_dest_node_id_excludes_wildcard() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let mut wc = make_route(ALL_NODES_ID, "dst", "lnk2");
        wc.component1 = "ns2".to_string();
        db.add_route(wc).await.unwrap();
        let routes = db.get_routes_for_dest_node_id("dst").await;
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source_node_id, "src");
    }

    #[tokio::test]
    async fn mark_route_applied() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_applied(r.id).await.unwrap();
        assert_eq!(
            db.get_route_by_id(r.id).await.unwrap().status,
            RouteStatus::Applied
        );
    }

    #[tokio::test]
    async fn mark_route_failed() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_failed(r.id, "oops").await.unwrap();
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.status, RouteStatus::Failed);
        assert_eq!(got.status_msg, "oops");
    }

    #[tokio::test]
    async fn mark_route_deleted_and_delete() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_deleted(r.id).await.unwrap();
        assert!(db.get_route_by_id(r.id).await.unwrap().deleted);
        db.delete_route(r.id).await.unwrap();
        assert!(db.get_route_by_id(r.id).await.is_none());
    }

    #[tokio::test]
    async fn delete_route_not_found() {
        let (_f, db) = tmp_db().await;
        assert!(db.delete_route(9999).await.is_err());
    }

    #[tokio::test]
    async fn repoint_route() {
        let (_f, db) = tmp_db().await;
        let r = db
            .add_route(make_route("src", "dst", "old_lnk"))
            .await
            .unwrap();
        db.repoint_route(r.id, "new_lnk", RouteStatus::Pending, "")
            .await
            .unwrap();
        let got = db.get_route_by_id(r.id).await.unwrap();
        assert_eq!(got.link_id, "new_lnk");
    }

    #[tokio::test]
    async fn get_route_for_src_dest_name() {
        let (_f, db) = tmp_db().await;
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
    async fn filter_routes_by_src_dest() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert_eq!(db.filter_routes_by_src_dest("", "").await.len(), 1);
        assert_eq!(db.filter_routes_by_src_dest("src", "").await.len(), 1);
        assert_eq!(db.filter_routes_by_src_dest("", "dst").await.len(), 1);
        assert_eq!(db.filter_routes_by_src_dest("src", "dst").await.len(), 1);
        assert!(db.filter_routes_by_src_dest("other", "").await.is_empty());
    }

    #[tokio::test]
    async fn get_destination_node_id_for_name() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route(ALL_NODES_ID, "dst_node", "lnk"))
            .await
            .unwrap();
        let result = db
            .get_destination_node_id_for_name("org", "ns", "svc", Some(1))
            .await;
        assert_eq!(result.as_deref(), Some("dst_node"));
    }

    #[tokio::test]
    async fn get_routes_by_link_id() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "link-abc"))
            .await
            .unwrap();
        assert_eq!(db.get_routes_by_link_id("link-abc").await.len(), 1);
        assert!(db.get_routes_by_link_id("link-xyz").await.is_empty());
    }

    // ── Link CRUD ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_get_link() {
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(db.get_link("lid", "src", "dst").await.is_some());
    }

    #[tokio::test]
    async fn add_link_missing_fields_returns_error() {
        let (_f, db) = tmp_db().await;
        let mut l = make_link("src", "dst", "ep:8080", "lid");
        l.source_node_id = String::new();
        assert!(db.add_link(l).await.is_err());
    }

    #[tokio::test]
    async fn update_link_status() {
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        let mut updated = make_link("src", "dst", "ep:8080", "lid");
        updated.status = LinkStatus::Applied;
        db.update_link(updated).await.unwrap();
        assert_eq!(
            db.get_link("lid", "src", "dst").await.unwrap().status,
            LinkStatus::Applied
        );
    }

    #[tokio::test]
    async fn delete_link() {
        let (_f, db) = tmp_db().await;
        let l = db
            .add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        db.delete_link(&l).await.unwrap();
        assert!(db.get_link("lid", "src", "dst").await.is_none());
    }

    #[tokio::test]
    async fn find_link_between_nodes() {
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(db.find_link_between_nodes("src", "dst").await.is_some());
        assert!(db.find_link_between_nodes("dst", "src").await.is_some());
        assert!(db.find_link_between_nodes("a", "b").await.is_none());
    }

    #[tokio::test]
    async fn get_link_for_source_and_endpoint() {
        let (_f, db) = tmp_db().await;
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
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid1"))
            .await
            .unwrap();
        db.add_link(make_link("other", "src", "ep:9090", "lid2"))
            .await
            .unwrap();
        assert_eq!(db.get_links_for_node("src").await.len(), 2);
    }

    // ── Channel CRUD ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_get_channel() {
        let (_f, db) = tmp_db().await;
        db.save_channel("c1", vec!["mod".to_string()])
            .await
            .unwrap();
        let ch = db.get_channel("c1").await.unwrap();
        assert_eq!(ch.id, "c1");
        assert_eq!(ch.moderators, vec!["mod"]);
    }

    #[tokio::test]
    async fn save_channel_duplicate_returns_error() {
        let (_f, db) = tmp_db().await;
        db.save_channel("c1", vec![]).await.unwrap();
        assert!(db.save_channel("c1", vec![]).await.is_err());
    }

    #[tokio::test]
    async fn delete_channel() {
        let (_f, db) = tmp_db().await;
        db.save_channel("c1", vec![]).await.unwrap();
        db.delete_channel("c1").await.unwrap();
        assert!(db.get_channel("c1").await.is_none());
    }

    #[tokio::test]
    async fn delete_channel_not_found() {
        let (_f, db) = tmp_db().await;
        assert!(db.delete_channel("missing").await.is_err());
    }

    #[tokio::test]
    async fn update_channel() {
        let (_f, db) = tmp_db().await;
        db.save_channel("c1", vec!["mod".to_string()])
            .await
            .unwrap();
        db.update_channel(Channel {
            id: "c1".to_string(),
            moderators: vec!["mod".to_string()],
            participants: vec!["p1".to_string()],
        })
        .await
        .unwrap();
        assert_eq!(db.get_channel("c1").await.unwrap().participants, vec!["p1"]);
    }

    #[tokio::test]
    async fn update_channel_not_found_returns_error() {
        let (_f, db) = tmp_db().await;
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
        let (_f, db) = tmp_db().await;
        db.save_channel("c1", vec![]).await.unwrap();
        db.save_channel("c2", vec![]).await.unwrap();
        assert_eq!(db.list_channels().await.len(), 2);
    }
}

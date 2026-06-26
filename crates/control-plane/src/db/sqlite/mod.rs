// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use diesel::OptionalExtension;
use diesel::connection::SimpleConnection;
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
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use super::model::{
    ALL_NODES_ID, ConnDetailsJson, DbClientConfig, DbTimestamp, JsonStrings, Link, LinkStatus,
    Node, Route, RouteName, RouteStatus, TopologySegment, TopologySegmentGroup,
    TopologySegmentLink, has_connection_details_changed,
};
use super::schema::{
    links, nodes, routes, topology_segment_groups, topology_segment_links, topology_segments,
};
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
            RouteStatus::Deleted => 3,
        };
        out.set_value(v);
        Ok(IsNull::No)
    }
}

impl ToSql<Integer, Sqlite> for LinkStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let v: i32 = match self {
            LinkStatus::Pending => 0,
            LinkStatus::Connecting => 1,
            LinkStatus::Applied => 2,
            LinkStatus::Failed => 3,
            LinkStatus::Deleted => 4,
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

impl ToSql<Text, Sqlite> for DbClientConfig {
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
                    // Match pool connections: WAL + long busy wait so migrations do not fight
                    // concurrent pool opens during tests (avoids "database is locked").
                    conn.batch_execute(
                        "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=30000; PRAGMA synchronous=NORMAL;",
                    )
                    .map_err(|e| format!("pragma setup failed: {e}"))?;
                    conn.run_pending_migrations(MIGRATIONS)
                        .map(|_| ())
                        .map_err(|e| format!("migration failed: {e}"))
                })
        })
        .await
        .map_err(|e| format!("migration task panicked: {e}"))??;

        // Build async pool.
        // Each connection enables WAL mode (one writer + concurrent readers) and a
        // long busy timeout so concurrent writers (southbound + reconciler under test)
        // retry instead of returning "database is locked".
        let mut manager_config = ManagerConfig::<AsyncSqliteConn>::default();
        manager_config.custom_setup = Box::new(|url| {
            let url = url.to_string();
            Box::pin(async move {
                let mut conn: AsyncSqliteConn = AsyncSqliteConn::establish(&url).await?;
                conn.batch_execute(
                    "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=30000; PRAGMA synchronous=NORMAL;",
                )
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

    /// Set a route's status (and message) unless it has already been soft-deleted.
    /// Returns Ok(()) even when the route is missing or deleted — both are acceptable
    /// because a concurrent delete may have removed this route while the reconciler
    /// was waiting for an ack.
    async fn set_route_status(&self, route_id: &str, status: RouteStatus, msg: &str) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "set_route_status pool",
            msg: e.to_string(),
        })?;
        diesel::update(
            routes::table
                .find(route_id)
                .filter(routes::status.ne(RouteStatus::Deleted)),
        )
        .set((
            routes::status.eq(status),
            routes::status_msg.eq(msg),
            routes::last_updated.eq(ts),
        ))
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "set_route_status",
            msg: e.to_string(),
        })?;
        Ok(())
    }
}

// ─── DataAccess impl ──────────────────────────────────────────────────────────

#[async_trait]
impl DataAccess for SqliteDb {
    // ── Nodes ──────────────────────────────────────────────────────────────

    async fn list_nodes(&self) -> Result<Vec<Node>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "list_nodes pool",
            msg: e.to_string(),
        })?;
        nodes::table
            .load::<Node>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "list_nodes",
                msg: e.to_string(),
            })
    }

    async fn get_node(&self, id: &str) -> Result<Option<Node>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_node pool",
            msg: e.to_string(),
        })?;
        nodes::table
            .find(id)
            .first::<Node>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_node",
                msg: e.to_string(),
            })
    }

    async fn save_node(&self, node: Node) -> Result<(String, bool)> {
        let existing = self.get_node(&node.id).await?;
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
            .map_err(|e| match e {
                diesel::result::Error::DatabaseError(
                    diesel::result::DatabaseErrorKind::UniqueViolation,
                    _,
                ) => Error::RouteAlreadyExists {
                    id: route.to_string(),
                },
                other => Error::DbError {
                    context: "add_route",
                    msg: other.to_string(),
                },
            })?;
        Ok(route)
    }

    async fn get_route_by_id(&self, route_id: &str) -> Result<Option<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_route_by_id pool",
            msg: e.to_string(),
        })?;
        routes::table
            .find(route_id)
            .first::<Route>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_route_by_id",
                msg: e.to_string(),
            })
    }

    async fn get_routes_for_node(&self, node_id: &str) -> Result<Vec<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_routes_for_node pool",
            msg: e.to_string(),
        })?;
        routes::table
            .filter(routes::source_node_id.eq(node_id))
            .load::<Route>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_routes_for_node",
                msg: e.to_string(),
            })
    }

    async fn get_routes_for_dest_node_id(&self, node_id: &str) -> Result<Vec<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_routes_for_dest_node_id pool",
            msg: e.to_string(),
        })?;
        routes::table
            .filter(routes::dest_node_id.eq(node_id))
            .filter(routes::source_node_id.ne(ALL_NODES_ID))
            .load::<Route>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_routes_for_dest_node_id",
                msg: e.to_string(),
            })
    }

    async fn get_routes_for_dest_node_id_and_name(
        &self,
        node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
    ) -> Result<Vec<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_routes_for_dest_node_id_and_name pool",
            msg: e.to_string(),
        })?;
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
        q.load::<Route>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_routes_for_dest_node_id_and_name",
                msg: e.to_string(),
            })
    }

    async fn get_route_for_src_dest_name(
        &self,
        src_node_id: &str,
        name: &RouteName<'_>,
        dest_node_id: &str,
        link_id: Option<&str>,
    ) -> Result<Option<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_route_for_src_dest_name pool",
            msg: e.to_string(),
        })?;
        let mut q = routes::table
            .filter(routes::source_node_id.eq(src_node_id))
            .filter(routes::component0.eq(name.component0))
            .filter(routes::component1.eq(name.component1))
            .filter(routes::component2.eq(name.component2))
            .into_boxed();
        if !dest_node_id.is_empty() {
            q = q.filter(routes::dest_node_id.eq(dest_node_id));
        }
        if let Some(lid) = link_id {
            q = q.filter(routes::link_id.eq(lid));
        }
        if let Some(id) = name.component_id {
            q = q.filter(routes::component_id.eq(id));
        } else {
            q = q.filter(routes::component_id.is_null());
        }
        q.limit(1)
            .first::<Route>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_route_for_src_dest_name",
                msg: e.to_string(),
            })
    }

    async fn filter_routes_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Vec<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "filter_routes_by_src_dest pool",
            msg: e.to_string(),
        })?;
        let mut q = routes::table.into_boxed();
        if !source_node_id.is_empty() {
            q = q.filter(routes::source_node_id.eq(source_node_id));
        }
        if !dest_node_id.is_empty() {
            q = q.filter(routes::dest_node_id.eq(dest_node_id));
        }
        q.load::<Route>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "filter_routes_by_src_dest",
                msg: e.to_string(),
            })
    }

    async fn get_destination_node_id_for_name(
        &self,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<&str>,
    ) -> Result<Option<String>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_destination_node_id_for_name pool",
            msg: e.to_string(),
        })?;
        let mut q = routes::table
            .filter(routes::source_node_id.eq(ALL_NODES_ID))
            .filter(routes::status.ne(RouteStatus::Deleted))
            .filter(routes::component0.eq(component0))
            .filter(routes::component1.eq(component1))
            .filter(routes::component2.eq(component2))
            .order(routes::created_at.asc())
            .select(routes::dest_node_id)
            .into_boxed();
        if let Some(id) = component_id {
            q = q.filter(routes::component_id.eq(id));
        } else {
            q = q.filter(routes::component_id.is_null());
        }
        q.first::<String>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_destination_node_id_for_name",
                msg: e.to_string(),
            })
    }

    async fn get_routes_by_link_id(&self, link_id: &str) -> Result<Vec<Route>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_routes_by_link_id pool",
            msg: e.to_string(),
        })?;
        routes::table
            .filter(routes::link_id.eq(link_id))
            .load::<Route>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_routes_by_link_id",
                msg: e.to_string(),
            })
    }

    async fn delete_route(&self, route_id: &str) -> Result<()> {
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
            return Err(Error::RouteNotFound {
                id: route_id.to_string(),
            });
        }
        Ok(())
    }

    async fn mark_route_deleted(&self, route_id: &str) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "mark_route_deleted pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((
                routes::status.eq(RouteStatus::Deleted),
                routes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "mark_route_deleted",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound {
                id: route_id.to_string(),
            });
        }
        Ok(())
    }

    async fn mark_route_applied(&self, route_id: &str) -> Result<()> {
        self.set_route_status(route_id, RouteStatus::Applied, "")
            .await
    }

    async fn mark_route_failed(&self, route_id: &str, msg: &str) -> Result<()> {
        self.set_route_status(route_id, RouteStatus::Failed, msg)
            .await
    }

    async fn update_route_link_id(&self, route_id: &str, link_id: &str) -> Result<()> {
        let ts = DbTimestamp::from(SystemTime::now());
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "update_route_link_id pool",
            msg: e.to_string(),
        })?;
        let n = diesel::update(routes::table.find(route_id))
            .set((
                routes::link_id.eq(Some(link_id)),
                routes::status.eq(RouteStatus::Pending),
                routes::last_updated.eq(ts),
            ))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "update_route_link_id",
                msg: e.to_string(),
            })?;
        if n == 0 {
            return Err(Error::RouteNotFound {
                id: route_id.to_string(),
            });
        }
        Ok(())
    }

    // ── Links ──────────────────────────────────────────────────────────────

    async fn add_link(&self, mut link: Link) -> Result<Link> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
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
                links::conn_config_data.eq(DbClientConfig::from(link.conn_config_data.clone())),
                links::status.eq(link.status),
                links::status_msg.eq(&link.status_msg),
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
            links::conn_config_data.eq(DbClientConfig::from(link.conn_config_data.clone())),
            links::status.eq(link.status),
            links::status_msg.eq(&link.status_msg),
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
    ) -> Result<Option<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_link pool",
            msg: e.to_string(),
        })?;
        links::table
            .filter(links::link_id.eq(link_id))
            .filter(links::status.ne(LinkStatus::Deleted))
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
            .map_err(|e| Error::DbError {
                context: "get_link",
                msg: e.to_string(),
            })
    }

    async fn find_link_between_nodes(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Option<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "find_link_between_nodes pool",
            msg: e.to_string(),
        })?;
        links::table
            .filter(links::status.ne(LinkStatus::Deleted))
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
            .map_err(|e| Error::DbError {
                context: "find_link_between_nodes",
                msg: e.to_string(),
            })
    }

    async fn find_link_between_groups(&self, group_a: &str, group_b: &str) -> Result<Option<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "find_link_between_groups pool",
            msg: e.to_string(),
        })?;
        links::table
            .filter(links::status.ne(LinkStatus::Deleted))
            .filter(
                links::source_group
                    .eq(group_a)
                    .and(links::dest_group.eq(group_b))
                    .or(links::source_group
                        .eq(group_b)
                        .and(links::dest_group.eq(group_a))),
            )
            .order(links::last_updated.desc())
            .first::<Link>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "find_link_between_groups",
                msg: e.to_string(),
            })
    }

    async fn find_or_create_link(&self, mut link: Link) -> Result<(Link, bool)> {
        if link.link_id.is_empty()
            || link.source_node_id.is_empty()
            || link.dest_endpoint.is_empty()
        {
            return Err(Error::LinkMissingFields);
        }
        link.last_updated = SystemTime::now();
        let ts = DbTimestamp::from(link.last_updated);
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "find_or_create_link pool",
            msg: e.to_string(),
        })?;

        // Use BEGIN IMMEDIATE to acquire a write lock before the SELECT,
        // preventing TOCTOU races between multiple CP instances.
        conn.batch_execute("BEGIN IMMEDIATE")
            .await
            .map_err(|e| Error::DbError {
                context: "find_or_create_link begin",
                msg: e.to_string(),
            })?;

        let result = async {
            // Uniqueness depends on whether the link has a known destination node.
            // Unclaimed (dest_node_id empty): unique by (source_node_id, dest_group)
            // Claimed (dest_node_id set): unique by node pair (bidirectional)
            let existing = if link.dest_node_id.is_empty() {
                links::table
                    .filter(links::status.ne(LinkStatus::Deleted))
                    .filter(links::source_node_id.eq(&link.source_node_id))
                    .filter(links::dest_group.eq(&link.dest_group))
                    .order(links::last_updated.desc())
                    .first::<Link>(&mut conn)
                    .await
                    .optional()
                    .map_err(|e| Error::DbError {
                        context: "find_or_create_link select",
                        msg: e.to_string(),
                    })?
            } else {
                links::table
                    .filter(links::status.ne(LinkStatus::Deleted))
                    .filter(
                        links::source_node_id
                            .eq(&link.source_node_id)
                            .and(links::dest_node_id.eq(&link.dest_node_id))
                            .or(links::source_node_id
                                .eq(&link.dest_node_id)
                                .and(links::dest_node_id.eq(&link.source_node_id))),
                    )
                    .order(links::last_updated.desc())
                    .first::<Link>(&mut conn)
                    .await
                    .optional()
                    .map_err(|e| Error::DbError {
                        context: "find_or_create_link select",
                        msg: e.to_string(),
                    })?
            };

            if let Some(existing) = existing {
                return Ok((existing, false));
            }

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
                    links::conn_config_data.eq(DbClientConfig::from(link.conn_config_data.clone())),
                    links::status.eq(link.status),
                    links::status_msg.eq(&link.status_msg),
                    links::last_updated.eq(ts),
                ))
                .execute(&mut conn)
                .await
                .map_err(|e| Error::DbError {
                    context: "find_or_create_link insert",
                    msg: e.to_string(),
                })?;

            Ok((link, true))
        }
        .await;

        let commit_or_rollback = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        conn.batch_execute(commit_or_rollback)
            .await
            .map_err(|e| Error::DbError {
                context: "find_or_create_link commit/rollback",
                msg: e.to_string(),
            })?;

        result
    }

    async fn get_link_for_source_and_endpoint(
        &self,
        source_node_id: &str,
        dest_endpoint: &str,
    ) -> Result<Option<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_link_for_source_and_endpoint pool",
            msg: e.to_string(),
        })?;
        links::table
            .filter(links::source_node_id.eq(source_node_id))
            .filter(links::dest_endpoint.eq(dest_endpoint))
            .filter(links::status.ne(LinkStatus::Deleted))
            .order(links::last_updated.desc())
            .first::<Link>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_link_for_source_and_endpoint",
                msg: e.to_string(),
            })
    }

    async fn get_links_for_node(&self, node_id: &str) -> Result<Vec<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_links_for_node pool",
            msg: e.to_string(),
        })?;
        links::table
            .filter(
                links::source_node_id
                    .eq(node_id)
                    .or(links::dest_node_id.eq(node_id)),
            )
            .load::<Link>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_links_for_node",
                msg: e.to_string(),
            })
    }

    async fn filter_links_by_src_dest(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<Vec<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "filter_links_by_src_dest pool",
            msg: e.to_string(),
        })?;
        let order = (
            links::source_node_id.asc(),
            links::dest_node_id.asc(),
            links::link_id.asc(),
        );
        let result = match (source_node_id.is_empty(), dest_node_id.is_empty()) {
            (true, true) => links::table.order(order).load::<Link>(&mut conn).await,
            (false, true) => {
                links::table
                    .filter(links::source_node_id.eq(source_node_id))
                    .order(order)
                    .load::<Link>(&mut conn)
                    .await
            }
            (true, false) => {
                links::table
                    .filter(links::dest_node_id.eq(dest_node_id))
                    .order(order)
                    .load::<Link>(&mut conn)
                    .await
            }
            (false, false) => {
                links::table
                    .filter(links::source_node_id.eq(source_node_id))
                    .filter(links::dest_node_id.eq(dest_node_id))
                    .order(order)
                    .load::<Link>(&mut conn)
                    .await
            }
        };
        result.map_err(|e| Error::DbError {
            context: "filter_links_by_src_dest",
            msg: e.to_string(),
        })
    }

    async fn list_all_links(&self) -> Result<Vec<Link>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "list_all_links pool",
            msg: e.to_string(),
        })?;
        links::table
            .load::<Link>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "list_all_links",
                msg: e.to_string(),
            })
    }

    async fn claim_link(
        &self,
        link_id: &str,
        dest_group: &str,
        claimant_node_id: &str,
    ) -> Result<Option<Link>> {
        let now = SystemTime::now();
        let ts = DbTimestamp::from(now);
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "claim_link pool",
            msg: e.to_string(),
        })?;

        // Atomic CAS: only update if dest_node_id is empty and dest_group matches.
        let updated = diesel::update(
            links::table
                .filter(links::link_id.eq(link_id))
                .filter(links::dest_node_id.eq(""))
                .filter(links::dest_group.eq(dest_group))
                .filter(links::status.ne(LinkStatus::Deleted)),
        )
        .set((
            links::dest_node_id.eq(claimant_node_id),
            links::status.eq(LinkStatus::Applied),
            links::last_updated.eq(ts),
        ))
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "claim_link update",
            msg: e.to_string(),
        })?;

        if updated == 0 {
            return Ok(None);
        }

        // Fetch the updated link.
        let link = links::table
            .filter(links::link_id.eq(link_id))
            .filter(links::dest_node_id.eq(claimant_node_id))
            .first::<Link>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "claim_link fetch",
                msg: e.to_string(),
            })?;

        Ok(link)
    }

    // ── Topology (API-managed mode) ───────────────────────────────────────

    async fn create_segment(&self, name: &str) -> Result<TopologySegment> {
        let segment = TopologySegment {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: SystemTime::now(),
        };
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "create_segment pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(topology_segments::table)
            .values(segment.clone())
            .execute(&mut conn)
            .await
            .map_err(|e| {
                if matches!(
                    e,
                    diesel::result::Error::DatabaseError(
                        diesel::result::DatabaseErrorKind::UniqueViolation,
                        _
                    )
                ) {
                    Error::AlreadyExists {
                        entity: "segment",
                        name: name.to_string(),
                    }
                } else {
                    Error::DbError {
                        context: "create_segment",
                        msg: e.to_string(),
                    }
                }
            })?;
        Ok(segment)
    }

    async fn delete_segment(&self, id: &str) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_segment pool",
            msg: e.to_string(),
        })?;
        diesel::delete(topology_segments::table.find(id))
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "delete_segment",
                msg: e.to_string(),
            })?;
        Ok(())
    }

    async fn get_segment_by_name(&self, name: &str) -> Result<Option<TopologySegment>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_segment_by_name pool",
            msg: e.to_string(),
        })?;
        topology_segments::table
            .filter(topology_segments::name.eq(name))
            .first::<TopologySegment>(&mut conn)
            .await
            .optional()
            .map_err(|e| Error::DbError {
                context: "get_segment_by_name",
                msg: e.to_string(),
            })
    }

    async fn list_topology_segments(&self) -> Result<Vec<TopologySegment>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "list_topology_segments pool",
            msg: e.to_string(),
        })?;
        topology_segments::table
            .load::<TopologySegment>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "list_topology_segments",
                msg: e.to_string(),
            })
    }

    async fn add_group_to_segment(&self, segment_id: &str, group_name: &str) -> Result<()> {
        let group = TopologySegmentGroup {
            segment_id: segment_id.to_string(),
            group_name: group_name.to_string(),
        };
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "add_group_to_segment pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(topology_segment_groups::table)
            .values(group)
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "add_group_to_segment",
                msg: e.to_string(),
            })?;
        Ok(())
    }

    async fn remove_group_from_segment(&self, segment_id: &str, group_name: &str) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "remove_group_from_segment pool",
            msg: e.to_string(),
        })?;
        diesel::delete(
            topology_segment_groups::table
                .filter(topology_segment_groups::segment_id.eq(segment_id))
                .filter(topology_segment_groups::group_name.eq(group_name)),
        )
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "remove_group_from_segment",
            msg: e.to_string(),
        })?;
        Ok(())
    }

    async fn get_groups_for_segment(&self, segment_id: &str) -> Result<Vec<String>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_groups_for_segment pool",
            msg: e.to_string(),
        })?;
        topology_segment_groups::table
            .filter(topology_segment_groups::segment_id.eq(segment_id))
            .select(topology_segment_groups::group_name)
            .load::<String>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_groups_for_segment",
                msg: e.to_string(),
            })
    }

    async fn add_link_to_segment(
        &self,
        segment_id: &str,
        source_group: &str,
        dest_group: &str,
    ) -> Result<()> {
        let link = TopologySegmentLink {
            segment_id: segment_id.to_string(),
            source_group: source_group.to_string(),
            dest_group: dest_group.to_string(),
        };
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "add_link_to_segment pool",
            msg: e.to_string(),
        })?;
        diesel::insert_into(topology_segment_links::table)
            .values(link)
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "add_link_to_segment",
                msg: e.to_string(),
            })?;
        Ok(())
    }

    async fn delete_link_from_segment(
        &self,
        segment_id: &str,
        source_group: &str,
        dest_group: &str,
    ) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "delete_link_from_segment pool",
            msg: e.to_string(),
        })?;
        let seg_id = segment_id.to_string();
        let src = source_group.to_string();
        let dst = dest_group.to_string();
        diesel::delete(
            topology_segment_links::table
                .filter(topology_segment_links::segment_id.eq(&seg_id))
                .filter(
                    topology_segment_links::source_group
                        .eq(&src)
                        .and(topology_segment_links::dest_group.eq(&dst))
                        .or(topology_segment_links::source_group
                            .eq(&dst)
                            .and(topology_segment_links::dest_group.eq(&src))),
                ),
        )
        .execute(&mut conn)
        .await
        .map_err(|e| Error::DbError {
            context: "delete_link_from_segment",
            msg: e.to_string(),
        })?;
        Ok(())
    }

    async fn get_links_for_segment(&self, segment_id: &str) -> Result<Vec<(String, String)>> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "get_links_for_segment pool",
            msg: e.to_string(),
        })?;
        topology_segment_links::table
            .filter(topology_segment_links::segment_id.eq(segment_id))
            .select((
                topology_segment_links::source_group,
                topology_segment_links::dest_group,
            ))
            .load::<(String, String)>(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "get_links_for_segment",
                msg: e.to_string(),
            })
    }

    async fn clear_topology(&self) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| Error::DbError {
            context: "clear_topology pool",
            msg: e.to_string(),
        })?;
        diesel::delete(topology_segment_links::table)
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "clear_topology links",
                msg: e.to_string(),
            })?;
        diesel::delete(topology_segment_groups::table)
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "clear_topology groups",
                msg: e.to_string(),
            })?;
        diesel::delete(topology_segments::table)
            .execute(&mut conn)
            .await
            .map_err(|e| Error::DbError {
                context: "clear_topology segments",
                msg: e.to_string(),
            })?;
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ConnectionDetails, DataAccess};
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
                spire_mtls: None,
            }],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        }
    }

    fn make_route(src: &str, dst: &str, link: &str) -> Route {
        Route {
            id: String::new(),
            source_node_id: src.to_string(),
            source_group: String::new(),
            dest_node_id: dst.to_string(),
            dest_group: String::new(),
            link_id: Some(link.to_string()),
            component0: "org".to_string(),
            component1: "ns".to_string(),
            component2: "svc".to_string(),
            component_id: Some("00000000-0000-0000-0000-000000000001".to_string()),
            status: RouteStatus::Pending,
            status_msg: String::new(),
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        }
    }

    fn make_link(src: &str, dst: &str, ep: &str, lid: &str) -> Link {
        Link {
            link_id: lid.to_string(),
            source_node_id: src.to_string(),
            source_group: String::new(),
            dest_node_id: dst.to_string(),
            dest_group: String::new(),
            dest_endpoint: ep.to_string(),
            conn_config_data: slim_config::grpc::client::ClientConfig::default()
                .with_connection_type(slim_config::conn_type::ConnType::Remote),
            status: LinkStatus::Pending,
            status_msg: String::new(),
            created_at: SystemTime::now(),
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
        let got = db.get_node("n1").await.unwrap().unwrap();
        assert_eq!(got.id, "n1");
        assert_eq!(got.group_name.as_deref(), Some("grp"));
    }

    #[tokio::test]
    async fn list_nodes_returns_all() {
        let (_f, db) = tmp_db().await;
        db.save_node(make_node("n1", None)).await.unwrap();
        db.save_node(make_node("n2", None)).await.unwrap();
        assert_eq!(db.list_nodes().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn delete_node() {
        let (_f, db) = tmp_db().await;
        db.save_node(make_node("n1", None)).await.unwrap();
        db.delete_node("n1").await.unwrap();
        assert!(db.get_node("n1").await.unwrap().is_none());
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
        assert!(!r.id.is_empty());
        let got = db.get_route_by_id(&r.id).await.unwrap().unwrap();
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
        let routes = db.get_routes_for_node("src").await.unwrap();
        assert_eq!(routes.len(), 1);
    }

    #[tokio::test]
    async fn get_routes_for_dest_node_id_excludes_wildcard() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let mut wc = make_route(ALL_NODES_ID, "dst", "lnk2");
        wc.component1 = "ns2".to_string();
        db.add_route(wc).await.unwrap();
        let routes = db.get_routes_for_dest_node_id("dst").await.unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source_node_id, "src");
    }

    #[tokio::test]
    async fn mark_route_applied() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_applied(&r.id).await.unwrap();
        assert_eq!(
            db.get_route_by_id(&r.id).await.unwrap().unwrap().status,
            RouteStatus::Applied
        );
    }

    #[tokio::test]
    async fn mark_route_failed() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_failed(&r.id, "oops").await.unwrap();
        let got = db.get_route_by_id(&r.id).await.unwrap().unwrap();
        assert_eq!(got.status, RouteStatus::Failed);
        assert_eq!(got.status_msg, "oops");
    }

    #[tokio::test]
    async fn mark_route_deleted_and_delete() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        db.mark_route_deleted(&r.id).await.unwrap();
        assert_eq!(
            db.get_route_by_id(&r.id).await.unwrap().unwrap().status,
            RouteStatus::Deleted
        );
        db.delete_route(&r.id).await.unwrap();
        assert!(db.get_route_by_id(&r.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_route_not_found() {
        let (_f, db) = tmp_db().await;
        assert!(db.delete_route("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn get_route_for_src_dest_name() {
        let (_f, db) = tmp_db().await;
        let r = db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        let name = RouteName {
            component0: "org",
            component1: "ns",
            component2: "svc",
            component_id: Some("00000000-0000-0000-0000-000000000001"),
        };
        let found = db
            .get_route_for_src_dest_name("src", &name, "dst", Some("lnk"))
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, r.id);
    }

    #[tokio::test]
    async fn filter_routes_by_src_dest() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "lnk")).await.unwrap();
        assert_eq!(db.filter_routes_by_src_dest("", "").await.unwrap().len(), 1);
        assert_eq!(
            db.filter_routes_by_src_dest("src", "").await.unwrap().len(),
            1
        );
        assert_eq!(
            db.filter_routes_by_src_dest("", "dst").await.unwrap().len(),
            1
        );
        assert_eq!(
            db.filter_routes_by_src_dest("src", "dst")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            db.filter_routes_by_src_dest("other", "")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn get_destination_node_id_for_name() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route(ALL_NODES_ID, "dst_node", "lnk"))
            .await
            .unwrap();
        let result = db
            .get_destination_node_id_for_name(
                "org",
                "ns",
                "svc",
                Some("00000000-0000-0000-0000-000000000001"),
            )
            .await
            .unwrap();
        assert_eq!(result.as_deref(), Some("dst_node"));
    }

    #[tokio::test]
    async fn get_routes_by_link_id() {
        let (_f, db) = tmp_db().await;
        db.add_route(make_route("src", "dst", "link-abc"))
            .await
            .unwrap();
        assert_eq!(db.get_routes_by_link_id("link-abc").await.unwrap().len(), 1);
        assert!(
            db.get_routes_by_link_id("link-xyz")
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── Link CRUD ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_and_get_link() {
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(db.get_link("lid", "src", "dst").await.unwrap().is_some());
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
            db.get_link("lid", "src", "dst")
                .await
                .unwrap()
                .unwrap()
                .status,
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
        assert!(db.get_link("lid", "src", "dst").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_link_between_nodes() {
        let (_f, db) = tmp_db().await;
        db.add_link(make_link("src", "dst", "ep:8080", "lid"))
            .await
            .unwrap();
        assert!(
            db.find_link_between_nodes("src", "dst")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.find_link_between_nodes("dst", "src")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.find_link_between_nodes("a", "b")
                .await
                .unwrap()
                .is_none()
        );
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
                .unwrap()
                .is_some()
        );
        assert!(
            db.get_link_for_source_and_endpoint("src", "other")
                .await
                .unwrap()
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
        assert_eq!(db.get_links_for_node("src").await.unwrap().len(), 2);
    }

    // ── Topology tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn topology_create_and_list_segments() {
        let (_f, db) = tmp_db().await;
        let seg = db.create_segment("seg-1").await.unwrap();
        assert_eq!(seg.name, "seg-1");
        assert!(!seg.id.is_empty());

        let segs = db.list_topology_segments().await.unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].name, "seg-1");
    }

    #[tokio::test]
    async fn topology_get_segment_by_name() {
        let (_f, db) = tmp_db().await;
        db.create_segment("alpha").await.unwrap();

        let found = db.get_segment_by_name("alpha").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "alpha");

        let missing = db.get_segment_by_name("nope").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn topology_delete_segment_cascades() {
        let (_f, db) = tmp_db().await;
        let seg = db.create_segment("seg-1").await.unwrap();
        db.add_group_to_segment(&seg.id, "group-a").await.unwrap();
        db.add_link_to_segment(&seg.id, "group-a", "group-b")
            .await
            .unwrap();

        db.delete_segment(&seg.id).await.unwrap();

        assert!(db.list_topology_segments().await.unwrap().is_empty());
        assert!(db.get_groups_for_segment(&seg.id).await.unwrap().is_empty());
        assert!(db.get_links_for_segment(&seg.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn topology_add_and_remove_group() {
        let (_f, db) = tmp_db().await;
        let seg = db.create_segment("seg-1").await.unwrap();

        db.add_group_to_segment(&seg.id, "group-a").await.unwrap();
        db.add_group_to_segment(&seg.id, "group-b").await.unwrap();

        let groups = db.get_groups_for_segment(&seg.id).await.unwrap();
        assert_eq!(groups.len(), 2);
        assert!(groups.contains(&"group-a".to_string()));
        assert!(groups.contains(&"group-b".to_string()));

        db.remove_group_from_segment(&seg.id, "group-a")
            .await
            .unwrap();
        let groups = db.get_groups_for_segment(&seg.id).await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], "group-b");
    }

    #[tokio::test]
    async fn topology_add_and_delete_link() {
        let (_f, db) = tmp_db().await;
        let seg = db.create_segment("seg-1").await.unwrap();

        db.add_link_to_segment(&seg.id, "group-a", "group-b")
            .await
            .unwrap();
        db.add_link_to_segment(&seg.id, "group-b", "group-c")
            .await
            .unwrap();

        let links = db.get_links_for_segment(&seg.id).await.unwrap();
        assert_eq!(links.len(), 2);

        db.delete_link_from_segment(&seg.id, "group-a", "group-b")
            .await
            .unwrap();
        let links = db.get_links_for_segment(&seg.id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], ("group-b".to_string(), "group-c".to_string()));
    }

    #[tokio::test]
    async fn topology_clear_wipes_all() {
        let (_f, db) = tmp_db().await;
        let seg1 = db.create_segment("seg-1").await.unwrap();
        let seg2 = db.create_segment("seg-2").await.unwrap();
        db.add_group_to_segment(&seg1.id, "group-a").await.unwrap();
        db.add_link_to_segment(&seg2.id, "group-x", "group-y")
            .await
            .unwrap();

        db.clear_topology().await.unwrap();

        assert!(db.list_topology_segments().await.unwrap().is_empty());
        assert!(
            db.get_groups_for_segment(&seg1.id)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(db.get_links_for_segment(&seg2.id).await.unwrap().is_empty());
    }
}

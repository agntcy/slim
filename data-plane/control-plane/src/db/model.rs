// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, SystemTime};

use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::prelude::*;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::{BigInt, Integer, Text};
use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use twox_hash::XxHash64;

use super::schema::{channels, links, nodes, routes};

/// Wildcard node ID — matches all nodes.
pub const ALL_NODES_ID: &str = "*";

// ─── Diesel helper types ───────────────────────────────────────────────────────
//
// These newtypes bridge domain types that don't natively map to SQL column
// types (SystemTime→BigInt, Vec<T>→Text/JSON). They are only needed as
// intermediate values for Diesel serialisation/deserialisation; application
// code works directly with the domain types on the struct fields.

/// `SystemTime` ↔ `BigInt` (Unix seconds).
#[derive(Debug, Clone, Copy, AsExpression, FromSqlRow)]
#[diesel(sql_type = BigInt)]
pub struct DbTimestamp(pub i64);

impl From<DbTimestamp> for SystemTime {
    fn from(t: DbTimestamp) -> Self {
        SystemTime::UNIX_EPOCH + Duration::from_secs(t.0.max(0) as u64)
    }
}

impl From<SystemTime> for DbTimestamp {
    fn from(t: SystemTime) -> Self {
        let secs = t
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        DbTimestamp(secs)
    }
}

impl<DB: Backend> ToSql<BigInt, DB> for DbTimestamp
where
    i64: ToSql<BigInt, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> serialize::Result {
        ToSql::<BigInt, DB>::to_sql(&self.0, out)
    }
}

impl<DB: Backend> FromSql<BigInt, DB> for DbTimestamp
where
    i64: FromSql<BigInt, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        Ok(DbTimestamp(i64::from_sql(bytes)?))
    }
}

/// `Vec<ConnectionDetails>` ↔ `Text` (JSON).
#[derive(Debug, AsExpression, FromSqlRow)]
#[diesel(sql_type = Text)]
pub struct ConnDetailsJson(pub Vec<ConnectionDetails>);

impl From<ConnDetailsJson> for Vec<ConnectionDetails> {
    fn from(j: ConnDetailsJson) -> Self {
        j.0
    }
}

impl From<Vec<ConnectionDetails>> for ConnDetailsJson {
    fn from(v: Vec<ConnectionDetails>) -> Self {
        ConnDetailsJson(v)
    }
}

impl<DB: Backend> FromSql<Text, DB> for ConnDetailsJson
where
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        let s = String::from_sql(bytes)?;
        Ok(ConnDetailsJson(
            serde_json::from_str(&s).unwrap_or_default(),
        ))
    }
}

/// `Vec<String>` ↔ `Text` (JSON array).
#[derive(Debug, AsExpression, FromSqlRow)]
#[diesel(sql_type = Text)]
pub struct JsonStrings(pub Vec<String>);

impl From<JsonStrings> for Vec<String> {
    fn from(j: JsonStrings) -> Self {
        j.0
    }
}

impl From<Vec<String>> for JsonStrings {
    fn from(v: Vec<String>) -> Self {
        JsonStrings(v)
    }
}

impl<DB: Backend> FromSql<Text, DB> for JsonStrings
where
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        let s = String::from_sql(bytes)?;
        Ok(JsonStrings(serde_json::from_str(&s).unwrap_or_default()))
    }
}

// ─── Node ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = nodes)]
pub struct Node {
    pub id: String,
    pub group_name: Option<String>,
    #[diesel(deserialize_as = ConnDetailsJson, serialize_as = ConnDetailsJson)]
    pub conn_details: Vec<ConnectionDetails>,
    #[diesel(deserialize_as = DbTimestamp, serialize_as = DbTimestamp)]
    pub last_updated: SystemTime,
}

/// Per-node connection detail.
/// `client_config` is a raw JSON value that mirrors the data-plane's
/// `ClientConfig` structure so it can be sent verbatim as connection
/// config data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDetails {
    pub endpoint: String,
    pub external_endpoint: Option<String>,
    pub trust_domain: Option<String>,
    pub mtls_required: bool,
    pub client_config: serde_json::Value,
}

impl std::fmt::Display for ConnectionDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = vec![format!("endpoint: {}", self.endpoint)];
        if self.mtls_required {
            parts.push("mtls".to_string());
        }
        if let Some(ref ee) = self.external_endpoint
            && !ee.is_empty()
        {
            parts.push(format!("externalEndpoint: {ee}"));
        }
        if let Some(ref td) = self.trust_domain
            && !td.is_empty()
        {
            parts.push(format!("trustDomain: {td}"));
        }
        write!(f, "{}", parts.join(", "))
    }
}

/// Returns true if two `ConnectionDetails` slices differ in any meaningful way.
pub fn has_connection_details_changed(
    existing: &[ConnectionDetails],
    new: &[ConnectionDetails],
) -> bool {
    if existing.len() != new.len() {
        return true;
    }
    let existing_map: std::collections::HashMap<&str, &ConnectionDetails> = existing
        .iter()
        .map(|cd| (cd.endpoint.as_str(), cd))
        .collect();
    let new_map: std::collections::HashMap<&str, &ConnectionDetails> =
        new.iter().map(|cd| (cd.endpoint.as_str(), cd)).collect();

    for (key, ecd) in &existing_map {
        match new_map.get(key) {
            None => return true,
            Some(ncd) => {
                if ecd.mtls_required != ncd.mtls_required
                    || ecd.external_endpoint != ncd.external_endpoint
                    || ecd.trust_domain != ncd.trust_domain
                    || ecd.client_config != ncd.client_config
                {
                    return true;
                }
            }
        }
    }
    for key in new_map.keys() {
        if !existing_map.contains_key(key) {
            return true;
        }
    }
    false
}

/// Groups the four subscription-name components used in route lookups.
pub struct SubscriptionName<'a> {
    pub component0: &'a str,
    pub component1: &'a str,
    pub component2: &'a str,
    pub component_id: Option<i64>,
}

impl<'a> From<&'a Route> for SubscriptionName<'a> {
    fn from(r: &'a Route) -> Self {
        SubscriptionName {
            component0: &r.component0,
            component1: &r.component1,
            component2: &r.component2,
            component_id: r.component_id,
        }
    }
}

// ─── Route ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum RouteStatus {
    Applied,
    Failed,
    Pending,
}

impl<DB: Backend> FromSql<Integer, DB> for RouteStatus
where
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        Ok(match i32::from_sql(bytes)? {
            1 => RouteStatus::Applied,
            2 => RouteStatus::Failed,
            _ => RouteStatus::Pending,
        })
    }
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = routes)]
pub struct Route {
    pub id: i64,
    pub source_node_id: String,
    pub dest_node_id: String,
    pub link_id: String,
    pub component0: String,
    pub component1: String,
    pub component2: String,
    pub component_id: Option<i64>,
    pub status: RouteStatus,
    pub status_msg: String,
    pub deleted: bool,
    #[diesel(deserialize_as = DbTimestamp, serialize_as = DbTimestamp)]
    pub last_updated: SystemTime,
}

impl Route {
    pub fn unique_id(
        source_node_id: &str,
        component0: &str,
        component1: &str,
        component2: &str,
        component_id: Option<i64>,
        dest_node_id: &str,
        link_id: &str,
    ) -> i64 {
        let sep = '\x00';
        let comp_id = component_id.map(|v| v.to_string()).unwrap_or_default();
        let combined = format!(
            "{source_node_id}{sep}{component0}{sep}{component1}{sep}{component2}{sep}{comp_id}{sep}{dest_node_id}{sep}{link_id}"
        );
        let mut hasher = XxHash64::with_seed(0);
        hasher.write(combined.as_bytes());
        // Keep high bit clear to avoid potential signed-integer issues.
        (hasher.finish() & 0x7FFF_FFFF_FFFF_FFFF) as i64
    }

    pub fn compute_id(&self) -> i64 {
        Self::unique_id(
            &self.source_node_id,
            &self.component0,
            &self.component1,
            &self.component2,
            self.component_id,
            &self.dest_node_id,
            &self.link_id,
        )
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}/{}/{}/{:?}->node={} link={}",
            self.source_node_id,
            self.component0,
            self.component1,
            self.component2,
            self.component_id,
            self.dest_node_id,
            self.link_id,
        )
    }
}

// ─── Link ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum LinkStatus {
    Pending,
    Applied,
    Failed,
}

impl<DB: Backend> FromSql<Integer, DB> for LinkStatus
where
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        Ok(match i32::from_sql(bytes)? {
            1 => LinkStatus::Applied,
            2 => LinkStatus::Failed,
            _ => LinkStatus::Pending,
        })
    }
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = links, primary_key(link_id, source_node_id, dest_node_id, dest_endpoint))]
pub struct Link {
    pub link_id: String,
    pub source_node_id: String,
    pub dest_node_id: String,
    pub dest_endpoint: String,
    pub conn_config_data: String,
    pub status: LinkStatus,
    pub status_msg: String,
    pub deleted: bool,
    #[diesel(deserialize_as = DbTimestamp, serialize_as = DbTimestamp)]
    pub last_updated: SystemTime,
}

impl Link {
    pub fn storage_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.source_node_id, self.dest_node_id, self.dest_endpoint, self.link_id
        )
    }
}

impl std::fmt::Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}->{}({}) link={} [{:?}]",
            self.source_node_id, self.dest_node_id, self.dest_endpoint, self.link_id, self.status,
        )
    }
}

// ─── Channel ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = channels)]
pub struct Channel {
    pub id: String,
    #[diesel(deserialize_as = JsonStrings, serialize_as = JsonStrings)]
    pub moderators: Vec<String>,
    #[diesel(deserialize_as = JsonStrings, serialize_as = JsonStrings)]
    pub participants: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cd(endpoint: &str, external: Option<&str>, mtls: bool) -> ConnectionDetails {
        ConnectionDetails {
            endpoint: endpoint.to_string(),
            external_endpoint: external.map(|s| s.to_string()),
            trust_domain: None,
            mtls_required: mtls,
            client_config: serde_json::Value::Null,
        }
    }

    fn make_route(src: &str, dest: &str, link: &str) -> Route {
        Route {
            id: 0,
            source_node_id: src.to_string(),
            dest_node_id: dest.to_string(),
            link_id: link.to_string(),
            component0: "org".to_string(),
            component1: "ns".to_string(),
            component2: "type".to_string(),
            component_id: Some(1),
            status: RouteStatus::Pending,
            status_msg: String::new(),
            deleted: false,
            last_updated: std::time::SystemTime::now(),
        }
    }

    // ── Route::unique_id / compute_id ──────────────────────────────────────

    #[test]
    fn unique_id_is_deterministic() {
        let a = Route::unique_id("src", "c0", "c1", "c2", Some(7), "dst", "link1");
        let b = Route::unique_id("src", "c0", "c1", "c2", Some(7), "dst", "link1");
        assert_eq!(a, b);
    }

    #[test]
    fn unique_id_differs_for_different_inputs() {
        let a = Route::unique_id("src", "c0", "c1", "c2", Some(1), "dst", "link1");
        let b = Route::unique_id("src", "c0", "c1", "c2", Some(2), "dst", "link1");
        assert_ne!(a, b);

        let c = Route::unique_id("src", "c0", "c1", "c2", None, "dst", "link1");
        assert_ne!(a, c);

        let d = Route::unique_id("OTHER", "c0", "c1", "c2", Some(1), "dst", "link1");
        assert_ne!(a, d);
    }

    #[test]
    fn unique_id_high_bit_clear() {
        for _ in 0..50 {
            let id = Route::unique_id("s", "a", "b", "c", None, "d", "e");
            assert!(id >= 0, "high bit must be clear (id must be non-negative)");
        }
    }

    #[test]
    fn compute_id_matches_unique_id() {
        let r = make_route("src", "dst", "lnk");
        let expected = Route::unique_id(
            &r.source_node_id,
            &r.component0,
            &r.component1,
            &r.component2,
            r.component_id,
            &r.dest_node_id,
            &r.link_id,
        );
        assert_eq!(r.compute_id(), expected);
    }

    // ── Route Display ──────────────────────────────────────────────────────

    #[test]
    fn route_display() {
        let r = make_route("src", "dst", "lnk");
        let s = format!("{r}");
        assert!(s.contains("src"));
        assert!(s.contains("dst"));
        assert!(s.contains("lnk"));
        assert!(s.contains("org/ns/type"));
    }

    // ── Link::storage_key / Display ────────────────────────────────────────

    #[test]
    fn link_storage_key() {
        let link = Link {
            link_id: "lid".to_string(),
            source_node_id: "src".to_string(),
            dest_node_id: "dst".to_string(),
            dest_endpoint: "ep:9000".to_string(),
            conn_config_data: String::new(),
            status: LinkStatus::Pending,
            status_msg: String::new(),
            deleted: false,
            last_updated: std::time::SystemTime::now(),
        };
        assert_eq!(link.storage_key(), "src|dst|ep:9000|lid");
    }

    #[test]
    fn link_display() {
        let link = Link {
            link_id: "lid".to_string(),
            source_node_id: "src".to_string(),
            dest_node_id: "dst".to_string(),
            dest_endpoint: "ep:9000".to_string(),
            conn_config_data: String::new(),
            status: LinkStatus::Applied,
            status_msg: String::new(),
            deleted: false,
            last_updated: std::time::SystemTime::now(),
        };
        let s = format!("{link}");
        assert!(s.contains("src"));
        assert!(s.contains("dst"));
        assert!(s.contains("lid"));
        assert!(s.contains("Applied"));
    }

    // ── SubscriptionName From<&Route> ──────────────────────────────────────

    #[test]
    fn subscription_name_from_route() {
        let r = make_route("src", "dst", "lnk");
        let sn = SubscriptionName::from(&r);
        assert_eq!(sn.component0, "org");
        assert_eq!(sn.component1, "ns");
        assert_eq!(sn.component2, "type");
        assert_eq!(sn.component_id, Some(1));
    }

    // ── has_connection_details_changed ─────────────────────────────────────

    #[test]
    fn connection_details_unchanged_when_identical() {
        let cd = make_cd("ep:8080", None, false);
        assert!(!has_connection_details_changed(
            std::slice::from_ref(&cd),
            std::slice::from_ref(&cd)
        ));
    }

    #[test]
    fn connection_details_changed_different_length() {
        let cd = make_cd("ep:8080", None, false);
        assert!(has_connection_details_changed(&[], &[cd]));
    }

    #[test]
    fn connection_details_changed_different_mtls() {
        let a = make_cd("ep:8080", None, false);
        let b = make_cd("ep:8080", None, true);
        assert!(has_connection_details_changed(&[a], &[b]));
    }

    #[test]
    fn connection_details_changed_different_external_endpoint() {
        let a = make_cd("ep:8080", Some("ext:9090"), false);
        let b = make_cd("ep:8080", Some("other:9090"), false);
        assert!(has_connection_details_changed(&[a], &[b]));
    }

    #[test]
    fn connection_details_changed_different_trust_domain() {
        let mut a = make_cd("ep:8080", None, false);
        a.trust_domain = Some("domain-a.example".to_string());
        let mut b = make_cd("ep:8080", None, false);
        b.trust_domain = Some("domain-b.example".to_string());
        assert!(has_connection_details_changed(&[a], &[b]));
    }

    #[test]
    fn connection_details_unchanged_same_trust_domain() {
        let mut a = make_cd("ep:8080", None, false);
        a.trust_domain = Some("domain.example".to_string());
        let b = a.clone();
        assert!(!has_connection_details_changed(&[a], &[b]));
    }

    #[test]
    fn connection_details_changed_missing_key_in_new() {
        let a = make_cd("ep:8080", None, false);
        let b = make_cd("other:8080", None, false);
        assert!(has_connection_details_changed(&[a], &[b]));
    }

    // ── ConnectionDetails Display ──────────────────────────────────────────

    #[test]
    fn connection_details_display_basic() {
        let cd = make_cd("ep:8080", None, false);
        let s = format!("{cd}");
        assert!(s.contains("ep:8080"));
        assert!(!s.contains("mtls"));
    }

    #[test]
    fn connection_details_display_with_mtls_and_external() {
        let cd = make_cd("ep:8080", Some("ext:9090"), true);
        let s = format!("{cd}");
        assert!(s.contains("mtls"));
        assert!(s.contains("ext:9090"));
    }

    #[test]
    fn connection_details_display_empty_external_omitted() {
        let cd = make_cd("ep:8080", Some(""), false);
        let s = format!("{cd}");
        assert!(!s.contains("externalEndpoint"));
    }

    #[test]
    fn connection_details_display_trust_domain() {
        let mut cd = make_cd("ep:8080", None, false);
        cd.trust_domain = Some("example.org".to_string());
        let s = format!("{cd}");
        assert!(s.contains("example.org"));
    }
}

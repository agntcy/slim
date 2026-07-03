// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_config::client::ServerConnectionConfig;

use crate::db::model;
use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub(super) struct ReportedConnection {
    pub(super) endpoint: String,
    pub(super) link_id: String,
    pub(super) config_data: ServerConnectionConfig,
}

impl super::RouteService {
    /// Compute the effective endpoint and serialised JSON config data for a
    /// link from `source_node_id` to `dest_node_id`.
    pub async fn get_client_config(
        &self,
        source_node_id: &str,
        dest_node_id: &str,
    ) -> Result<(String, ServerConnectionConfig)> {
        let dest_node =
            self.0
                .db
                .get_node(dest_node_id)
                .await?
                .ok_or_else(|| Error::NodeNotFound {
                    id: dest_node_id.to_string(),
                })?;

        let src_node =
            self.0
                .db
                .get_node(source_node_id)
                .await?
                .ok_or_else(|| Error::NodeNotFound {
                    id: source_node_id.to_string(),
                })?;

        compute_client_config(&src_node, &dest_node)
    }
}

/// Compute the effective endpoint and serialised JSON config data from
/// already-loaded node objects, without hitting the DB.
pub(super) fn compute_client_config(
    src_node: &crate::db::Node,
    dst_node: &crate::db::Node,
) -> Result<(String, ServerConnectionConfig)> {
    if dst_node.conn_details.is_empty() {
        return Err(Error::InvalidInput(format!(
            "no connections for destination node {}",
            dst_node.id
        )));
    }
    let (conn, local_connection) = select_connection(dst_node, src_node);
    generate_config_data(conn, local_connection, dst_node)
}

/// Select the best connection detail from `dst_node` relative to `src_node`.
///
/// # Precondition
/// `dst_node.conn_details` must be non-empty.  The caller
/// (`compute_client_config`) is responsible for enforcing this by
/// returning an error when `conn_details` is empty before calling here.
pub(super) fn select_connection<'a>(
    dst_node: &'a crate::db::Node,
    src_node: &crate::db::Node,
) -> (&'a crate::db::ConnectionDetails, bool) {
    debug_assert!(
        !dst_node.conn_details.is_empty(),
        "select_connection called with empty conn_details for node {}",
        dst_node.id
    );
    let same_group = dst_node.group_name == src_node.group_name;
    if same_group {
        return (&dst_node.conn_details[0], true);
    }
    for conn in &dst_node.conn_details {
        if conn
            .external_endpoint
            .as_deref()
            .map(|e| !e.is_empty())
            .unwrap_or(false)
        {
            return (conn, false);
        }
    }
    (&dst_node.conn_details[0], false)
}

/// Generate the serialised JSON connection config data for a link.
pub(super) fn generate_config_data(
    detail: &crate::db::ConnectionDetails,
    local_connection: bool,
    dest_node: &crate::db::Node,
) -> Result<(String, ServerConnectionConfig)> {
    use slim_config::client::RequiredAuthMethod;

    let endpoint = if local_connection {
        detail.endpoint.clone()
    } else {
        detail
            .external_endpoint
            .clone()
            .filter(|e| !e.is_empty())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no external endpoint for connection {}",
                    detail.endpoint
                ))
            })?
    };

    let (effective_endpoint, tls_required, auth_method) =
        if detail.tls_required && detail.auth_method == model::AuthMethod::Spire {
            let trust_domain = detail
                .spire_trust_domain
                .as_deref()
                .or(dest_node.group_name.as_deref())
                .map(|s| s.to_string());
            (
                format!("https://{endpoint}"),
                true,
                RequiredAuthMethod::Spire { trust_domain },
            )
        } else {
            (
                format!("http://{endpoint}"),
                false,
                RequiredAuthMethod::None,
            )
        };

    let server_config = ServerConnectionConfig {
        endpoint: effective_endpoint.clone(),
        tls_required,
        auth_method,
        backoff: Some(2000),
        timeout: None,
    };

    Ok((effective_endpoint, server_config))
}

pub(super) fn find_reported_connection<'a>(
    reported: &'a [ReportedConnection],
    dest_endpoint: &str,
) -> Option<&'a ReportedConnection> {
    reported
        .iter()
        .find(|rc| endpoint_matches(&rc.endpoint, dest_endpoint))
}

pub(super) fn find_reported_connection_for_dest<'a>(
    reported: &'a [ReportedConnection],
    dst_node: &crate::db::Node,
) -> Option<&'a ReportedConnection> {
    for rc in reported {
        for detail in &dst_node.conn_details {
            if endpoint_matches(&rc.endpoint, &detail.endpoint) {
                return Some(rc);
            }
            if let Some(ext) = &detail.external_endpoint
                && !ext.is_empty()
                && endpoint_matches(&rc.endpoint, ext)
            {
                return Some(rc);
            }
        }
    }
    None
}

pub(crate) fn is_connection_not_found(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("connection not found")
        || lower.contains("no such connection")
        || (lower.contains("connection") && lower.contains("not found"))
}

pub(super) fn endpoint_matches(reported: &str, node_endpoint: &str) -> bool {
    let normalize = |ep: &str| ep.trim().trim_end_matches('/').to_lowercase();
    let r = normalize(reported);
    let n = normalize(node_endpoint);
    if r == n {
        return true;
    }
    if r == format!("https://{n}") || r == format!("http://{n}") {
        return true;
    }
    if n == format!("https://{r}") || n == format!("http://{r}") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::super::test_utils::{make_conn_details, make_node, make_route_service};
    use super::*;
    use crate::db::inmemory::InMemoryDb;
    use crate::db::model::AuthMethod;
    use slim_config::client::RequiredAuthMethod;

    #[test]
    fn select_connection_same_group_returns_first() {
        let dst = make_node(
            "dst",
            Some("grp"),
            vec![make_conn_details("dst:8080", Some("ext:9090"))],
        );
        let src = make_node("src", Some("grp"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    #[test]
    fn select_connection_different_group_prefers_external() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![
                make_conn_details("dst:8080", None),
                make_conn_details("dst:8081", Some("ext:9090")),
            ],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.external_endpoint.as_deref(), Some("ext:9090"));
    }

    #[test]
    fn select_connection_different_group_no_external_falls_back() {
        let dst = make_node(
            "dst",
            Some("grp1"),
            vec![make_conn_details("dst:8080", None)],
        );
        let src = make_node("src", Some("grp2"), vec![]);
        let (conn, local) = select_connection(&dst, &src);
        assert!(!local);
        assert_eq!(conn.endpoint, "dst:8080");
    }

    #[test]
    fn generate_config_data_local_http() {
        let cd = make_conn_details("host:8080", None);
        let dest = make_node("dst", Some("g"), vec![cd.clone()]);
        let (ep, config) = generate_config_data(&cd, true, &dest).unwrap();
        assert!(ep.starts_with("http://"));
        assert!(!config.tls_required);
    }

    #[test]
    fn generate_config_data_external_no_mtls() {
        let cd = make_conn_details("host:8080", Some("ext:9090"));
        let dest = make_node("dst", Some("g1"), vec![cd.clone()]);
        let (ep, config) = generate_config_data(&cd, false, &dest).unwrap();
        assert!(ep.contains("ext:9090"));
        assert!(!config.tls_required);
    }

    #[test]
    fn generate_config_data_no_external_endpoint_remote_returns_error() {
        let cd = make_conn_details("host:8080", None);
        let dest = make_node("dst", None, vec![cd.clone()]);
        assert!(generate_config_data(&cd, false, &dest).is_err());
    }

    #[tokio::test]
    async fn get_client_config_dest_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let svc = make_route_service(db);
        let result = svc.get_client_config("src", "ghost_dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost_dst"));
    }

    #[tokio::test]
    async fn get_client_config_src_not_found_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![make_conn_details("dst:8080", None)],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_client_config("ghost_src", "dst").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost_src"));
    }

    #[tokio::test]
    async fn get_client_config_no_conn_details_returns_error() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: None,
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: None,
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let result = svc.get_client_config("src", "dst").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_client_config_same_group_local() {
        let db = InMemoryDb::shared();
        let dst = crate::db::Node {
            id: "dst".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![make_conn_details("dst:8080", None)],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(dst).await.unwrap();
        let src = crate::db::Node {
            id: "src".to_string(),
            group_name: Some("grp".to_string()),
            conn_details: vec![],
            created_at: SystemTime::now(),
            last_updated: SystemTime::now(),
        };
        db.save_node(src).await.unwrap();
        let svc = make_route_service(db);
        let (ep, _) = svc.get_client_config("src", "dst").await.unwrap();
        assert!(ep.starts_with("http://dst:8080"));
    }

    #[test]
    fn connection_not_found_exact() {
        assert!(is_connection_not_found("connection not found"));
        assert!(is_connection_not_found("Connection Not Found"));
    }

    #[test]
    fn connection_not_found_no_such() {
        assert!(is_connection_not_found("no such connection"));
        assert!(is_connection_not_found("No Such Connection"));
    }

    #[test]
    fn connection_not_found_split_words() {
        assert!(is_connection_not_found("the connection was not found"));
    }

    #[test]
    fn connection_not_found_rejects_unrelated() {
        assert!(!is_connection_not_found("subscription not found"));
        assert!(!is_connection_not_found("route not found"));
        assert!(!is_connection_not_found("node not found"));
        assert!(!is_connection_not_found("not found"));
    }

    #[test]
    fn endpoint_matches_exact() {
        assert!(endpoint_matches("https://host:8080", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_scheme_added() {
        assert!(endpoint_matches("https://host:8080", "host:8080"));
        assert!(endpoint_matches("http://host:8080", "host:8080"));
    }

    #[test]
    fn endpoint_matches_scheme_on_node_side() {
        assert!(endpoint_matches("host:8080", "https://host:8080"));
        assert!(endpoint_matches("host:8080", "http://host:8080"));
    }

    #[test]
    fn endpoint_matches_trailing_slash() {
        assert!(endpoint_matches("https://host:8080/", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_case_insensitive() {
        assert!(endpoint_matches("HTTPS://Host:8080", "https://host:8080"));
    }

    #[test]
    fn endpoint_matches_no_false_positive() {
        assert!(!endpoint_matches("https://host:8080", "https://other:8080"));
        assert!(!endpoint_matches("host:9090", "host:8080"));
    }

    #[test]
    fn generate_config_data_tls_spire_returns_https_and_spire_auth() {
        let cd = crate::db::ConnectionDetails {
            endpoint: "host:8080".to_string(),
            external_endpoint: Some("ext:9090".to_string()),
            tls_required: true,
            auth_method: AuthMethod::Spire,
            spire_trust_domain: Some("mygroup".to_string()),
        };
        let dest = make_node("dst", Some("mygroup"), vec![cd.clone()]);
        let (ep, config) = generate_config_data(&cd, false, &dest).unwrap();
        assert!(ep.starts_with("https://"), "expected https, got {ep}");
        assert!(config.tls_required);
        assert_eq!(
            config.auth_method,
            RequiredAuthMethod::Spire {
                trust_domain: Some("mygroup".to_string())
            }
        );
    }

    #[test]
    fn generate_config_data_tls_spire_falls_back_to_group_trust_domain() {
        let cd = crate::db::ConnectionDetails {
            endpoint: "host:8080".to_string(),
            external_endpoint: Some("ext:9090".to_string()),
            tls_required: true,
            auth_method: AuthMethod::Spire,
            spire_trust_domain: None, // no explicit trust domain
        };
        let dest = make_node("dst", Some("fallback-group"), vec![cd.clone()]);
        let (_, config) = generate_config_data(&cd, false, &dest).unwrap();
        assert_eq!(
            config.auth_method,
            RequiredAuthMethod::Spire {
                trust_domain: Some("fallback-group".to_string())
            }
        );
    }

    #[test]
    fn find_reported_connection_matches_by_endpoint() {
        use slim_config::client::ServerConnectionConfig;
        let reported = vec![
            ReportedConnection {
                endpoint: "https://host:8080".to_string(),
                link_id: "link-1".to_string(),
                config_data: ServerConnectionConfig::default(),
            },
            ReportedConnection {
                endpoint: "https://other:9090".to_string(),
                link_id: "link-2".to_string(),
                config_data: ServerConnectionConfig::default(),
            },
        ];
        let found = find_reported_connection(&reported, "host:8080");
        assert!(found.is_some());
        assert_eq!(found.unwrap().link_id, "link-1");
        assert!(find_reported_connection(&reported, "missing:1234").is_none());
    }

    #[test]
    fn find_reported_connection_for_dest_matches_internal_or_external() {
        use slim_config::client::ServerConnectionConfig;
        let reported = vec![ReportedConnection {
            endpoint: "https://ext:9090".to_string(),
            link_id: "link-ext".to_string(),
            config_data: ServerConnectionConfig::default(),
        }];
        let dst = make_node(
            "dst",
            None,
            vec![make_conn_details("host:8080", Some("ext:9090"))],
        );
        let found = find_reported_connection_for_dest(&reported, &dst);
        assert!(found.is_some());
        assert_eq!(found.unwrap().link_id, "link-ext");

        let unrelated = make_node("other", None, vec![make_conn_details("other:1111", None)]);
        assert!(find_reported_connection_for_dest(&reported, &unrelated).is_none());
    }
}

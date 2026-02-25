// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, bail};

use crate::proto::controller::proto::v1::Connection;

/// Parse a route string "organization/namespace/agentname/agentid" into its components.
/// Returns (organization, namespace, agent_type, agent_id).
pub fn parse_route(route: &str) -> Result<(String, String, String, u64)> {
    let parts: Vec<&str> = route.split('/').collect();
    if parts.len() != 4 {
        bail!(
            "invalid route format '{}', expected 'organization/namespace/agentname/agentid'",
            route
        );
    }
    if parts.iter().any(|p| p.is_empty()) {
        bail!(
            "invalid route format '{}', expected 'organization/namespace/agentname/agentid'",
            route
        );
    }
    let agent_id: u64 = parts[3]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid agent instance ID (must be u64): '{}'", parts[3]))?;
    Ok((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
        agent_id,
    ))
}

/// Return true if the string looks like an endpoint URL (contains ':' or starts with http/https).
pub fn is_endpoint(s: &str) -> bool {
    s.contains(':') || s.starts_with("http://") || s.starts_with("https://")
}

/// Parse an endpoint string "http://host:port" or "https://host:port" into a Connection.
/// Returns (Connection, connection_id).
pub fn parse_endpoint(endpoint: &str) -> Result<(Connection, String)> {
    let url = url::Url::parse(endpoint)
        .map_err(|e| anyhow::anyhow!("failed to parse endpoint '{}': {}", endpoint, e))?;

    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        bail!(
            "unsupported scheme '{}' in endpoint '{}', must be 'http' or 'https'",
            scheme,
            endpoint
        );
    }

    let host = url.host_str().unwrap_or("");
    if host.is_empty() {
        bail!("invalid endpoint '{}': host part is missing", endpoint);
    }

    let port = url
        .port()
        .ok_or_else(|| anyhow::anyhow!("invalid endpoint '{}': port part is missing", endpoint))?;

    if port == 0 {
        bail!(
            "port number '{}' in endpoint '{}' is out of range (1-65535)",
            port,
            endpoint
        );
    }

    let conn = Connection {
        connection_id: endpoint.to_string(),
        config_data: String::new(),
    };

    Ok((conn, endpoint.to_string()))
}

/// Parse a JSON config file and extract the connection info.
/// The file must contain an "endpoint" key.
pub fn parse_config_file(config_file: &str) -> Result<Connection> {
    if config_file.is_empty() {
        bail!("config file path cannot be empty");
    }
    if !config_file.ends_with(".json") {
        bail!("config file '{}' must be a JSON file", config_file);
    }

    let data = std::fs::read_to_string(config_file)
        .map_err(|e| anyhow::anyhow!("failed to read config file '{}': {}", config_file, e))?;

    let json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| anyhow::anyhow!("invalid JSON in config file '{}': {}", config_file, e))?;

    let endpoint = json
        .get("endpoint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "'endpoint' key not found or empty in config file '{}'",
                config_file
            )
        })?;

    Ok(Connection {
        connection_id: endpoint.to_string(),
        config_data: data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── parse_route ─────────────────────────────────────────────────────────

    #[test]
    fn parse_route_valid() {
        let (org, ns, agent, id) = parse_route("myorg/mynamespace/myagent/42").unwrap();
        assert_eq!(org, "myorg");
        assert_eq!(ns, "mynamespace");
        assert_eq!(agent, "myagent");
        assert_eq!(id, 42);
    }

    #[test]
    fn parse_route_zero_agent_id() {
        let (_, _, _, id) = parse_route("org/ns/agent/0").unwrap();
        assert_eq!(id, 0);
    }

    #[test]
    fn parse_route_max_u64_agent_id() {
        let (_, _, _, id) = parse_route("org/ns/agent/18446744073709551615").unwrap();
        assert_eq!(id, u64::MAX);
    }

    #[test]
    fn parse_route_too_few_parts() {
        assert!(parse_route("org/ns/agent").is_err());
    }

    #[test]
    fn parse_route_too_many_parts() {
        assert!(parse_route("org/ns/agent/42/extra").is_err());
    }

    #[test]
    fn parse_route_empty_middle_part() {
        assert!(parse_route("org//agent/42").is_err());
    }

    #[test]
    fn parse_route_leading_slash() {
        assert!(parse_route("/ns/agent/42").is_err());
    }

    #[test]
    fn parse_route_non_u64_agent_id() {
        assert!(parse_route("org/ns/agent/notanumber").is_err());
    }

    #[test]
    fn parse_route_negative_agent_id() {
        assert!(parse_route("org/ns/agent/-1").is_err());
    }

    // ── is_endpoint ─────────────────────────────────────────────────────────

    #[test]
    fn is_endpoint_with_colon() {
        assert!(is_endpoint("localhost:50051"));
    }

    #[test]
    fn is_endpoint_http_prefix() {
        assert!(is_endpoint("http://localhost:50051"));
    }

    #[test]
    fn is_endpoint_https_prefix() {
        assert!(is_endpoint("https://localhost:50051"));
    }

    #[test]
    fn is_endpoint_plain_string() {
        assert!(!is_endpoint("some-node-id"));
    }

    #[test]
    fn is_endpoint_empty() {
        assert!(!is_endpoint(""));
    }

    #[test]
    fn is_endpoint_uuid_like() {
        assert!(!is_endpoint("550e8400-e29b-41d4-a716-446655440000"));
    }

    // ── parse_endpoint ──────────────────────────────────────────────────────

    #[test]
    fn parse_endpoint_valid_http() {
        let (conn, id) = parse_endpoint("http://localhost:8080").unwrap();
        assert_eq!(conn.connection_id, "http://localhost:8080");
        assert_eq!(conn.config_data, "");
        assert_eq!(id, "http://localhost:8080");
    }

    #[test]
    fn parse_endpoint_valid_https() {
        // Use a non-default port; url::Url::port() returns None for scheme defaults (443).
        let (conn, id) = parse_endpoint("https://example.com:8443").unwrap();
        assert_eq!(conn.connection_id, "https://example.com:8443");
        assert_eq!(id, "https://example.com:8443");
    }

    #[test]
    fn parse_endpoint_unsupported_scheme() {
        assert!(parse_endpoint("grpc://host:50051").is_err());
    }

    #[test]
    fn parse_endpoint_missing_port() {
        assert!(parse_endpoint("http://localhost").is_err());
    }

    #[test]
    fn parse_endpoint_not_a_url() {
        assert!(parse_endpoint("not_a_url").is_err());
    }

    // ── parse_config_file ───────────────────────────────────────────────────

    #[test]
    fn parse_config_file_empty_path() {
        assert!(parse_config_file("").is_err());
    }

    #[test]
    fn parse_config_file_non_json_extension() {
        assert!(parse_config_file("config.yaml").is_err());
    }

    #[test]
    fn parse_config_file_nonexistent() {
        assert!(parse_config_file("/nonexistent/path/config.json").is_err());
    }

    #[test]
    fn parse_config_file_valid() {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": "http://host:8080"}}"#).unwrap();
        let conn = parse_config_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(conn.connection_id, "http://host:8080");
        assert!(conn.config_data.contains("endpoint"));
    }

    #[test]
    fn parse_config_file_config_data_is_full_json() {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let json = r#"{"endpoint": "http://host:9090", "extra": "value"}"#;
        write!(f, "{}", json).unwrap();
        let conn = parse_config_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(conn.config_data, json);
    }

    #[test]
    fn parse_config_file_invalid_json() {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, "not json").unwrap();
        assert!(parse_config_file(f.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn parse_config_file_missing_endpoint_key() {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"other_key": "value"}}"#).unwrap();
        assert!(parse_config_file(f.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn parse_config_file_empty_endpoint_value() {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(f, r#"{{"endpoint": ""}}"#).unwrap();
        assert!(parse_config_file(f.path().to_str().unwrap()).is_err());
    }
}

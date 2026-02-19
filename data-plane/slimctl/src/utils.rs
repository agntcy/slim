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

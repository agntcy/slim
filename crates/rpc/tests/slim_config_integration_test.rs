// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the hierarchical slim.yaml config loading and
//! `create_app_from_slim_config` one-call API.
//!
//! Tests that require a live SLIM node start one in-process using
//! `Service::run_server` on a random free port.

use std::net::TcpListener;
use std::time::Duration;

use slim_config::component::id::{ID, Kind};
use slim_config::server::ServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_service::{Service, KIND};
use slim_service::node_config::load_slim_node_config;
use tempfile::TempDir;

// ============================================================================
// Helpers
// ============================================================================

/// Bind to port 0 to let the OS pick a free port, then release and return it.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Minimal slim.yaml content that points at the given endpoint with a shared-secret identity.
fn slim_yaml(endpoint: &str, app_name: &str, secret: &str) -> String {
    format!(
        "node:\n  endpoint: \"{endpoint}\"\napp:\n  name: \"{app_name}\"\n  identity:\n    type: shared_secret\n    data: \"{secret}\"\n  identity_verifier:\n    type: shared_secret\n    data: \"{secret}\"\n"
    )
}

fn make_service(name: &str) -> Service {
    Service::new(ID::new_with_name(Kind::new(KIND).unwrap(), name).unwrap())
}

fn insecure_server_config(endpoint: String) -> ServerConfig {
    ServerConfig {
        endpoint,
        tls_setting: TlsServerConfig::insecure(),
        ..Default::default()
    }
}

// ============================================================================
// Test 1: create_app_from_slim_config — end-to-end with a real server
// ============================================================================

/// Verifies that `load_slim_node_config` + `Service::create_app_from_slim_config` produces
/// a connected, subscribed app against a live in-process SLIM node.
#[tokio::test]
async fn test_create_app_from_slim_config() {
    let port = free_port();
    let endpoint = format!("http://127.0.0.1:{port}");
    let secret = "test-secret-with-sufficient-length-for-hmac-key";

    // Start a SLIM server on the chosen port
    let server_svc = make_service("cfg-integ-server");
    server_svc
        .run_server(&insecure_server_config(format!("127.0.0.1:{port}")))
        .await
        .expect("server should start");

    // Give the server a moment to accept connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Write slim.yaml to a temporary directory
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("slim.yaml");
    std::fs::write(
        &config_path,
        slim_yaml(&endpoint, "test/config/receiver", secret),
    )
    .unwrap();

    // Load config from the explicit path (bypasses CWD discovery)
    let config = load_slim_node_config(Some(config_path.to_str().unwrap()))
        .expect("slim.yaml should parse");

    // Create the app using the one-call API on a separate client service
    let client_svc = make_service("cfg-integ-client");
    let handle = client_svc
        .create_app_from_slim_config(config)
        .await
        .expect("create_app_from_slim_config should succeed");

    // The connection ID must be non-zero
    assert!(handle.conn_id > 0, "conn_id should be non-zero");

    // The name should be the 3-component app name from the config.
    let (c0, c1, c2) = handle.name.str_components();
    let name_str = format!("{c0}/{c1}/{c2}");
    assert_eq!(
        name_str, "test/config/receiver",
        "name should match the app name from slim.yaml, got: {name_str}"
    );
}

// ============================================================================
// Test 2: env-var override
// ============================================================================

/// Verifies that `SLIM_NODE_ADDRESS` takes precedence over the endpoint in slim.yaml.
/// Uses `test_fork::fork` so env-var mutations don't leak to parallel tests.
#[test_fork::fork]
#[test]
fn test_load_slim_config_env_var_override() {
    // SAFETY: runs in a forked child process — no concurrent threads.
    unsafe {
        std::env::set_var("SLIM_NODE_ADDRESS", "http://override.example.com:9999");
        std::env::remove_var("SLIM_APP_NAME");
        std::env::remove_var("SLIM_NODE_CERT_PATH");
        std::env::remove_var("SLIM_IDENTITY_TOKEN");
        std::env::remove_var("SLIM_IDENTITY_SHARED_SECRET");
    }

    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("slim.yaml");
    std::fs::write(
        &config_path,
        "node:\n  endpoint: \"http://original.example.com:46357\"\n",
    )
    .unwrap();

    let config = load_slim_node_config(Some(config_path.to_str().unwrap()))
        .expect("config should load");

    assert_eq!(
        config.node.endpoint,
        "http://override.example.com:9999",
        "SLIM_NODE_ADDRESS should override the file endpoint"
    );
}

// ============================================================================
// Test 3: missing config returns an error
// ============================================================================

/// Verifies that `load_slim_node_config` fails gracefully when no config can be found
/// and no env vars are set.
#[test_fork::fork]
#[test]
fn test_load_slim_config_missing_returns_error() {
    // SAFETY: forked child process.
    unsafe {
        std::env::remove_var("SLIM_NODE_ADDRESS");
        std::env::remove_var("SLIM_APP_NAME");
        std::env::remove_var("SLIM_NODE_CERT_PATH");
        std::env::remove_var("SLIM_IDENTITY_TOKEN");
        std::env::remove_var("SLIM_IDENTITY_SHARED_SECRET");
    }

    let result = load_slim_node_config(Some("/nonexistent/path/slim.yaml"));
    assert!(
        result.is_err(),
        "loading a non-existent config file should return an error"
    );
}

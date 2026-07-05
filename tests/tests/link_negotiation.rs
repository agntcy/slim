//! Link negotiation between two SLIM dataplane nodes.
//!
//! Covers bidirectional negotiation (request on the server, reply on the client),
//! remote version advertisement in logs, and legacy-binary compatibility scenarios
//! (ignored until `.dist/bin/slim-legacy` exists).

use slim_integration_tests::{
    binaries::{require_legacy_slim_binary, require_slim_binary, workspace_root},
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn link_neg_port_replacements(server_port: u16, node_b_port: u16) -> HashMap<String, String> {
    HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{server_port}"),
        ),
        (
            "0.0.0.0:46481".to_string(),
            format!("0.0.0.0:{node_b_port}"),
        ),
        (
            "http://localhost:46480".to_string(),
            format!("http://localhost:{server_port}"),
        ),
    ])
}

fn client_server_port_replacements(server_port: u16) -> HashMap<String, String> {
    HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{server_port}"),
        ),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{server_port}"),
        ),
    ])
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn spawn_slim(slim: &Path, config: &Path) -> std::process::Child {
    Command::new(slim)
        .arg("--config")
        .arg(config)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| {
            panic!(
                "failed to start slim with config {}: {err}",
                config.display()
            )
        })
}

/// Shared two-node topology.
fn run_two_new_nodes_link_negotiation(verify_version: bool) {
    let temp_dir = new_temp_dir("slim-integration-link-neg-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let server_port = reserve_port();
    let node_b_port = reserve_port();
    let replacements = link_neg_port_replacements(server_port, node_b_port);

    let testdata = testdata_dir();
    let server_config = write_temp_config(
        &temp_dir,
        &testdata.join("server.yaml"),
        "server.yaml",
        &replacements,
    );
    let node_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("link-neg-node-with-client-config.yaml"),
        "node-b.yaml",
        &replacements,
    );

    let slim = require_slim_binary();

    let mut server_session = Some(spawn_slim(&slim, &server_config));
    let server_logs = ProcessLogWatcher::attach(server_session.as_mut().expect("server session"));

    server_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("server did not start:\n{output}");
        });

    let mut node_b_session = Some(spawn_slim(&slim, &node_b_config));
    let node_b_logs = ProcessLogWatcher::attach(node_b_session.as_mut().expect("node b session"));

    // Server (A) receives the negotiation request (is_reply=false).
    server_logs
        .wait_contains("received link negotiation", Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_b_session, Duration::from_secs(5));
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("server did not log link negotiation:\n{output}");
        });

    // Client (B) receives the reply (is_reply=true).
    node_b_logs
        .wait_contains("received link negotiation", Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_b_session, Duration::from_secs(5));
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("node B did not log link negotiation:\n{output}");
        });

    if verify_version {
        server_logs
            .wait_matches(contains_semver_fragment, Duration::from_secs(10))
            .unwrap_or_else(|output| {
                terminate_session(&mut node_b_session, Duration::from_secs(5));
                terminate_session(&mut server_session, Duration::from_secs(5));
                panic!("server did not log remote version:\n{output}");
            });

        node_b_logs
            .wait_matches(contains_semver_fragment, Duration::from_secs(10))
            .unwrap_or_else(|output| {
                terminate_session(&mut node_b_session, Duration::from_secs(5));
                terminate_session(&mut server_session, Duration::from_secs(5));
                panic!("node B did not log remote version:\n{output}");
            });
    }

    session_still_running(
        server_session.as_mut().expect("server session"),
        Duration::from_millis(500),
    )
    .unwrap_or_else(|err| {
        terminate_session(&mut node_b_session, Duration::from_secs(5));
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!("server exited early: {err}");
    });

    session_still_running(
        node_b_session.as_mut().expect("node b session"),
        Duration::from_millis(500),
    )
    .unwrap_or_else(|err| {
        terminate_session(&mut node_b_session, Duration::from_secs(5));
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!("node B exited early: {err}");
    });

    terminate_session(&mut node_b_session, Duration::from_secs(5));
    terminate_session(&mut server_session, Duration::from_secs(5));
}

/// Current client connecting to a legacy server stays up without a negotiation reply.
fn run_new_client_connects_to_legacy_server() {
    let temp_dir = new_temp_dir("slim-integration-link-neg-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let server_port = reserve_port();
    let replacements = client_server_port_replacements(server_port);
    let testdata = testdata_dir();

    let server_config = write_temp_config(
        &temp_dir,
        &testdata.join("server.yaml"),
        "legacy-server.yaml",
        &replacements,
    );
    let client_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "new-client.yaml",
        &replacements,
    );

    let legacy_slim = require_legacy_slim_binary();
    let slim = require_slim_binary();

    let mut server_session = Some(spawn_slim(&legacy_slim, &server_config));
    let server_logs = ProcessLogWatcher::attach(server_session.as_mut().expect("server session"));
    server_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("legacy server did not start:\n{output}");
        });

    let mut client_session = Some(spawn_slim(&slim, &client_config));
    let client_logs = ProcessLogWatcher::attach(client_session.as_mut().expect("client session"));
    client_logs
        .wait_contains("new connection initiated locally", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_session, Duration::from_secs(5));
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("client did not initiate connection:\n{output}");
        });

    session_still_running(
        client_session.as_mut().expect("client session"),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|err| {
        terminate_session(&mut client_session, Duration::from_secs(5));
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!("client exited early: {err}");
    });

    terminate_session(&mut client_session, Duration::from_secs(5));
    terminate_session(&mut server_session, Duration::from_secs(5));
}

/// Legacy client connecting to a current server is accepted without negotiation.
fn run_legacy_client_connects_to_new_server() {
    let temp_dir = new_temp_dir("slim-integration-link-neg-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let server_port = reserve_port();
    let replacements = client_server_port_replacements(server_port);
    let testdata = testdata_dir();

    let server_config = write_temp_config(
        &temp_dir,
        &testdata.join("server.yaml"),
        "new-server.yaml",
        &replacements,
    );
    let client_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "legacy-client.yaml",
        &replacements,
    );

    let slim = require_slim_binary();
    let legacy_slim = require_legacy_slim_binary();

    let mut server_session = Some(spawn_slim(&slim, &server_config));
    let server_logs = ProcessLogWatcher::attach(server_session.as_mut().expect("server session"));
    server_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("server did not start:\n{output}");
        });

    let mut client_session = Some(spawn_slim(&legacy_slim, &client_config));
    let _client_logs = ProcessLogWatcher::attach(client_session.as_mut().expect("client session"));

    server_logs
        .wait_contains("new connection received from remote", Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_session, Duration::from_secs(5));
            terminate_session(&mut server_session, Duration::from_secs(5));
            panic!("server did not accept legacy client:\n{output}");
        });

    session_still_running(
        server_session.as_mut().expect("server session"),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|err| {
        terminate_session(&mut client_session, Duration::from_secs(5));
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!("server exited early: {err}");
    });

    terminate_session(&mut client_session, Duration::from_secs(5));
    terminate_session(&mut server_session, Duration::from_secs(5));
}

/// Two current SLIM nodes complete link negotiation in both directions.
#[test]
fn completes_link_negotiation_in_both_directions() {
    run_two_new_nodes_link_negotiation(false);
}

/// Negotiation logs include a semver-like remote version on both nodes.
#[test]
fn logs_remote_version_after_link_negotiation() {
    run_two_new_nodes_link_negotiation(true);
}

/// New client connecting to a legacy server remains stable without a negotiation reply.
#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn new_client_connects_to_old_server() {
    run_new_client_connects_to_legacy_server();
}

/// Legacy client connecting to a current server is accepted without negotiation.
#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn old_client_connects_to_new_server() {
    run_legacy_client_connects_to_new_server();
}

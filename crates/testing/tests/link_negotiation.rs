//! Link negotiation between two SLIM dataplane nodes.
//!
//! Covers bidirectional negotiation (request on the server, reply on the client)
//! and remote version advertisement in logs.

use slim_testing::{
    binaries::{require_slim_binary, workspace_root},
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

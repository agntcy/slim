//! Header MAC validation across federated SLIM dataplane nodes.
//!
//! Covers the happy path (link negotiation + cross-node sdk-mock delivery) and
//! tampered-destination rejection. Both depend on `slimctl n route add` and stay
//! `#[ignore]` until topology-based routing replaces manual route wiring.

use slim_integration_tests::{
    binaries::{
        require_sdk_mock_binary, require_slim_binary, require_slimctl_binary, workspace_root,
    },
    constants::{MSG_CONTROLPLANE_SERVER_STARTED, MSG_HEADER_INTEGRITY_FAILED, MSG_HELLO_FROM_A},
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

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn header_integrity_replacements(
    node_a_port: u16,
    node_b_port: u16,
    ctrl_a_port: u16,
    ctrl_b_port: u16,
) -> HashMap<String, String> {
    HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{node_a_port}"),
        ),
        (
            "0.0.0.0:46358".to_string(),
            format!("0.0.0.0:{ctrl_a_port}"),
        ),
        (
            "0.0.0.0:46481".to_string(),
            format!("0.0.0.0:{node_b_port}"),
        ),
        (
            "0.0.0.0:46482".to_string(),
            format!("0.0.0.0:{ctrl_b_port}"),
        ),
        (
            "http://localhost:46480".to_string(),
            format!("http://localhost:{node_a_port}"),
        ),
    ])
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

fn spawn_slim_with_env(slim: &Path, config: &Path, env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = Command::new(slim);
    cmd.arg("--config")
        .arg(config)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.spawn().unwrap_or_else(|err| {
        panic!(
            "failed to start slim with config {}: {err}",
            config.display()
        )
    })
}

fn spawn_sdk_mock(
    sdk_mock: &Path,
    config: &Path,
    local_name: &str,
    remote_name: &str,
    message: Option<&str>,
) -> std::process::Child {
    let mut cmd = Command::new(sdk_mock);
    cmd.arg("--config")
        .arg(config)
        .arg("--local-name")
        .arg(local_name)
        .arg("--remote-name")
        .arg(remote_name)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(message) = message {
        cmd.arg("--message").arg(message);
    }

    cmd.spawn().unwrap_or_else(|err| {
        panic!(
            "failed to start sdk-mock with config {}: {err}",
            config.display()
        )
    })
}

fn wait_for_federated_link(node_a_logs: &ProcessLogWatcher, node_b_logs: &ProcessLogWatcher) {
    node_b_logs
        .wait_contains("client connected", Duration::from_secs(15))
        .unwrap_or_else(|output| panic!("node B did not connect to node A:\n{output}"));
    node_a_logs
        .wait_contains("received link negotiation", Duration::from_secs(15))
        .unwrap_or_else(|output| panic!("node A did not log link negotiation:\n{output}"));
    node_b_logs
        .wait_contains("received link negotiation", Duration::from_secs(15))
        .unwrap_or_else(|output| panic!("node B did not log link negotiation:\n{output}"));
}

struct HeaderMacTopology {
    controller_a_endpoint: String,
    controller_b_endpoint: String,
    client_a_config: PathBuf,
    client_b_config: PathBuf,
    client_a_via: PathBuf,
    client_b_via: PathBuf,
    node_a: Option<std::process::Child>,
    node_b: Option<std::process::Child>,
    node_b_logs: ProcessLogWatcher,
}

fn setup_header_mac_topology(tamper_destination: bool) -> HeaderMacTopology {
    let temp_dir = new_temp_dir("slim-integration-header-mac-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let node_a_port = reserve_port();
    let node_b_port = reserve_port();
    let controller_a_port = reserve_port();
    let controller_b_port = reserve_port();
    let repl = header_integrity_replacements(
        node_a_port,
        node_b_port,
        controller_a_port,
        controller_b_port,
    );

    let testdata = testdata_dir();
    let node_a_name = if tamper_destination {
        "node-a-tamper.yaml"
    } else {
        "node-a.yaml"
    };
    let node_b_name = if tamper_destination {
        "node-b-tamper.yaml"
    } else {
        "node-b.yaml"
    };
    let node_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("header-mac-node-a.yaml"),
        node_a_name,
        &repl,
    );
    let node_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("header-mac-node-b.yaml"),
        node_b_name,
        &repl,
    );

    let client_replacements_a = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{node_a_port}"),
    )]);
    let client_replacements_b = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{node_b_port}"),
    )]);
    let client_a_name = if tamper_destination {
        "header-mac-tamper-client-a.yaml"
    } else {
        "header-mac-client-a.yaml"
    };
    let client_b_name = if tamper_destination {
        "header-mac-tamper-client-b.yaml"
    } else {
        "header-mac-client-b.yaml"
    };
    let client_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        client_a_name,
        &client_replacements_a,
    );
    let client_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        client_b_name,
        &client_replacements_b,
    );

    let via_peer_b = HashMap::from([(
        "http://127.0.0.1:46481".to_string(),
        format!("http://127.0.0.1:{node_b_port}"),
    )]);
    let via_peer_a = HashMap::from([(
        "http://127.0.0.1:46357".to_string(),
        format!("http://127.0.0.1:{node_a_port}"),
    )]);
    let via_a_name = if tamper_destination {
        "header-mac-tamper-client-a-via.json"
    } else {
        "header-mac-client-a-via.json"
    };
    let via_b_name = if tamper_destination {
        "header-mac-tamper-client-b-via.json"
    } else {
        "header-mac-client-b-via.json"
    };
    let client_a_via = write_temp_config(
        &temp_dir,
        &testdata.join("header-mac-via-peer-b.json"),
        via_a_name,
        &via_peer_b,
    );
    let client_b_via = write_temp_config(
        &temp_dir,
        &testdata.join("header-mac-via-peer-a.json"),
        via_b_name,
        &via_peer_a,
    );

    let slim = require_slim_binary();
    let mut node_a = if tamper_destination {
        Some(spawn_slim_with_env(
            &slim,
            &node_a_config,
            &[("SLIM_TEST_TAMPER_DESTINATION", "1")],
        ))
    } else {
        Some(spawn_slim(&slim, &node_a_config))
    };
    let node_a_logs = ProcessLogWatcher::attach(node_a.as_mut().expect("node a"));
    node_a_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_a, Duration::from_secs(30));
            panic!("node A dataplane did not start:\n{output}");
        });

    let mut node_b = Some(spawn_slim(&slim, &node_b_config));
    let node_b_logs = ProcessLogWatcher::attach(node_b.as_mut().expect("node b"));
    node_b_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_b, Duration::from_secs(30));
            terminate_session(&mut node_a, Duration::from_secs(30));
            panic!("node B dataplane did not start:\n{output}");
        });

    node_a_logs
        .wait_contains(MSG_CONTROLPLANE_SERVER_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_b, Duration::from_secs(30));
            terminate_session(&mut node_a, Duration::from_secs(30));
            panic!("node A control plane did not start:\n{output}");
        });
    node_b_logs
        .wait_contains(MSG_CONTROLPLANE_SERVER_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut node_b, Duration::from_secs(30));
            terminate_session(&mut node_a, Duration::from_secs(30));
            panic!("node B control plane did not start:\n{output}");
        });

    wait_for_federated_link(&node_a_logs, &node_b_logs);
    drop(node_a_logs);

    HeaderMacTopology {
        controller_a_endpoint: format!("127.0.0.1:{controller_a_port}"),
        controller_b_endpoint: format!("127.0.0.1:{controller_b_port}"),
        client_a_config,
        client_b_config,
        client_a_via,
        client_b_via,
        node_a,
        node_b,
        node_b_logs,
    }
}

fn wire_cross_node_routes(
    slimctl: &Path,
    setup: &HeaderMacTopology,
    sdk_mock: &Path,
) -> (
    Option<std::process::Child>,
    ProcessLogWatcher,
    Option<std::process::Child>,
    ProcessLogWatcher,
) {
    let mut client_b = Some(spawn_sdk_mock(
        sdk_mock,
        &setup.client_b_config,
        "b",
        "a",
        None,
    ));
    let client_b_logs = ProcessLogWatcher::attach(client_b.as_mut().expect("client b"));

    let route_b = wait_for_route_with_connections_format(
        slimctl,
        &setup.controller_b_endpoint,
        "org/default/b",
        Duration::from_secs(10),
    );
    run_slimctl_node_add_route_via(
        slimctl,
        &setup.controller_a_endpoint,
        &route_b,
        &setup.client_a_via,
        Duration::from_secs(10),
    );

    let mut client_a = Some(spawn_sdk_mock(
        sdk_mock,
        &setup.client_a_config,
        "a",
        "b",
        Some("hey"),
    ));
    let client_a_logs = ProcessLogWatcher::attach(client_a.as_mut().expect("client a"));

    let route_a = wait_for_route_with_connections_format(
        slimctl,
        &setup.controller_a_endpoint,
        "org/default/a",
        Duration::from_secs(10),
    );
    run_slimctl_node_add_route_via(
        slimctl,
        &setup.controller_b_endpoint,
        &route_a,
        &setup.client_b_via,
        Duration::from_secs(10),
    );

    (client_a, client_a_logs, client_b, client_b_logs)
}

#[test]
#[ignore = "pending: slimctl route add removed — update to topology-based routing"]
fn routes_sdk_mock_across_federated_nodes() {
    let slimctl = require_slimctl_binary();
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_header_mac_topology(false);

    let (mut client_a, client_a_logs, mut client_b, client_b_logs) =
        wire_cross_node_routes(&slimctl, &setup, &sdk_mock);

    client_b_logs
        .wait_contains(MSG_HELLO_FROM_A, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.node_b, Duration::from_secs(30));
            terminate_session(&mut setup.node_a, Duration::from_secs(30));
            panic!("client B did not receive message from A:\n{output}");
        });

    client_a_logs
        .wait_contains("hello from the b", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.node_b, Duration::from_secs(30));
            terminate_session(&mut setup.node_a, Duration::from_secs(30));
            panic!("client A did not receive message from B:\n{output}");
        });

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut client_b, Duration::from_secs(2));
    terminate_session(&mut setup.node_b, Duration::from_secs(30));
    terminate_session(&mut setup.node_a, Duration::from_secs(30));
}

#[test]
#[ignore = "pending: slimctl route add removed — update to topology-based routing"]
fn rejects_tampered_destination_packets() {
    let slimctl = require_slimctl_binary();
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_header_mac_topology(true);

    let (mut client_a, client_a_logs, mut client_b, client_b_logs) =
        wire_cross_node_routes(&slimctl, &setup, &sdk_mock);

    setup
        .node_b_logs
        .wait_contains(MSG_HEADER_INTEGRITY_FAILED, Duration::from_secs(20))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.node_b, Duration::from_secs(30));
            terminate_session(&mut setup.node_a, Duration::from_secs(30));
            panic!("node B did not log header integrity failure:\n{output}");
        });

    client_b_logs
        .wait_not_contains(MSG_HELLO_FROM_A, Duration::from_secs(5))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.node_b, Duration::from_secs(30));
            terminate_session(&mut setup.node_a, Duration::from_secs(30));
            panic!("client B unexpectedly received tampered message:\n{output}");
        });

    client_a_logs
        .wait_not_contains("hello from the b", Duration::from_secs(5))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.node_b, Duration::from_secs(30));
            terminate_session(&mut setup.node_a, Duration::from_secs(30));
            panic!("client A unexpectedly received tampered reply:\n{output}");
        });

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut client_b, Duration::from_secs(2));
    terminate_session(&mut setup.node_b, Duration::from_secs(30));
    terminate_session(&mut setup.node_a, Duration::from_secs(30));
}

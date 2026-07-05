//! Message routing between two SLIM nodes with embedded controllers.
//!
//! These scenarios depend on the removed `slimctl n route add` command and stay
//! `#[ignore]` until topology-based routing replaces manual route wiring.

use slim_integration_tests::{
    binaries::{require_sdk_mock_binary, require_slim_binary, require_slimctl_binary, workspace_root},
    constants::{MSG_CONTROLPLANE_SERVER_STARTED, MSG_HELLO_FROM_A},
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
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

struct TwoNodeRoutingSetup {
    data_plane_b_port: u16,
    controller_a_endpoint: String,
    controller_b_endpoint: String,
    client_a_config: PathBuf,
    client_b_config: PathBuf,
    client_a_via: PathBuf,
    client_b_via: PathBuf,
    server_a: Option<std::process::Child>,
    server_b: Option<std::process::Child>,
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

fn setup_two_node_routing() -> TwoNodeRoutingSetup {
    let temp_dir = new_temp_dir("slim-integration-routing-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let data_plane_a_port = reserve_port();
    let data_plane_b_port = reserve_port();
    let controller_a_port = reserve_port();
    let controller_b_port = reserve_port();

    let replacements_a = HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{data_plane_a_port}"),
        ),
        (
            "0.0.0.0:46358".to_string(),
            format!("0.0.0.0:{controller_a_port}"),
        ),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{data_plane_a_port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_a_port}"),
        ),
        ("slim/node-0".to_string(), "slim/node-a".to_string()),
    ]);
    let replacements_b = HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{data_plane_b_port}"),
        ),
        (
            "0.0.0.0:46358".to_string(),
            format!("0.0.0.0:{controller_b_port}"),
        ),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{data_plane_b_port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_b_port}"),
        ),
        ("slim/node-0".to_string(), "slim/node-b".to_string()),
    ]);
    let client_replacements_a = HashMap::from([
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{data_plane_a_port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_a_port}"),
        ),
    ]);
    let client_replacements_b = HashMap::from([
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{data_plane_b_port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_b_port}"),
        ),
    ]);
    let json_replacements = HashMap::from([
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_a_port}"),
        ),
        (
            "http://127.0.0.1:46367".to_string(),
            format!("http://127.0.0.1:{data_plane_b_port}"),
        ),
    ]);

    let testdata = testdata_dir();
    let server_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("server-with-controller.yaml"),
        "server-a-config.yaml",
        &replacements_a,
    );
    let server_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("server-with-controller.yaml"),
        "server-b-config.yaml",
        &replacements_b,
    );
    let client_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-a-config.yaml",
        &client_replacements_a,
    );
    let client_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-b-config.yaml",
        &client_replacements_b,
    );
    let client_a_via = write_temp_config(
        &temp_dir,
        &testdata.join("client-a-config-data.json"),
        "client-a-config-data.json",
        &json_replacements,
    );
    let client_b_via = write_temp_config(
        &temp_dir,
        &testdata.join("client-b-config-data.json"),
        "client-b-config-data.json",
        &json_replacements,
    );

    let slim = require_slim_binary();
    let mut server_a = Some(spawn_slim(&slim, &server_a_config));
    let mut server_b = Some(spawn_slim(&slim, &server_b_config));
    let server_a_logs = ProcessLogWatcher::attach(server_a.as_mut().expect("server a"));
    let server_b_logs = ProcessLogWatcher::attach(server_b.as_mut().expect("server b"));

    thread::sleep(Duration::from_secs(2));
    server_a_logs
        .wait_contains(MSG_CONTROLPLANE_SERVER_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_b, Duration::from_secs(30));
            terminate_session(&mut server_a, Duration::from_secs(30));
            panic!("server A control plane did not start:\n{output}");
        });
    server_b_logs
        .wait_contains(MSG_CONTROLPLANE_SERVER_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_b, Duration::from_secs(30));
            terminate_session(&mut server_a, Duration::from_secs(30));
            panic!("server B control plane did not start:\n{output}");
        });

    TwoNodeRoutingSetup {
        data_plane_b_port,
        controller_a_endpoint: format!("127.0.0.1:{controller_a_port}"),
        controller_b_endpoint: format!("127.0.0.1:{controller_b_port}"),
        client_a_config,
        client_b_config,
        client_a_via,
        client_b_via,
        server_a,
        server_b,
    }
}

#[test]
#[ignore = "pending: slimctl route add removed — update to topology-based routing"]
fn delivers_messages_both_ways() {
    let slimctl = require_slimctl_binary();
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_two_node_routing();

    let mut client_b = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_b_config,
        "b",
        "a",
        None,
    ));
    let client_b_logs = ProcessLogWatcher::attach(client_b.as_mut().expect("client b"));

    let route_b = wait_for_route_with_uuid_suffix(
        &slimctl,
        &setup.controller_b_endpoint,
        "org/default/b",
        Duration::from_secs(10),
    );
    run_slimctl_node_add_route_via(
        &slimctl,
        &setup.controller_a_endpoint,
        &route_b,
        &setup.client_b_via,
        Duration::from_secs(10),
    );

    let mut client_a = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_a_config,
        "a",
        "b",
        Some("hey"),
    ));
    let client_a_logs = ProcessLogWatcher::attach(client_a.as_mut().expect("client a"));

    let route_a = wait_for_route_with_uuid_suffix(
        &slimctl,
        &setup.controller_a_endpoint,
        "org/default/a",
        Duration::from_secs(10),
    );
    run_slimctl_node_add_route_via(
        &slimctl,
        &setup.controller_b_endpoint,
        &route_a,
        &setup.client_a_via,
        Duration::from_secs(10),
    );

    client_b_logs
        .wait_contains(MSG_HELLO_FROM_A, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.server_b, Duration::from_secs(30));
            terminate_session(&mut setup.server_a, Duration::from_secs(30));
            panic!("client B did not receive message from A:\n{output}");
        });

    client_a_logs
        .wait_contains("hello from the b", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b, Duration::from_secs(2));
            terminate_session(&mut setup.server_b, Duration::from_secs(30));
            terminate_session(&mut setup.server_a, Duration::from_secs(30));
            panic!("client A did not receive message from B:\n{output}");
        });

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut client_b, Duration::from_secs(2));
    terminate_session(&mut setup.server_b, Duration::from_secs(30));
    terminate_session(&mut setup.server_a, Duration::from_secs(30));
}

#[test]
#[ignore = "pending: slimctl route add removed — update to topology-based routing"]
fn lists_routes_and_connections() {
    let slimctl = require_slimctl_binary();
    let sdk_mock = require_sdk_mock_binary();
    let mut setup = setup_two_node_routing();

    let mut client_b = Some(spawn_sdk_mock(
        &sdk_mock,
        &setup.client_b_config,
        "b",
        "a",
        None,
    ));
    let _client_b_logs = ProcessLogWatcher::attach(client_b.as_mut().expect("client b"));

    let route_b = wait_for_route_with_uuid_suffix(
        &slimctl,
        &setup.controller_b_endpoint,
        "org/default/b",
        Duration::from_secs(10),
    );
    run_slimctl_node_add_route_via(
        &slimctl,
        &setup.controller_a_endpoint,
        &route_b,
        &setup.client_b_via,
        Duration::from_secs(10),
    );

    let route_list = run_slimctl_node_output(
        &slimctl,
        &setup.controller_a_endpoint,
        &["route", "list"],
    );
    assert!(
        route_list.contains("org/default/b"),
        "route list on node A should include org/default/b:\n{route_list}"
    );

    let connection_list = run_slimctl_node_output(
        &slimctl,
        &setup.controller_a_endpoint,
        &["connection", "list"],
    );
    assert!(
        connection_list.contains(&format!(":{}", setup.data_plane_b_port)),
        "connection list on node A should include node B dataplane port:\n{connection_list}"
    );

    terminate_session(&mut client_b, Duration::from_secs(2));
    terminate_session(&mut setup.server_b, Duration::from_secs(30));
    terminate_session(&mut setup.server_a, Duration::from_secs(30));
}

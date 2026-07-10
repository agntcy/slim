//! Message routing through an external SLIM control plane.
//!
//! Exercises multi-subscriber route listing, link outline, and subscription cleanup.
//! Stays `#[ignore]` until Edge connection route cleanup is fixed upstream.

use regex::Regex;
use slim_integration_tests::{
    binaries::{
        require_control_plane_binary, require_sdk_mock_binary, require_slim_binary,
        require_slimctl_binary, workspace_root,
    },
    constants::{
        MSG_CONNECTED_TO_CONTROL_PLANE, MSG_CONTROL_PLANE_STARTED, MSG_HELLO_FROM_A,
        MSG_NOTIFY_CONTROL_PLANE_LOST_SUBSCRIPTION,
    },
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

fn spawn_control_plane(control_plane: &Path, config: &Path, db_path: &Path) -> std::process::Child {
    Command::new(control_plane)
        .arg("--config")
        .arg(config)
        .env("DATABASE_FILEPATH", db_path)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| {
            panic!(
                "failed to start control plane with config {}: {err}",
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

fn assert_output_contains(output: &[u8], needle: &str, context: &str) {
    let text = String::from_utf8_lossy(output);
    assert!(
        text.contains(needle),
        "{context}: expected output to contain {needle:?}, got:\n{text}"
    );
}

fn assert_output_not_contains(output: &[u8], needle: &str, context: &str) {
    let text = String::from_utf8_lossy(output);
    assert!(
        !text.contains(needle),
        "{context}: expected output not to contain {needle:?}, got:\n{text}"
    );
}

/// Delivers messages via the control plane, validates routes/links, and checks cleanup.
#[test]
#[ignore = "pending: fix route cleanup for Edge connections in a follow-up PR"]
fn delivers_messages_and_cleans_up_routes_via_control_plane() {
    let temp_dir = new_temp_dir("slim-integration-control-plane-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let data_plane_a_port = reserve_port();
    let data_plane_b_port = reserve_port();
    let control_plane_north_port = reserve_port();
    let control_plane_south_port = reserve_port();
    let controller_b_port = reserve_port();

    let server_a_replacements = HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{data_plane_a_port}"),
        ),
        (
            "http://127.0.0.1:50052".to_string(),
            format!("http://127.0.0.1:{control_plane_south_port}"),
        ),
    ]);
    let server_b_replacements = HashMap::from([
        (
            "0.0.0.0:46367".to_string(),
            format!("0.0.0.0:{data_plane_b_port}"),
        ),
        (
            "0.0.0.0:46368".to_string(),
            format!("0.0.0.0:{controller_b_port}"),
        ),
        (
            "http://127.0.0.1:50052".to_string(),
            format!("http://127.0.0.1:{control_plane_south_port}"),
        ),
    ]);
    let client_a_replacements = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{data_plane_a_port}"),
    )]);
    let client_b_replacements = HashMap::from([(
        "http://localhost:46357".to_string(),
        format!("http://localhost:{data_plane_b_port}"),
    )]);
    let control_plane_replacements = HashMap::from([
        (
            "0.0.0.0:50051".to_string(),
            format!("0.0.0.0:{control_plane_north_port}"),
        ),
        (
            "0.0.0.0:50052".to_string(),
            format!("0.0.0.0:{control_plane_south_port}"),
        ),
    ]);

    let testdata = testdata_dir();
    let server_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("server-a-config-cp.yaml"),
        "server-a-config-cp.yaml",
        &server_a_replacements,
    );
    let server_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("server-b-config-cp.yaml"),
        "server-b-config-cp.yaml",
        &server_b_replacements,
    );
    let client_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-a-config.yaml",
        &client_a_replacements,
    );
    let client_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-b-config.yaml",
        &client_b_replacements,
    );
    let control_plane_config = write_temp_config(
        &temp_dir,
        &testdata.join("control-plane-config.yaml"),
        "control-plane-config.yaml",
        &control_plane_replacements,
    );
    let db_path = temp_dir.join("controlplane.db");
    fs::write(&db_path, []).expect("create control plane db file");

    let slim = require_slim_binary();
    let control_plane = require_control_plane_binary();
    let sdk_mock = require_sdk_mock_binary();
    let slimctl = require_slimctl_binary();
    let cp_north_endpoint = format!("127.0.0.1:{control_plane_north_port}");

    let mut server_b = Some(spawn_slim(&slim, &server_b_config));
    let server_b_logs = ProcessLogWatcher::attach(server_b.as_mut().expect("server b"));
    thread::sleep(Duration::from_secs(8));

    let mut control_plane_session = Some(spawn_control_plane(
        &control_plane,
        &control_plane_config,
        &db_path,
    ));
    let cp_logs = ProcessLogWatcher::attach(
        control_plane_session
            .as_mut()
            .expect("control plane session"),
    );
    cp_logs
        .wait_contains(MSG_CONTROL_PLANE_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("control plane did not start:\n{output}");
        });

    server_b_logs
        .wait_contains(MSG_CONNECTED_TO_CONTROL_PLANE, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("server B did not connect to control plane:\n{output}");
        });

    let mut server_a = Some(spawn_slim(&slim, &server_a_config));
    let server_a_logs = ProcessLogWatcher::attach(server_a.as_mut().expect("server a"));
    thread::sleep(Duration::from_secs(2));

    server_a_logs
        .wait_contains(MSG_CONNECTED_TO_CONTROL_PLANE, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("server A did not connect to control plane:\n{output}");
        });

    let mut client_b1 = Some(spawn_sdk_mock(&sdk_mock, &client_b_config, "b1", "a", None));
    let client_b1_logs = ProcessLogWatcher::attach(client_b1.as_mut().expect("client b1"));

    let mut client_b2 = Some(spawn_sdk_mock(&sdk_mock, &client_b_config, "b2", "a", None));
    let _client_b2_logs = ProcessLogWatcher::attach(client_b2.as_mut().expect("client b2"));

    thread::sleep(Duration::from_secs(3));

    let mut client_a = Some(spawn_sdk_mock(
        &sdk_mock,
        &client_a_config,
        "a",
        "b1",
        Some("hey"),
    ));
    let client_a_logs = ProcessLogWatcher::attach(client_a.as_mut().expect("client a"));

    client_b1_logs
        .wait_contains(MSG_HELLO_FROM_A, Duration::from_secs(5))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b2, Duration::from_secs(2));
            terminate_session(&mut client_b1, Duration::from_secs(2));
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("client b1 did not receive message from A:\n{output}");
        });

    client_a_logs
        .wait_contains("hello from the b1", Duration::from_secs(5))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut client_b2, Duration::from_secs(2));
            terminate_session(&mut client_b1, Duration::from_secs(2));
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("client A did not receive message from b1:\n{output}");
        });

    let route_list_a = run_slimctl_controller_retry(
        &slimctl,
        &cp_north_endpoint,
        &["route", "list", "-n", "slim/a"],
        Duration::from_secs(10),
    );
    assert_output_contains(&route_list_a, "org/default/b1", "routes on slim/a");
    assert_output_contains(&route_list_a, "org/default/b2", "routes on slim/a");

    let route_list_b = run_slimctl_controller_retry(
        &slimctl,
        &cp_north_endpoint,
        &["route", "list", "-n", "slim/b"],
        Duration::from_secs(10),
    );
    assert_output_contains(&route_list_b, "org/default/a", "routes on slim/b");

    let links_out = run_slimctl_controller_retry(
        &slimctl,
        &cp_north_endpoint,
        &["link", "outline"],
        Duration::from_secs(10),
    );
    let links = String::from_utf8_lossy(&links_out);
    assert!(
        links.contains("Number of links: 1"),
        "expected one control-plane link, got:\n{links}"
    );
    let link_re = Regex::new(
        r"(?m)^\s*[0-9a-f-]{36}\s+slim/a\s+slim/b\s+http://127\.0\.0\.1:\d+\s+APPLIED\s+-\s+No\s+\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\s*$",
    )
    .expect("valid link outline regex");
    assert!(
        link_re.is_match(&links),
        "link outline did not match expected format:\n{links}"
    );

    terminate_session(&mut client_b1, Duration::from_secs(2));
    terminate_session(&mut client_b2, Duration::from_secs(2));

    server_b_logs
        .wait_contains(
            MSG_NOTIFY_CONTROL_PLANE_LOST_SUBSCRIPTION,
            Duration::from_secs(15),
        )
        .unwrap_or_else(|output| {
            terminate_session(&mut client_a, Duration::from_secs(2));
            terminate_session(&mut server_a, Duration::from_secs(30));
            terminate_session(&mut control_plane_session, Duration::from_secs(30));
            terminate_session(&mut server_b, Duration::from_secs(30));
            panic!("server B did not notify control plane about lost subscription:\n{output}");
        });

    thread::sleep(Duration::from_secs(6));

    let route_list_a_after = Command::new(&slimctl)
        .arg("controller")
        .arg("route")
        .arg("list")
        .arg("-n")
        .arg("slim/a")
        .arg("--server")
        .arg(&cp_north_endpoint)
        .output()
        .expect("slimctl controller route list for slim/a");
    assert!(
        route_list_a_after.status.success(),
        "slimctl route list for slim/a failed:\n{}",
        String::from_utf8_lossy(&route_list_a_after.stderr)
    );

    let route_list_b_after = Command::new(&slimctl)
        .arg("controller")
        .arg("route")
        .arg("list")
        .arg("-n")
        .arg("slim/b")
        .arg("--server")
        .arg(&cp_north_endpoint)
        .output()
        .expect("slimctl controller route list for slim/b");
    assert!(
        route_list_b_after.status.success(),
        "slimctl route list for slim/b failed:\n{}",
        String::from_utf8_lossy(&route_list_b_after.stderr)
    );

    let mut route_list_b_combined = route_list_b_after.stdout.clone();
    route_list_b_combined.extend_from_slice(&route_list_b_after.stderr);
    assert_output_contains(
        &route_list_b_combined,
        "org/default/a",
        "routes on slim/b after cleanup",
    );
    assert_output_not_contains(
        &route_list_b_combined,
        "org/default/b1",
        "routes on slim/b after cleanup",
    );
    assert_output_not_contains(
        &route_list_b_combined,
        "org/default/b2",
        "routes on slim/b after cleanup",
    );

    terminate_session(&mut client_a, Duration::from_secs(2));
    terminate_session(&mut server_a, Duration::from_secs(30));
    terminate_session(&mut control_plane_session, Duration::from_secs(30));
    terminate_session(&mut server_b, Duration::from_secs(30));
}

//! Subscription ACK and message delivery through a SLIM relay.
//!
//! Verifies that sdk-mock applications subscribe, receive ACKs on the remote
//! path, and exchange messages end-to-end through a central relay node.

use slim_integration_tests::{
    binaries::{require_sdk_mock_binary, require_slim_binary, workspace_root},
    constants::{MSG_HELLO_FROM_A, MSG_QUEUEING_REPLY, MSG_SUBSCRIPTION_REMOTE_ACK},
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

fn relay_port_replacements(relay_port: u16) -> HashMap<String, String> {
    HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{relay_port}"),
        ),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{relay_port}"),
        ),
    ])
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RelayConfigs {
    relay: PathBuf,
    app_a: PathBuf,
    app_b: PathBuf,
}

fn setup_relay_configs(temp_dir: &Path, relay_port: u16) -> RelayConfigs {
    let replacements = relay_port_replacements(relay_port);
    let testdata = testdata_dir();

    RelayConfigs {
        relay: write_temp_config(
            temp_dir,
            &testdata.join("server.yaml"),
            "relay.yaml",
            &replacements,
        ),
        app_a: write_temp_config(
            temp_dir,
            &testdata.join("client.yaml"),
            "app-a.yaml",
            &replacements,
        ),
        app_b: write_temp_config(
            temp_dir,
            &testdata.join("client.yaml"),
            "app-b.yaml",
            &replacements,
        ),
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

fn terminate_all(sessions: &mut [Option<std::process::Child>]) {
    for session in sessions {
        terminate_session(session, Duration::from_secs(5));
    }
}

fn wait_relay_started(
    relay: &mut Option<std::process::Child>,
    relay_logs: &ProcessLogWatcher,
) {
    relay_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_all(&mut [relay.take()]);
            panic!("relay did not start:\n{output}");
        });
}

fn assert_message_delivery(
    relay: &mut Option<std::process::Child>,
    app_a: &mut Option<std::process::Child>,
    app_b: &mut Option<std::process::Child>,
    app_a_logs: &ProcessLogWatcher,
    app_b_logs: &ProcessLogWatcher,
) {
    app_a_logs
        .wait_contains(MSG_QUEUEING_REPLY, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_all(&mut [app_b.take(), app_a.take(), relay.take()]);
            panic!("app A did not queue a reply:\n{output}");
        });

    app_b_logs
        .wait_contains(MSG_HELLO_FROM_A, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_all(&mut [app_b.take(), app_a.take(), relay.take()]);
            panic!("app B did not receive reply from app A:\n{output}");
        });
}

fn start_subscriber(
    sdk_mock: &Path,
    config: &Path,
    ack_needle: &str,
    relay: &mut Option<std::process::Child>,
) -> (Option<std::process::Child>, ProcessLogWatcher) {
    let mut app_a = Some(spawn_sdk_mock(sdk_mock, config, "a", "b", None));
    let app_a_logs = ProcessLogWatcher::attach(app_a.as_mut().expect("app a session"));
    app_a_logs
        .wait_contains(ack_needle, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_all(&mut [app_a.take(), relay.take()]);
            panic!("app A subscription setup failed:\n{output}");
        });
    (app_a, app_a_logs)
}

fn start_sender(
    sdk_mock: &Path,
    config: &Path,
) -> (Option<std::process::Child>, ProcessLogWatcher) {
    let mut app_b = Some(spawn_sdk_mock(sdk_mock, config, "b", "a", Some("hey")));
    let app_b_logs = ProcessLogWatcher::attach(app_b.as_mut().expect("app b session"));
    (app_b, app_b_logs)
}

/// Current relay + current apps on the remote ACK subscription path.
fn run_new_relay_remote_ack_message_delivery() {
    let temp_dir = new_temp_dir("slim-integration-sub-ack-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let relay_port = reserve_port();
    let configs = setup_relay_configs(&temp_dir, relay_port);

    let slim = require_slim_binary();
    let sdk_mock = require_sdk_mock_binary();

    let mut relay = Some(spawn_slim(&slim, &configs.relay));
    let relay_logs = ProcessLogWatcher::attach(relay.as_mut().expect("relay session"));
    wait_relay_started(&mut relay, &relay_logs);

    let (mut app_a, app_a_logs) = start_subscriber(
        &sdk_mock,
        &configs.app_a,
        MSG_SUBSCRIPTION_REMOTE_ACK,
        &mut relay,
    );
    let (mut app_b, app_b_logs) = start_sender(&sdk_mock, &configs.app_b);

    assert_message_delivery(&mut relay, &mut app_a, &mut app_b, &app_a_logs, &app_b_logs);
    terminate_all(&mut [app_b, app_a, relay]);
}

#[test]
fn upgrades_subscription_to_remote_ack_path_and_delivers_messages() {
    run_new_relay_remote_ack_message_delivery();
}

#[test]
fn routes_messages_between_two_apps_via_new_relay() {
    run_new_relay_remote_ack_message_delivery();
}

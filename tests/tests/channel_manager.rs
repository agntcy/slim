//! Channel lifecycle and group messaging through the channel manager.
//!
//! Exercises create/add/remove/delete flows via `slimctl cm` while three client
//! apps join a channel and exchange messages.

use slim_integration_tests::{
    binaries::{
        require_channel_manager_binary, require_client_binary, require_slim_binary,
        require_slimctl_binary, workspace_root,
    },
    constants::{
        MSG_CLIENT_READY, MSG_SESSION_CLOSED, MSG_SESSION_HANDLER_TASK_STARTED,
        MSG_TEST_CLIENT_C_MESSAGE,
    },
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const CHANNEL_NAME: &str = "org/default/test-channel";
const GROUP_SECRET: &str = "group-abcdef-12345678901234567890";

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn channel_manager_port_replacements(
    data_plane_port: u16,
    channel_manager_port: u16,
) -> HashMap<String, String> {
    HashMap::from([
        (
            "0.0.0.0:46357".to_string(),
            format!("0.0.0.0:{data_plane_port}"),
        ),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{data_plane_port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{data_plane_port}"),
        ),
        (
            "0.0.0.0:10356".to_string(),
            format!("0.0.0.0:{channel_manager_port}"),
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

fn spawn_channel_manager(channel_manager: &Path, config: &Path) -> std::process::Child {
    Command::new(channel_manager)
        .arg("--config-file")
        .arg(config)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| {
            panic!(
                "failed to start channel-manager with config {}: {err}",
                config.display()
            )
        })
}

fn spawn_client(
    client: &Path,
    config: &Path,
    local_name: &str,
    message: Option<&str>,
) -> std::process::Child {
    let mut cmd = Command::new(client);
    cmd.arg("--config")
        .arg(config)
        .arg("--local-name")
        .arg(local_name)
        .arg("--secret")
        .arg(GROUP_SECRET)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(message) = message {
        cmd.arg("--message").arg(message);
    }

    cmd.spawn().unwrap_or_else(|err| {
        panic!(
            "failed to start client {} with config {}: {err}",
            local_name,
            config.display()
        )
    })
}

fn run_slimctl_cm(slimctl: &Path, cm_endpoint: &str, args: &[&str]) -> Vec<u8> {
    run_combined_output_with_retry(Duration::from_secs(10), || {
        let mut cmd = Command::new(slimctl);
        cmd.arg("cm");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg("--server").arg(cm_endpoint);
        cmd
    })
}

fn assert_output_contains(output: &[u8], needle: &str, context: &str) {
    let text = String::from_utf8_lossy(output);
    assert!(
        text.contains(needle),
        "{context}: expected output to contain {needle:?}, got:\n{text}"
    );
}

/// SLIM node creates a channel, adds participants, verifies messaging, then tears down.
#[test]
fn slim_node_manages_channel_participants_and_messaging() {
    let temp_dir = new_temp_dir("slim-integration-gm-");
    let _cleanup = TempDirCleanup(temp_dir.clone());

    let data_plane_port = reserve_port();
    let channel_manager_port = reserve_port();
    let replacements = channel_manager_port_replacements(data_plane_port, channel_manager_port);
    let testdata = testdata_dir();

    let server_config = write_temp_config(
        &temp_dir,
        &testdata.join("server.yaml"),
        "server-a-config.yaml",
        &replacements,
    );
    let client_a_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-a-config.yaml",
        &replacements,
    );
    let client_b_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-b-config.yaml",
        &replacements,
    );
    let client_c_config = write_temp_config(
        &temp_dir,
        &testdata.join("client.yaml"),
        "client-c-config.yaml",
        &replacements,
    );
    let channel_manager_config = write_temp_config(
        &temp_dir,
        &testdata.join("channel-manager-config.yaml"),
        "channel-manager-config.yaml",
        &replacements,
    );

    let slim = require_slim_binary();
    let channel_manager = require_channel_manager_binary();
    let client = require_client_binary();
    let slimctl = require_slimctl_binary();
    let cm_endpoint = format!("127.0.0.1:{channel_manager_port}");

    let mut slim_session = Some(spawn_slim(&slim, &server_config));
    let slim_logs = ProcessLogWatcher::attach(slim_session.as_mut().expect("slim session"));
    slim_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("SLIM node did not start dataplane:\n{output}");
        });

    let mut channel_manager_session = Some(spawn_channel_manager(
        &channel_manager,
        &channel_manager_config,
    ));
    let cm_logs = ProcessLogWatcher::attach(
        channel_manager_session
            .as_mut()
            .expect("channel manager session"),
    );
    cm_logs
        .wait_contains("Starting gRPC server", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("channel manager did not start gRPC server:\n{output}");
        });

    let mut client_a_session = Some(spawn_client(
        &client,
        &client_a_config,
        "org/default/a",
        None,
    ));
    let client_a_logs =
        ProcessLogWatcher::attach(client_a_session.as_mut().expect("client a session"));

    let mut client_b_session = Some(spawn_client(
        &client,
        &client_b_config,
        "org/default/b",
        None,
    ));
    let client_b_logs =
        ProcessLogWatcher::attach(client_b_session.as_mut().expect("client b session"));

    let mut client_c_session = Some(spawn_client(
        &client,
        &client_c_config,
        "org/default/c",
        Some(MSG_TEST_CLIENT_C_MESSAGE),
    ));
    let client_c_logs =
        ProcessLogWatcher::attach(client_c_session.as_mut().expect("client c session"));

    for (label, logs) in [
        ("client A", &client_a_logs),
        ("client B", &client_b_logs),
        ("client C", &client_c_logs),
    ] {
        logs.wait_contains(MSG_CLIENT_READY, Duration::from_secs(15))
            .unwrap_or_else(|output| {
                terminate_session(&mut client_c_session, Duration::from_secs(2));
                terminate_session(&mut client_b_session, Duration::from_secs(2));
                terminate_session(&mut client_a_session, Duration::from_secs(2));
                terminate_session(&mut channel_manager_session, Duration::from_secs(30));
                terminate_session(&mut slim_session, Duration::from_secs(30));
                panic!("{label} did not connect and subscribe:\n{output}");
            });
    }

    let create_output = run_slimctl_cm(&slimctl, &cm_endpoint, &["create-channel", CHANNEL_NAME]);
    assert!(!create_output.is_empty());
    assert_output_contains(&create_output, "created successfully", "create channel");

    let participant_a = "org/default/a";
    let participant_b = "org/default/b";
    let participant_c = "org/default/c";

    let add_a_output = run_slimctl_cm(
        &slimctl,
        &cm_endpoint,
        &["add-participant", CHANNEL_NAME, participant_a],
    );
    assert!(!add_a_output.is_empty());
    assert_output_contains(&add_a_output, "added to channel", "add participant a");
    client_a_logs
        .wait_contains(MSG_SESSION_HANDLER_TASK_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client A did not start session handler:\n{output}");
        });

    let add_b_output = run_slimctl_cm(
        &slimctl,
        &cm_endpoint,
        &["add-participant", CHANNEL_NAME, participant_b],
    );
    assert!(!add_b_output.is_empty());
    assert_output_contains(&add_b_output, "added to channel", "add participant b");
    client_b_logs
        .wait_contains(MSG_SESSION_HANDLER_TASK_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client B did not start session handler:\n{output}");
        });

    let add_c_output = run_slimctl_cm(
        &slimctl,
        &cm_endpoint,
        &["add-participant", CHANNEL_NAME, participant_c],
    );
    assert!(!add_c_output.is_empty());
    assert_output_contains(&add_c_output, "added to channel", "add participant c");
    client_c_logs
        .wait_contains(MSG_SESSION_HANDLER_TASK_STARTED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client C did not start session handler:\n{output}");
        });

    client_a_logs
        .wait_contains(MSG_TEST_CLIENT_C_MESSAGE, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client A did not receive message from C:\n{output}");
        });

    client_b_logs
        .wait_contains(MSG_TEST_CLIENT_C_MESSAGE, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client B did not receive message from C:\n{output}");
        });

    let delete_participant_output = run_slimctl_cm(
        &slimctl,
        &cm_endpoint,
        &["delete-participant", CHANNEL_NAME, participant_c],
    );
    assert!(!delete_participant_output.is_empty());
    client_c_logs
        .wait_contains(MSG_SESSION_CLOSED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client C session was not closed:\n{output}");
        });

    let delete_channel_output =
        run_slimctl_cm(&slimctl, &cm_endpoint, &["delete-channel", CHANNEL_NAME]);
    assert!(!delete_channel_output.is_empty());
    client_a_logs
        .wait_contains(MSG_SESSION_CLOSED, Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut client_c_session, Duration::from_secs(2));
            terminate_session(&mut client_b_session, Duration::from_secs(2));
            terminate_session(&mut client_a_session, Duration::from_secs(2));
            terminate_session(&mut channel_manager_session, Duration::from_secs(30));
            terminate_session(&mut slim_session, Duration::from_secs(30));
            panic!("client A session was not closed after channel delete:\n{output}");
        });

    terminate_session(&mut client_c_session, Duration::from_secs(2));
    terminate_session(&mut client_b_session, Duration::from_secs(2));
    terminate_session(&mut client_a_session, Duration::from_secs(2));
    terminate_session(&mut channel_manager_session, Duration::from_secs(30));
    terminate_session(&mut slim_session, Duration::from_secs(30));
}

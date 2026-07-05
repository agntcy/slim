//! Channel manager invite failures when the participant is not connected.
//!
//! Verifies that adding a nonexistent participant to a channel returns an error
//! from `slimctl cm add-participant`.

use slim_integration_tests::{
    binaries::{
        require_channel_manager_binary, require_slim_binary, require_slimctl_binary,
        workspace_root,
    },
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const CHANNEL_NAME: &str = "org/default/test-channel-timeout";

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

/// Adding a participant that is not connected fails with an invite error.
#[test]
fn add_nonexistent_participant_fails() {
    let temp_dir = new_temp_dir("slim-integration-gm-timeout-");
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
    let channel_manager_config = write_temp_config(
        &temp_dir,
        &testdata.join("channel-manager-config.yaml"),
        "channel-manager-config.yaml",
        &replacements,
    );

    let slim = require_slim_binary();
    let channel_manager = require_channel_manager_binary();
    let slimctl = require_slimctl_binary();
    let cm_endpoint = format!("127.0.0.1:{channel_manager_port}");

    let mut slim_session = Some(spawn_slim(&slim, &server_config));
    thread::sleep(Duration::from_secs(2));

    let mut channel_manager_session =
        Some(spawn_channel_manager(&channel_manager, &channel_manager_config));
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

    let create_output = run_slimctl_cm(
        &slimctl,
        &cm_endpoint,
        &["create-channel", CHANNEL_NAME],
    );
    assert!(!create_output.is_empty());

    let add_output = Command::new(&slimctl)
        .arg("cm")
        .arg("add-participant")
        .arg(CHANNEL_NAME)
        .arg("org/default/a")
        .arg("--server")
        .arg(&cm_endpoint)
        .output()
        .expect("failed to run slimctl add-participant");

    thread::sleep(Duration::from_secs(2));

    assert!(
        !add_output.status.success(),
        "expected add-participant to fail, got success:\n{}",
        String::from_utf8_lossy(&add_output.stdout)
    );

    let combined = {
        let mut buf = add_output.stderr.clone();
        buf.extend_from_slice(&add_output.stdout);
        String::from_utf8_lossy(&buf).into_owned()
    };
    assert!(
        combined.contains("failed to invite participant"),
        "expected invite failure in output, got:\n{combined}"
    );

    terminate_session(&mut channel_manager_session, Duration::from_secs(30));
    terminate_session(&mut slim_session, Duration::from_secs(30));
}

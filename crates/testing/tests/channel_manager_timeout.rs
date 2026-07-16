//! Channel manager invite failures when the participant is not connected.
//!
//! Verifies that adding a nonexistent participant to a channel returns an error
//! from `slimctl cm add-participant`.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use slim_testing::{
    binaries::{
        require_channel_manager_binary, require_slim_binary, require_slimctl_binary, workspace_root,
    },
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

    run_slimctl_cm(&slimctl, &cm_endpoint, &["create-channel", CHANNEL_NAME])
        .assert()
        .success();

    // Inviting a participant that never connected must fail; the invite error is
    // reported on stderr by slimctl's anyhow error handler.
    Command::new(&slimctl)
        .arg("cm")
        .arg("add-participant")
        .arg(CHANNEL_NAME)
        .arg("org/default/a")
        .arg("--server")
        .arg(&cm_endpoint)
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to invite participant"));

    terminate_session(&mut channel_manager_session, Duration::from_secs(30));
    terminate_session(&mut slim_session, Duration::from_secs(30));
}

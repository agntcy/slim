//! Helpers for spawning SLIM test processes (data-plane nodes, the external
//! control plane, and `sdk-mock` apps) with a consistent working directory and
//! piped stdout/stderr so their logs can be inspected via [`ProcessLogWatcher`].
//!
//! [`ProcessLogWatcher`]: super::ProcessLogWatcher

use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::binaries::workspace_root;

/// Spawn a `slim` data-plane node with the given config file.
pub fn spawn_slim(slim: &Path, config: &Path) -> Child {
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

/// Spawn a `slim` data-plane node with additional environment variables set.
pub fn spawn_slim_with_env(slim: &Path, config: &Path, env: &[(&str, &str)]) -> Child {
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

/// Spawn the external `slim-control-plane` with the given config and DB path.
pub fn spawn_control_plane(control_plane: &Path, config: &Path, db_path: &Path) -> Child {
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

/// Spawn an `sdk-mock` app with the given local/remote names and an optional
/// message payload (passed via `--message` only when `Some`).
pub fn spawn_sdk_mock(
    sdk_mock: &Path,
    config: &Path,
    local_name: &str,
    remote_name: &str,
    message: Option<&str>,
) -> Child {
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

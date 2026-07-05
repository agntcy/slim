//! P2P and group session compatibility across current and legacy SLIM binaries.
//!
//! Exercises sender-app/receiver-app combinations against current or legacy `slim`
//! nodes. All scenarios stay `#[ignore]` until `.dist/bin/*-legacy` exists.

use slim_integration_tests::{
    binaries::{
        require_legacy_receiver_app_binary, require_legacy_sender_app_binary,
        require_legacy_slim_binary, require_receiver_app_binary, require_sender_app_binary,
        require_slim_binary, workspace_root,
    },
    constants::{MSG_ALL_PARTICIPANTS_REPLIED, MSG_WAITING_INCOMING_SESSION},
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const SHARED_SECRET: &str = "a-very-long-shared-secret-abcdef1234567890";

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

fn spawn_receiver(receiver: &Path, local: &str, slim_endpoint: &str) -> std::process::Child {
    Command::new(receiver)
        .arg("--local")
        .arg(local)
        .arg("--shared-secret")
        .arg(SHARED_SECRET)
        .arg("--slim")
        .arg(slim_endpoint)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to start receiver {local}: {err}"))
}

fn spawn_p2p_sender(sender: &Path, local: &str, participant: &str, slim_endpoint: &str) -> std::process::Child {
    Command::new(sender)
        .arg("--local")
        .arg(local)
        .arg("--shared-secret")
        .arg(SHARED_SECRET)
        .arg("--slim")
        .arg(slim_endpoint)
        .arg("--session-type")
        .arg("p2p")
        .arg("--participants")
        .arg(participant)
        .current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to start p2p sender {local}: {err}"))
}

fn spawn_group_sender(
    sender: &Path,
    local: &str,
    participants: &[&str],
    slim_endpoint: &str,
) -> std::process::Child {
    let mut cmd = Command::new(sender);
    cmd.arg("--local")
        .arg(local)
        .arg("--shared-secret")
        .arg(SHARED_SECRET)
        .arg("--slim")
        .arg(slim_endpoint)
        .arg("--session-type")
        .arg("group")
        .arg("--participants");
    for participant in participants {
        cmd.arg(participant);
    }
    cmd.current_dir(workspace_root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to start group sender {local}: {err}"))
}

fn setup_slim_server(temp_dir: &Path) -> (PathBuf, u16) {
    let slim_port = reserve_port();
    let replacements = HashMap::from([(
        "0.0.0.0:46357".to_string(),
        format!("0.0.0.0:{slim_port}"),
    )]);
    let config = write_temp_config(
        temp_dir,
        &testdata_dir().join("server.yaml"),
        "slim.yaml",
        &replacements,
    );
    (config, slim_port)
}

fn run_p2p_session(slim: &Path, sender: &Path, receiver: &Path) {
    let temp_dir = new_temp_dir("slim-integration-backward-compat-");
    let _cleanup = TempDirCleanup(temp_dir.clone());
    let (slim_config, slim_port) = setup_slim_server(&temp_dir);
    let slim_endpoint = format!("http://localhost:{slim_port}");

    let mut slim_session = Some(spawn_slim(slim, &slim_config));
    let slim_logs = ProcessLogWatcher::attach(slim_session.as_mut().expect("slim session"));
    slim_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("slim did not start:\n{output}");
        });

    let mut receiver_session = Some(spawn_receiver(
        receiver,
        "agntcy/ns/alice",
        &slim_endpoint,
    ));
    let receiver_logs =
        ProcessLogWatcher::attach(receiver_session.as_mut().expect("receiver session"));
    receiver_logs
        .wait_contains(MSG_WAITING_INCOMING_SESSION, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("receiver did not enter wait state:\n{output}");
        });

    let mut sender_session = Some(spawn_p2p_sender(
        sender,
        "agntcy/ns/bob",
        "agntcy/ns/alice",
        &slim_endpoint,
    ));
    let sender_logs = ProcessLogWatcher::attach(sender_session.as_mut().expect("sender session"));

    sender_logs
        .wait_contains("Session", Duration::from_secs(15))
        .and_then(|_| sender_logs.wait_contains(" established", Duration::from_secs(1)))
        .unwrap_or_else(|output| {
            terminate_session(&mut sender_session, Duration::from_secs(5));
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("sender did not establish session:\n{output}");
        });

    sender_logs
        .wait_contains("Sending: Message 1", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut sender_session, Duration::from_secs(5));
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("sender did not send first message:\n{output}");
        });

    sender_logs
        .wait_contains("Reply from agntcy/ns/alice", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut sender_session, Duration::from_secs(5));
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("sender did not receive reply from alice:\n{output}");
        });

    sender_logs
        .wait_contains("Sending: Message 10", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut sender_session, Duration::from_secs(5));
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("sender did not send final message:\n{output}");
        });

    sender_logs
        .wait_contains(MSG_ALL_PARTICIPANTS_REPLIED, Duration::from_secs(30))
        .unwrap_or_else(|output| {
            terminate_session(&mut sender_session, Duration::from_secs(5));
            terminate_session(&mut receiver_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("sender did not complete with all replies:\n{output}");
        });

    terminate_session(&mut sender_session, Duration::from_secs(5));
    terminate_session(&mut receiver_session, Duration::from_secs(5));
    terminate_session(&mut slim_session, Duration::from_secs(5));
}

fn run_group_session(
    slim: &Path,
    moderator: &Path,
    participant1: &Path,
    participant2: &Path,
) {
    let temp_dir = new_temp_dir("slim-integration-backward-compat-");
    let _cleanup = TempDirCleanup(temp_dir.clone());
    let (slim_config, slim_port) = setup_slim_server(&temp_dir);
    let slim_endpoint = format!("http://localhost:{slim_port}");

    let mut slim_session = Some(spawn_slim(slim, &slim_config));
    let slim_logs = ProcessLogWatcher::attach(slim_session.as_mut().expect("slim session"));
    slim_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("slim did not start:\n{output}");
        });

    let mut alice_session = Some(spawn_receiver(
        participant1,
        "agntcy/ns/alice",
        &slim_endpoint,
    ));
    let alice_logs = ProcessLogWatcher::attach(alice_session.as_mut().expect("alice session"));
    alice_logs
        .wait_contains(MSG_WAITING_INCOMING_SESSION, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("alice did not enter wait state:\n{output}");
        });

    let mut charlie_session = Some(spawn_receiver(
        participant2,
        "agntcy/ns/charlie",
        &slim_endpoint,
    ));
    let charlie_logs =
        ProcessLogWatcher::attach(charlie_session.as_mut().expect("charlie session"));
    charlie_logs
        .wait_contains(MSG_WAITING_INCOMING_SESSION, Duration::from_secs(10))
        .unwrap_or_else(|output| {
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("charlie did not enter wait state:\n{output}");
        });

    let mut moderator_session = Some(spawn_group_sender(
        moderator,
        "agntcy/ns/bob",
        &["agntcy/ns/alice", "agntcy/ns/charlie"],
        &slim_endpoint,
    ));
    let moderator_logs =
        ProcessLogWatcher::attach(moderator_session.as_mut().expect("moderator session"));

    moderator_logs
        .wait_contains("Session", Duration::from_secs(15))
        .and_then(|_| moderator_logs.wait_contains(" established", Duration::from_secs(1)))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("moderator did not establish session:\n{output}");
        });

    moderator_logs
        .wait_contains("agntcy/ns/alice", Duration::from_secs(15))
        .and_then(|_| moderator_logs.wait_contains("joined session", Duration::from_secs(1)))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("alice did not join group session:\n{output}");
        });

    moderator_logs
        .wait_contains("agntcy/ns/charlie", Duration::from_secs(15))
        .and_then(|_| moderator_logs.wait_contains("joined session", Duration::from_secs(1)))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("charlie did not join group session:\n{output}");
        });

    moderator_logs
        .wait_contains("Sending: Message 1", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("moderator did not send first message:\n{output}");
        });

    moderator_logs
        .wait_contains("Reply from agntcy/ns/alice", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("moderator did not receive reply from alice:\n{output}");
        });

    moderator_logs
        .wait_contains("Reply from agntcy/ns/charlie", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("moderator did not receive reply from charlie:\n{output}");
        });

    moderator_logs
        .wait_contains(MSG_ALL_PARTICIPANTS_REPLIED, Duration::from_secs(30))
        .unwrap_or_else(|output| {
            terminate_session(&mut moderator_session, Duration::from_secs(5));
            terminate_session(&mut charlie_session, Duration::from_secs(5));
            terminate_session(&mut alice_session, Duration::from_secs(5));
            terminate_session(&mut slim_session, Duration::from_secs(5));
            panic!("moderator did not complete with all replies:\n{output}");
        });

    terminate_session(&mut moderator_session, Duration::from_secs(5));
    terminate_session(&mut charlie_session, Duration::from_secs(5));
    terminate_session(&mut alice_session, Duration::from_secs(5));
    terminate_session(&mut slim_session, Duration::from_secs(5));
}

// ── P2P sessions ─────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn current_slim_latest_sender_legacy_receiver_p2p() {
    run_p2p_session(
        &require_slim_binary(),
        &require_sender_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn current_slim_legacy_sender_latest_receiver_p2p() {
    run_p2p_session(
        &require_slim_binary(),
        &require_legacy_sender_app_binary(),
        &require_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn current_slim_legacy_sender_legacy_receiver_p2p() {
    run_p2p_session(
        &require_slim_binary(),
        &require_legacy_sender_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn legacy_slim_latest_sender_legacy_receiver_p2p() {
    run_p2p_session(
        &require_legacy_slim_binary(),
        &require_sender_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn legacy_slim_legacy_sender_latest_receiver_p2p() {
    run_p2p_session(
        &require_legacy_slim_binary(),
        &require_legacy_sender_app_binary(),
        &require_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn legacy_slim_latest_sender_latest_receiver_p2p() {
    run_p2p_session(
        &require_legacy_slim_binary(),
        &require_sender_app_binary(),
        &require_receiver_app_binary(),
    );
}

// ── Group sessions ───────────────────────────────────────────────────────────

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn current_slim_latest_moderator_mixed_participants_group() {
    run_group_session(
        &require_slim_binary(),
        &require_sender_app_binary(),
        &require_receiver_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn current_slim_legacy_moderator_mixed_participants_group() {
    run_group_session(
        &require_slim_binary(),
        &require_legacy_sender_app_binary(),
        &require_receiver_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn legacy_slim_latest_moderator_mixed_participants_group() {
    run_group_session(
        &require_legacy_slim_binary(),
        &require_sender_app_binary(),
        &require_receiver_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

#[test]
#[ignore = "requires legacy binaries — build with: task -d tests tests:build-legacy-binaries"]
fn legacy_slim_legacy_moderator_mixed_participants_group() {
    run_group_session(
        &require_legacy_slim_binary(),
        &require_legacy_sender_app_binary(),
        &require_receiver_app_binary(),
        &require_legacy_receiver_app_binary(),
    );
}

//! Server and client SLIM nodes connect for each curated config pair under `config/`.
//!
//! Scans `config/<dir>/` for matching `server-config.yaml` and `client-config.yaml`
//! files, rewrites ports to avoid collisions, and verifies startup plus dataplane
//! connection logs.

use slim_integration_tests::{
    binaries::{require_slim_binary, workspace_root},
    helpers::*,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

struct ConfigCase {
    server_path: PathBuf,
    client_path: PathBuf,
}

fn abs_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn port_replacements(port: u16) -> HashMap<String, String> {
    HashMap::from([
        ("0.0.0.0:46357".to_string(), format!("0.0.0.0:{port}")),
        (
            "http://localhost:46357".to_string(),
            format!("http://localhost:{port}"),
        ),
        (
            "http://127.0.0.1:46357".to_string(),
            format!("http://127.0.0.1:{port}"),
        ),
        (
            "https://localhost:46357".to_string(),
            format!("https://localhost:{port}"),
        ),
        (
            "https://127.0.0.1:46357".to_string(),
            format!("https://127.0.0.1:{port}"),
        ),
        (
            "ws://localhost:46357".to_string(),
            format!("ws://localhost:{port}"),
        ),
        (
            "ws://127.0.0.1:46357".to_string(),
            format!("ws://127.0.0.1:{port}"),
        ),
        (
            "wss://localhost:46357".to_string(),
            format!("wss://localhost:{port}"),
        ),
        (
            "wss://127.0.0.1:46357".to_string(),
            format!("wss://127.0.0.1:{port}"),
        ),
    ])
}

fn resolve_config_cases() -> Vec<ConfigCase> {
    let config_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config");
    let entries = match fs::read_dir(&config_root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut seen = HashMap::<PathBuf, ()>::new();
    let mut out = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || seen.contains_key(&path) {
            continue;
        }
        seen.insert(path.clone(), ());

        let server_cfg = path.join("server-config.yaml");
        let client_cfg = path.join("client-config.yaml");

        if server_cfg.is_file() && client_cfg.is_file() {
            out.push(ConfigCase {
                server_path: abs_path(&server_cfg),
                client_path: abs_path(&client_cfg),
            });
        }
    }

    out
}

/// Start server + client SLIM and wait for connection logs.
fn run_config_startup_case(case: &ConfigCase) {
    let config_dir = case
        .server_path
        .parent()
        .expect("server config must have a parent directory");

    eprintln!(
        "Testing config dir {} — server: {} client: {}",
        config_dir.display(),
        case.server_path.display(),
        case.client_path.display()
    );

    assert!(
        case.server_path.is_file(),
        "server configuration file not found: {}",
        case.server_path.display()
    );
    assert!(
        case.client_path.is_file(),
        "client configuration file not found: {}",
        case.client_path.display()
    );

    let data_plane_port = reserve_port();
    let replacements = port_replacements(data_plane_port);

    let server_config =
        write_temp_config_near_source(&case.server_path, "tmp-server-config-", &replacements);
    let client_config =
        write_temp_config_near_source(&case.client_path, "tmp-client-config-", &replacements);

    let _temp_configs = TempConfigCleanup {
        paths: vec![server_config.clone(), client_config.clone()],
    };

    let repo_root = workspace_root();
    let slim = require_slim_binary();

    let mut server_session = Some(
        Command::new(&slim)
            .arg("--config")
            .arg(&server_config)
            .current_dir(&repo_root)
            .env("PASSWORD", "password")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "failed to start server SLIM with config {}: {e}",
                    case.server_path.display()
                )
            }),
    );

    let server_result = wait_for_log(
        server_session.as_mut().expect("server session"),
        "dataplane server started",
        Duration::from_secs(15),
    );
    server_result.unwrap_or_else(|output| {
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!(
            "server did not log 'dataplane server started' for {}:\n{output}",
            case.server_path.display()
        )
    });

    let mut client_session = Some(
        Command::new(&slim)
            .arg("--config")
            .arg(&client_config)
            .current_dir(&repo_root)
            .env("PASSWORD", "password")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "failed to start client SLIM with config {}: {e}",
                    case.client_path.display()
                )
            }),
    );

    let client_result = wait_for_log(
        client_session.as_mut().expect("client session"),
        "client connected",
        Duration::from_secs(15),
    );
    client_result.unwrap_or_else(|output| {
        terminate_session(&mut client_session, Duration::from_secs(5));
        terminate_session(&mut server_session, Duration::from_secs(5));
        panic!(
            "client did not log 'client connected' for {}:\n{output}",
            case.client_path.display()
        )
    });

    terminate_session(&mut client_session, Duration::from_secs(5));
    terminate_session(&mut server_session, Duration::from_secs(5));
}

struct TempConfigCleanup {
    paths: Vec<PathBuf>,
}

impl Drop for TempConfigCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// For each discovered config pair, start server then client and wait for connection logs.
#[test]
fn server_and_client_start_and_connect_for_all_configs() {
    let cases = resolve_config_cases();
    assert!(
        !cases.is_empty(),
        "no config/<dir>/{{server-config,client-config}}.yaml pairs found"
    );

    for case in &cases {
        run_config_startup_case(case);
    }
}

#[test]
fn resolve_config_cases_finds_server_and_client_pairs() {
    let cases = resolve_config_cases();
    assert!(!cases.is_empty());

    for case in &cases {
        assert!(case.server_path.is_file());
        assert!(case.client_path.is_file());
    }
}

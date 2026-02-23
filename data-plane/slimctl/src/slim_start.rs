use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
#[cfg(not(test))]
use anyhow::bail;

#[cfg(not(test))]
const DEFAULT_ENDPOINT: &str = "127.0.0.1:8080";

#[cfg(not(test))]
pub fn start(config: String, endpoint: String) -> Result<()> {
    let slim_bin = find_slim_binary()?;

    let mut temp_config: Option<PathBuf> = None;
    let config_path = if config.trim().is_empty() {
        let generated = create_default_temp_config()?;
        temp_config = Some(generated.clone());
        generated
    } else {
        PathBuf::from(config)
    };

    let endpoint_value = if endpoint.trim().is_empty() {
        DEFAULT_ENDPOINT.to_string()
    } else {
        endpoint
    };

    let mut command = std::process::Command::new(&slim_bin);
    command.arg("--config").arg(&config_path);
    command.env("SLIM_ENDPOINT", endpoint_value);
    command.stdin(std::process::Stdio::inherit());
    command.stdout(std::process::Stdio::inherit());
    command.stderr(std::process::Stdio::inherit());

    let status = command
        .status()
        .with_context(|| format!("failed to launch slim binary at {}", slim_bin.display()))?;

    if let Some(path) = temp_config {
        let _ = std::fs::remove_file(path);
    }

    if !status.success() {
        bail!("slim exited with status {status}");
    }

    Ok(())
}

#[cfg(test)]
pub fn start(_config: String, _endpoint: String) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn find_slim_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SLIM_BIN_PATH")
        && !path.trim().is_empty()
    {
        return Ok(PathBuf::from(path));
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let sibling = parent.join("slim");
        if sibling.exists() {
            return Ok(sibling);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            let candidate = entry.join("slim");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    bail!("unable to locate slim binary (set SLIM_BIN_PATH or ensure slim is in PATH)")
}

fn create_default_temp_config() -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!("slimctl-config-{}.yaml", std::process::id()));

    let content = r#"# Auto-generated temporary configuration for slimctl
runtime:
  n_cores: 0
  thread_name: "slim-worker"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "${env:SLIM_ENDPOINT}"
          tls:
            insecure: true
      clients: []
"#;

    let mut file = std::fs::File::create(&path)
        .with_context(|| format!("failed to create temporary config at {}", path.display()))?;
    file.write_all(content.as_bytes())
        .context("failed to write temporary slim config")?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::create_default_temp_config;

    #[test]
    fn creates_default_temp_config_file() {
        let path = create_default_temp_config().expect("should create temp config");
        let contents = std::fs::read_to_string(&path).expect("should read temp config");

        assert!(contents.contains("# Auto-generated temporary configuration for slimctl"));
        assert!(contents.contains("${env:SLIM_ENDPOINT}"));

        let _ = std::fs::remove_file(path);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use duration_string::DurationString;
use serde::{Deserialize, Serialize};

use crate::defaults::{
    DEFAULT_EMPTY_CONFIG, DEFAULT_SERVER, DEFAULT_TIMEOUT, DEFAULT_TLS_INSECURE,
};

/// Persistent configuration stored in ~/.config/slimctl/config.yaml
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlimctlConfig {
    #[serde(default)]
    pub common_opts: CommonOptsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonOptsConfig {
    #[serde(default)]
    pub basic_auth_creds: String,
    #[serde(default = "default_server")]
    pub server: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    #[serde(default = "default_tls_insecure")]
    pub tls_insecure: bool,
    #[serde(default)]
    pub tls_insecure_skip_verify: bool,
    #[serde(default)]
    pub tls_ca_file: String,
    #[serde(default)]
    pub tls_cert_file: String,
    #[serde(default)]
    pub tls_key_file: String,
}

fn default_server() -> String {
    DEFAULT_SERVER.to_string()
}
fn default_timeout() -> String {
    DEFAULT_TIMEOUT.to_string()
}
fn default_tls_insecure() -> bool {
    DEFAULT_TLS_INSECURE
}

impl Default for CommonOptsConfig {
    fn default() -> Self {
        Self {
            basic_auth_creds: DEFAULT_EMPTY_CONFIG.to_string(),
            server: DEFAULT_SERVER.to_string(),
            timeout: DEFAULT_TIMEOUT.to_string(),
            tls_insecure: DEFAULT_TLS_INSECURE,
            tls_insecure_skip_verify: false,
            tls_ca_file: DEFAULT_EMPTY_CONFIG.to_string(),
            tls_cert_file: DEFAULT_EMPTY_CONFIG.to_string(),
            tls_key_file: DEFAULT_EMPTY_CONFIG.to_string(),
        }
    }
}

/// Resolved options merged from config file + env vars + CLI flags
#[derive(Debug, Clone)]
pub struct ResolvedOpts {
    pub server: String,
    pub timeout: Duration,
    pub tls_insecure: bool,
    pub tls_insecure_skip_verify: bool,
    pub tls_ca_file: String,
    pub tls_cert_file: String,
    pub tls_key_file: String,
    pub basic_auth_creds: String,
}

impl ResolvedOpts {
    /// Merge config file values with CLI overrides.
    /// CLI values (Some) override config file values.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        config: &SlimctlConfig,
        server: Option<&str>,
        timeout: Option<&str>,
        tls_insecure: bool,
        tls_insecure_skip_verify: bool,
        tls_ca_file: Option<&str>,
        tls_cert_file: Option<&str>,
        tls_key_file: Option<&str>,
        basic_auth_creds: Option<&str>,
    ) -> Result<Self> {
        let server = server
            .map(str::to_string)
            .unwrap_or_else(|| config.common_opts.server.clone());
        let timeout_str = timeout.unwrap_or(&config.common_opts.timeout);
        let timeout = parse_duration(timeout_str)
            .with_context(|| format!("invalid timeout value: '{}'", timeout_str))?;
        let tls_ca_file = tls_ca_file
            .map(str::to_string)
            .unwrap_or_else(|| config.common_opts.tls_ca_file.clone());
        let tls_cert_file = tls_cert_file
            .map(str::to_string)
            .unwrap_or_else(|| config.common_opts.tls_cert_file.clone());
        let tls_key_file = tls_key_file
            .map(str::to_string)
            .unwrap_or_else(|| config.common_opts.tls_key_file.clone());
        let basic_auth_creds = basic_auth_creds
            .map(str::to_string)
            .unwrap_or_else(|| config.common_opts.basic_auth_creds.clone());
        // Bool flags: CLI `true` wins; otherwise fall back to the config file value.
        let tls_insecure = tls_insecure || config.common_opts.tls_insecure;
        let tls_insecure_skip_verify =
            tls_insecure_skip_verify || config.common_opts.tls_insecure_skip_verify;

        Ok(Self {
            server,
            timeout,
            tls_insecure,
            tls_insecure_skip_verify,
            tls_ca_file,
            tls_cert_file,
            tls_key_file,
            basic_auth_creds,
        })
    }
}

/// Load configuration from the first existing candidate path:
/// 1. `config_file` if provided (via `--config` flag) — error if it does not exist
/// 2. `$HOME/.slimctl/config.yaml`
/// 3. `./config.yaml` (current directory)
///
/// Returns defaults if no file is found.
pub fn load_config(config_file: Option<&str>) -> Result<SlimctlConfig> {
    if let Some(path_str) = config_file {
        let path = PathBuf::from(path_str);
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        return serde_yaml::from_str(&data)
            .with_context(|| format!("failed to parse config file: {}", path.display()));
    }
    for path in config_search_paths() {
        if path.exists() {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config file: {}", path.display()))?;
            return serde_yaml::from_str(&data)
                .with_context(|| format!("failed to parse config file: {}", path.display()));
        }
    }
    Ok(SlimctlConfig::default())
}

/// Return candidate config file paths in priority order:
/// 1. `$HOME/.slimctl/config.yaml`
/// 2. `./config.yaml`
fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs_home() {
        paths.push(home.join(".slimctl").join("config.yaml"));
    }
    paths.push(PathBuf::from("config.yaml"));
    paths
}

/// Save configuration to `config_file` if provided, otherwise to `$HOME/.slimctl/config.yaml`.
pub fn save_config(config: &SlimctlConfig, config_file: Option<&str>) -> Result<()> {
    let path = match config_file {
        Some(p) => PathBuf::from(p),
        None => config_file_path()?,
    };
    let dir = path.parent().expect("config path must have a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
    let data = serde_yaml::to_string(config).context("failed to serialize config")?;
    std::fs::write(&path, data)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Return the default config file path: `$HOME/.slimctl/config.yaml`
pub fn config_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("config.yaml"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Parse a duration string like "15s", "1m", "500ms" into a std::time::Duration.
pub fn parse_duration(s: &str) -> Result<Duration> {
    Ok(DurationString::try_from(s.to_string())?.into())
}

/// Serializes all tests that mutate the `HOME` environment variable.
/// Shared across test modules to prevent races when multiple modules do HOME-dependent I/O.
#[cfg(test)]
#[allow(clippy::disallowed_types)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::{
        DEFAULT_EMPTY_CONFIG, DEFAULT_SERVER, DEFAULT_TIMEOUT, DEFAULT_TLS_INSECURE,
    };

    // ── parse_duration ──────────────────────────────────────────────────────

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration("15s").unwrap(), Duration::from_secs(15));
    }

    #[test]
    fn parse_duration_fractional_seconds() {
        assert_eq!(
            parse_duration("1500ms").unwrap(),
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
    }

    #[test]
    fn parse_duration_milliseconds() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration("not-a-duration").is_err());
    }

    #[test]
    fn parse_duration_empty() {
        assert!(parse_duration("").is_err());
    }

    // ── CommonOptsConfig::default ───────────────────────────────────────────

    #[test]
    fn common_opts_config_defaults() {
        let c = CommonOptsConfig::default();
        assert_eq!(c.server, DEFAULT_SERVER);
        assert_eq!(c.timeout, DEFAULT_TIMEOUT);
        assert_eq!(c.tls_insecure, DEFAULT_TLS_INSECURE);
        assert!(!c.tls_insecure_skip_verify);
        assert_eq!(c.basic_auth_creds, DEFAULT_EMPTY_CONFIG);
        assert_eq!(c.tls_ca_file, DEFAULT_EMPTY_CONFIG);
        assert_eq!(c.tls_cert_file, DEFAULT_EMPTY_CONFIG);
        assert_eq!(c.tls_key_file, DEFAULT_EMPTY_CONFIG);
    }

    #[test]
    fn slimctl_config_default_delegates_to_common_opts() {
        let c = SlimctlConfig::default();
        assert_eq!(c.common_opts.server, DEFAULT_SERVER);
    }

    // ── ResolvedOpts::resolve ───────────────────────────────────────────────

    #[test]
    fn resolve_uses_config_defaults_when_no_cli() {
        let config = SlimctlConfig::default();
        let opts = ResolvedOpts::resolve(&config, None, None, false, false, None, None, None, None)
            .unwrap();
        assert_eq!(opts.server, DEFAULT_SERVER);
        assert_eq!(opts.timeout, Duration::from_secs(15));
        assert!(!opts.tls_insecure);
        assert!(!opts.tls_insecure_skip_verify);
        assert!(opts.basic_auth_creds.is_empty());
        assert!(opts.tls_ca_file.is_empty());
        assert!(opts.tls_cert_file.is_empty());
        assert!(opts.tls_key_file.is_empty());
    }

    #[test]
    fn resolve_cli_server_overrides_config() {
        let config = SlimctlConfig::default();
        let opts = ResolvedOpts::resolve(
            &config,
            Some("custom:9999"),
            None,
            false,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(opts.server, "custom:9999");
    }

    #[test]
    fn resolve_inherits_server_from_config() {
        let mut config = SlimctlConfig::default();
        config.common_opts.server = "from-config:1234".to_string();
        let opts = ResolvedOpts::resolve(&config, None, None, false, false, None, None, None, None)
            .unwrap();
        assert_eq!(opts.server, "from-config:1234");
    }

    #[test]
    fn resolve_invalid_timeout_returns_error() {
        let config = SlimctlConfig::default();
        assert!(
            ResolvedOpts::resolve(
                &config,
                None,
                Some("not-a-duration"),
                false,
                false,
                None,
                None,
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn resolve_cli_timeout_overrides_config() {
        let config = SlimctlConfig::default();
        let opts = ResolvedOpts::resolve(
            &config,
            None,
            Some("30s"),
            false,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(opts.timeout, Duration::from_secs(30));
    }

    #[test]
    fn resolve_inherits_timeout_from_config() {
        let mut config = SlimctlConfig::default();
        config.common_opts.timeout = "5m".to_string();
        let opts = ResolvedOpts::resolve(&config, None, None, false, false, None, None, None, None)
            .unwrap();
        assert_eq!(opts.timeout, Duration::from_secs(300));
    }

    #[test]
    fn resolve_tls_flags_passed_through() {
        let config = SlimctlConfig::default();
        let opts =
            ResolvedOpts::resolve(&config, None, None, true, true, None, None, None, None).unwrap();
        assert!(opts.tls_insecure);
        assert!(opts.tls_insecure_skip_verify);
    }

    #[test]
    fn resolve_cli_basic_auth_overrides_config() {
        let mut config = SlimctlConfig::default();
        config.common_opts.basic_auth_creds = "config:pass".to_string();
        let opts = ResolvedOpts::resolve(
            &config,
            None,
            None,
            false,
            false,
            None,
            None,
            None,
            Some("cli:pass"),
        )
        .unwrap();
        assert_eq!(opts.basic_auth_creds, "cli:pass");
    }

    #[test]
    fn resolve_inherits_basic_auth_from_config() {
        let mut config = SlimctlConfig::default();
        config.common_opts.basic_auth_creds = "user:pass".to_string();
        let opts = ResolvedOpts::resolve(&config, None, None, false, false, None, None, None, None)
            .unwrap();
        assert_eq!(opts.basic_auth_creds, "user:pass");
    }

    #[test]
    fn resolve_cli_tls_files_override_config() {
        let config = SlimctlConfig::default();
        let opts = ResolvedOpts::resolve(
            &config,
            None,
            None,
            false,
            false,
            Some("/ca.pem"),
            Some("/cert.pem"),
            Some("/key.pem"),
            None,
        )
        .unwrap();
        assert_eq!(opts.tls_ca_file, "/ca.pem");
        assert_eq!(opts.tls_cert_file, "/cert.pem");
        assert_eq!(opts.tls_key_file, "/key.pem");
    }

    #[test]
    fn resolve_inherits_tls_files_from_config() {
        let mut config = SlimctlConfig::default();
        config.common_opts.tls_ca_file = "/etc/ca.pem".to_string();
        let opts = ResolvedOpts::resolve(&config, None, None, false, false, None, None, None, None)
            .unwrap();
        assert_eq!(opts.tls_ca_file, "/etc/ca.pem");
    }

    // ── config_file_path ────────────────────────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn config_file_path_under_home() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let path = config_file_path().unwrap();
        assert!(path.starts_with(dir.path()));
        assert!(path.ends_with("config.yaml"));
        assert!(path.to_str().unwrap().contains("slimctl"));
    }

    // ── load_config / save_config ───────────────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn load_config_returns_default_when_no_file() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let config = load_config(None).unwrap();
        assert_eq!(config.common_opts.server, DEFAULT_SERVER);
        assert_eq!(config.common_opts.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_and_load_config_roundtrip() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };

        let mut config = SlimctlConfig::default();
        config.common_opts.server = "testhost:9999".to_string();
        config.common_opts.timeout = "30s".to_string();
        config.common_opts.tls_insecure = false;
        save_config(&config, None).unwrap();

        let loaded = load_config(None).unwrap();
        assert_eq!(loaded.common_opts.server, "testhost:9999");
        assert_eq!(loaded.common_opts.timeout, "30s");
        assert!(!loaded.common_opts.tls_insecure);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_config_creates_directories() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let config = SlimctlConfig::default();
        save_config(&config, None).unwrap();
        assert!(config_file_path().unwrap().exists());
    }

    #[test]
    fn load_config_explicit_path_reads_that_file() {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::io::Write::write_all(&mut f, b"common_opts:\n  server: explicit-host:1234\n").unwrap();
        let config = load_config(Some(f.path().to_str().unwrap())).unwrap();
        assert_eq!(config.common_opts.server, "explicit-host:1234");
    }

    #[test]
    fn load_config_explicit_path_missing_returns_error() {
        assert!(load_config(Some("/nonexistent/path/config.yaml")).is_err());
    }

    #[test]
    fn save_config_explicit_path_writes_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.yaml");
        let config = SlimctlConfig::default();
        save_config(&config, Some(path.to_str().unwrap())).unwrap();
        assert!(path.exists());
        let loaded = load_config(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(loaded.common_opts.server, DEFAULT_SERVER);
    }
}

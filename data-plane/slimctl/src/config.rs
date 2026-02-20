// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_SERVER: &str = "localhost:50051";
const DEFAULT_TIMEOUT: &str = "15s";
const DEFAULT_TLS_INSECURE: bool = false;

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
            basic_auth_creds: String::new(),
            server: DEFAULT_SERVER.to_string(),
            timeout: DEFAULT_TIMEOUT.to_string(),
            tls_insecure: DEFAULT_TLS_INSECURE,
            tls_insecure_skip_verify: false,
            tls_ca_file: String::new(),
            tls_cert_file: String::new(),
            tls_key_file: String::new(),
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

/// Load configuration from ~/.config/slimctl/config.yaml.
/// Returns defaults if the file does not exist.
pub fn load_config() -> Result<SlimctlConfig> {
    let path = config_file_path()?;
    if !path.exists() {
        return Ok(SlimctlConfig::default());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config: SlimctlConfig = serde_yaml::from_str(&data)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?;
    Ok(config)
}

/// Save configuration to ~/.config/slimctl/config.yaml.
pub fn save_config(config: &SlimctlConfig) -> Result<()> {
    let path = config_file_path()?;
    let dir = path.parent().expect("config path must have a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
    let data = serde_yaml::to_string(config).context("failed to serialize config")?;
    std::fs::write(&path, data)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Return the path to the config file: ~/.config/slimctl/config.yaml
pub fn config_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".config").join("slimctl").join("config.yaml"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Parse a duration string like "15s", "1m", "500ms" into a std::time::Duration.
pub fn parse_duration(s: &str) -> Result<Duration> {
    if let Some(ms) = s.strip_suffix("ms") {
        let v: u64 = ms.parse().context("invalid milliseconds")?;
        return Ok(Duration::from_millis(v));
    }
    if let Some(m) = s.strip_suffix('m') {
        let v: u64 = m.parse().context("invalid minutes")?;
        return Ok(Duration::from_secs(v * 60));
    }
    if let Some(s_val) = s.strip_suffix('s') {
        let v: f64 = s_val.parse().context("invalid seconds")?;
        return Ok(Duration::from_secs_f64(v));
    }
    // Try plain number as seconds
    let v: u64 = s
        .parse()
        .context("invalid duration (expected e.g. '15s')")?;
    Ok(Duration::from_secs(v))
}

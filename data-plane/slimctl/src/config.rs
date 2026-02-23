use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub common_opts: CommonOpts,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommonOpts {
    pub basic_auth_creds: Option<String>,
    pub server: Option<String>,
    pub timeout: Option<String>,
    pub tls_insecure: Option<bool>,
    pub tls_ca_file: Option<String>,
    pub tls_cert_file: Option<String>,
    pub tls_key_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveCommonOpts {
    pub basic_auth_creds: String,
    pub server: String,
    pub timeout: String,
    pub tls_insecure: bool,
    pub tls_ca_file: String,
    pub tls_cert_file: String,
    pub tls_key_file: String,
}

impl Default for EffectiveCommonOpts {
    fn default() -> Self {
        Self {
            basic_auth_creds: String::new(),
            server: "localhost:50051".to_string(),
            timeout: "15s".to_string(),
            tls_insecure: true,
            tls_ca_file: String::new(),
            tls_cert_file: String::new(),
            tls_key_file: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CliCommonOverrides {
    pub basic_auth_creds: Option<String>,
    pub server: Option<String>,
    pub timeout: Option<String>,
    pub tls_insecure: Option<bool>,
    pub tls_ca_file: Option<String>,
    pub tls_cert_file: Option<String>,
    pub tls_key_file: Option<String>,
}

fn home_config_file() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("slimctl")
            .join("config.yaml")
    })
}

fn candidate_config_files() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home_cfg) = home_config_file() {
        candidates.push(home_cfg);
    }
    candidates.push(PathBuf::from("config.yaml"));
    candidates
}

pub fn load_first_existing_config() -> Result<AppConfig> {
    for file_path in candidate_config_files() {
        if file_path.exists() {
            let raw = fs::read_to_string(&file_path)
                .with_context(|| format!("failed to read config from {}", file_path.display()))?;
            let parsed: AppConfig = serde_yaml::from_str(&raw)
                .with_context(|| format!("failed to parse YAML in {}", file_path.display()))?;
            return Ok(parsed);
        }
    }

    Ok(AppConfig::default())
}

pub fn load_home_config() -> Result<AppConfig> {
    let Some(path) = home_config_file() else {
        return Ok(AppConfig::default());
    };

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config from {}", path.display()))?;
    let parsed: AppConfig = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse YAML in {}", path.display()))?;

    Ok(parsed)
}

pub fn save_home_config(config: &AppConfig) -> Result<PathBuf> {
    let Some(path) = home_config_file() else {
        anyhow::bail!("cannot determine HOME for slimctl config path");
    };

    let parent = path
        .parent()
        .context("failed to resolve parent directory for slimctl config")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create config directory {}", parent.display()))?;

    let serialized = serde_yaml::to_string(config).context("failed to serialize config to YAML")?;
    fs::write(&path, serialized)
        .with_context(|| format!("failed to write config file {}", path.display()))?;

    Ok(path)
}

pub fn resolve_effective_opts(
    file_cfg: &AppConfig,
    cli: &CliCommonOverrides,
) -> EffectiveCommonOpts {
    let mut effective = EffectiveCommonOpts::default();

    if let Some(value) = &file_cfg.common_opts.basic_auth_creds {
        effective.basic_auth_creds = value.clone();
    }
    if let Some(value) = &file_cfg.common_opts.server {
        effective.server = value.clone();
    }
    if let Some(value) = &file_cfg.common_opts.timeout {
        effective.timeout = value.clone();
    }
    if let Some(value) = file_cfg.common_opts.tls_insecure {
        effective.tls_insecure = value;
    }
    if let Some(value) = &file_cfg.common_opts.tls_ca_file {
        effective.tls_ca_file = value.clone();
    }
    if let Some(value) = &file_cfg.common_opts.tls_cert_file {
        effective.tls_cert_file = value.clone();
    }
    if let Some(value) = &file_cfg.common_opts.tls_key_file {
        effective.tls_key_file = value.clone();
    }

    if let Some(value) = &cli.basic_auth_creds {
        effective.basic_auth_creds = value.clone();
    }
    if let Some(value) = &cli.server {
        effective.server = value.clone();
    }
    if let Some(value) = &cli.timeout {
        effective.timeout = value.clone();
    }
    if let Some(value) = cli.tls_insecure {
        effective.tls_insecure = value;
    }
    if let Some(value) = &cli.tls_ca_file {
        effective.tls_ca_file = value.clone();
    }
    if let Some(value) = &cli.tls_cert_file {
        effective.tls_cert_file = value.clone();
    }
    if let Some(value) = &cli.tls_key_file {
        effective.tls_key_file = value.clone();
    }

    if let Ok(value) = env::var("SLIMCTL_BASIC_AUTH_CREDS") {
        effective.basic_auth_creds = value;
    }
    if let Ok(value) = env::var("SLIMCTL_SERVER") {
        effective.server = value;
    }
    if let Ok(value) = env::var("SLIMCTL_TIMEOUT") {
        effective.timeout = value;
    }
    if let Ok(value) = env::var("SLIMCTL_TLS_INSECURE")
        && let Ok(parsed) = value.parse::<bool>()
    {
        effective.tls_insecure = parsed;
    }
    if let Ok(value) = env::var("SLIMCTL_TLS_CA_FILE") {
        effective.tls_ca_file = value;
    }
    if let Ok(value) = env::var("SLIMCTL_TLS_CERT_FILE") {
        effective.tls_cert_file = value;
    }
    if let Ok(value) = env::var("SLIMCTL_TLS_KEY_FILE") {
        effective.tls_key_file = value;
    }

    effective
}

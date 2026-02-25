// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::config::{load_config, parse_duration, save_config};

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// List current configuration values
    #[command(alias = "ls")]
    List,
    /// Set a configuration value
    Set(SetArgs),
}

#[derive(Args)]
pub struct SetArgs {
    #[command(subcommand)]
    pub command: SetCommand,
}

#[derive(Subcommand)]
pub enum SetCommand {
    /// Set basic auth credentials (username:password)
    BasicAuthCreds { value: String },
    /// Set the gRPC server address (host:port)
    Server { value: String },
    /// Set the request timeout (e.g. 15s, 1m)
    Timeout { value: String },
    /// Set TLS CA certificate file path
    TlsCaFile { value: String },
    /// Set TLS client certificate file path
    TlsCertFile { value: String },
    /// Set TLS client key file path
    TlsKeyFile { value: String },
    /// Set TLS insecure mode - disables TLS and uses plain HTTP (true/false)
    TlsInsecure { value: String },
    /// Set TLS insecure skip verify mode - skips TLS certificate verification (true/false)
    TlsInsecureSkipVerify { value: String },
}

pub async fn run(args: &ConfigArgs, config_file: Option<&str>) -> Result<()> {
    match &args.command {
        ConfigCommand::List => run_list(config_file),
        ConfigCommand::Set(set_args) => run_set(&set_args.command, config_file),
    }
}

fn run_list(config_file: Option<&str>) -> Result<()> {
    let config = load_config(config_file)?;
    let yaml = serde_yaml::to_string(&config)?;
    print!("{}", yaml);
    Ok(())
}

fn run_set(cmd: &SetCommand, config_file: Option<&str>) -> Result<()> {
    let mut config = load_config(config_file)?;
    match cmd {
        SetCommand::BasicAuthCreds { value } => {
            config.common_opts.basic_auth_creds = value.clone();
        }
        SetCommand::Server { value } => {
            config.common_opts.server = value.clone();
        }
        SetCommand::Timeout { value } => {
            // Validate the duration string
            parse_duration(value)?;
            config.common_opts.timeout = value.clone();
        }
        SetCommand::TlsCaFile { value } => {
            config.common_opts.tls_ca_file = value.clone();
        }
        SetCommand::TlsCertFile { value } => {
            config.common_opts.tls_cert_file = value.clone();
        }
        SetCommand::TlsKeyFile { value } => {
            config.common_opts.tls_key_file = value.clone();
        }
        SetCommand::TlsInsecure { value } => {
            let insecure: bool = value.parse().map_err(|_| {
                anyhow::anyhow!(
                    "invalid value '{}' for tls-insecure, expected true or false",
                    value
                )
            })?;
            config.common_opts.tls_insecure = insecure;
        }
        SetCommand::TlsInsecureSkipVerify { value } => {
            let skip_verify: bool = value.parse().map_err(|_| {
                anyhow::anyhow!(
                    "invalid value '{}' for tls-insecure-skip-verify, expected true or false",
                    value
                )
            })?;
            config.common_opts.tls_insecure_skip_verify = skip_verify;
        }
    }
    save_config(&config, config_file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_config(path: &str) {
        crate::config::save_config(&crate::config::SlimctlConfig::default(), Some(path)).unwrap();
    }

    #[test]
    fn set_tls_ca_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        init_config(&path_str);
        run_set(
            &SetCommand::TlsCaFile {
                value: "/path/ca.pem".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();
        let config = crate::config::load_config(Some(&path_str)).unwrap();
        assert_eq!(config.common_opts.tls_ca_file, "/path/ca.pem");
    }

    #[test]
    fn set_tls_cert_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        init_config(&path_str);
        run_set(
            &SetCommand::TlsCertFile {
                value: "/path/cert.pem".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();
        let config = crate::config::load_config(Some(&path_str)).unwrap();
        assert_eq!(config.common_opts.tls_cert_file, "/path/cert.pem");
    }

    #[test]
    fn set_tls_key_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        init_config(&path_str);
        run_set(
            &SetCommand::TlsKeyFile {
                value: "/path/key.pem".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();
        let config = crate::config::load_config(Some(&path_str)).unwrap();
        assert_eq!(config.common_opts.tls_key_file, "/path/key.pem");
    }

    #[test]
    fn set_basic_auth_creds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        init_config(&path_str);
        run_set(
            &SetCommand::BasicAuthCreds {
                value: "user:pass".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();
        let config = crate::config::load_config(Some(&path_str)).unwrap();
        assert_eq!(config.common_opts.basic_auth_creds, "user:pass");
    }
}

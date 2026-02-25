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

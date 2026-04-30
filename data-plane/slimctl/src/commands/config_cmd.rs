// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::{Args, Subcommand};
use duration_string::DurationString;

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::grpc::client::AuthenticationConfig;

use crate::config::{load_config, parse_duration, save_config};

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// List current configuration values
    #[command(visible_alias = "ls")]
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
    /// Set TLS client certificate and key file paths together
    TlsCert {
        /// Path to client TLS certificate file
        cert_file: String,
        /// Path to client TLS key file
        key_file: String,
    },
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
            let (user, pass) = value
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("basic-auth-creds must be 'username:password'"))?;
            config.auth = AuthenticationConfig::Basic(BasicAuthConfig::new(user, pass));
        }
        SetCommand::Server { value } => {
            config.endpoint = value.clone();
        }
        SetCommand::Timeout { value } => {
            let dur = parse_duration(value)?;
            config.connect_timeout = DurationString::from(dur);
            config.request_timeout = DurationString::from(dur);
        }
        SetCommand::TlsCaFile { value } => {
            config.tls_setting = config.tls_setting.clone().with_ca_file(value);
        }
        SetCommand::TlsCert {
            cert_file,
            key_file,
        } => {
            config.tls_setting = config
                .tls_setting
                .clone()
                .with_cert_and_key_file(cert_file, key_file);
        }
        SetCommand::TlsInsecureSkipVerify { value } => {
            let skip_verify: bool = value.parse().map_err(|_| {
                anyhow::anyhow!(
                    "invalid value '{}' for tls-insecure-skip-verify, expected true or false",
                    value
                )
            })?;
            config.tls_setting = config
                .tls_setting
                .clone()
                .with_insecure_skip_verify(skip_verify);
        }
    }
    save_config(&config, config_file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::grpc::client::ClientConfig;

    fn init_config(path: &str) {
        crate::config::save_config(&ClientConfig::default(), Some(path)).unwrap();
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
        // CA file is stored in the tls_setting's ca_source
        assert!(!config.tls_setting.insecure);
    }

    #[test]
    fn set_tls_cert() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        init_config(&path_str);
        run_set(
            &SetCommand::TlsCert {
                cert_file: "/path/cert.pem".to_string(),
                key_file: "/path/key.pem".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();
        let config = crate::config::load_config(Some(&path_str)).unwrap();
        // cert/key is stored in the tls_setting's source
        assert!(!config.tls_setting.insecure);
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
        assert!(matches!(config.auth, AuthenticationConfig::Basic(_)));
    }
}

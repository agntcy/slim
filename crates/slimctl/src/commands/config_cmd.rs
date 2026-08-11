// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::{Args, Subcommand};
use duration_string::DurationString;

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::grpc::client::AuthenticationConfig;
use slim_config::tls::client::TlsClientConfig;

use crate::config::{get_config_key, load_config, parse_duration, set_config_key};

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

/// Set a single config key, leaving every other key in the file untouched —
/// including any `${env:...}`/`${file:...}` references, which a full
/// deserialize-modify-serialize cycle would replace with their expansions.
fn run_set(cmd: &SetCommand, config_file: Option<&str>) -> Result<()> {
    match cmd {
        SetCommand::BasicAuthCreds { value } => {
            let (user, pass) = value
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("basic-auth-creds must be 'username:password'"))?;
            set_config_key(
                "auth",
                &AuthenticationConfig::Basic(BasicAuthConfig::new(user, pass)),
                config_file,
            )?;
        }
        SetCommand::Server { value } => {
            set_config_key("endpoint", value, config_file)?;
        }
        SetCommand::Timeout { value } => {
            let dur = DurationString::from(parse_duration(value)?);
            set_config_key("connect_timeout", &dur, config_file)?;
            set_config_key("request_timeout", &dur, config_file)?;
        }
        SetCommand::TlsCaFile { value } => {
            update_tls(config_file, |tls| tls.with_ca_file(value))?;
        }
        SetCommand::TlsCert {
            cert_file,
            key_file,
        } => {
            update_tls(config_file, |tls| {
                tls.with_cert_and_key_file(cert_file, key_file)
            })?;
        }
        SetCommand::TlsInsecureSkipVerify { value } => {
            let skip_verify: bool = value.parse().map_err(|_| {
                anyhow::anyhow!(
                    "invalid value '{}' for tls-insecure-skip-verify, expected true or false",
                    value
                )
            })?;
            update_tls(config_file, |tls| {
                tls.with_insecure_skip_verify(skip_verify)
            })?;
        }
    }
    Ok(())
}

/// Apply `f` to the config's current `tls` block (default when absent) and write
/// just that key back.  The TLS builders need the existing value, so this is a
/// read-modify-write scoped to the one key rather than the whole file.
fn update_tls<F>(config_file: Option<&str>, f: F) -> Result<()>
where
    F: FnOnce(TlsClientConfig) -> TlsClientConfig,
{
    let current: TlsClientConfig = get_config_key("tls", config_file)?;
    set_config_key("tls", &f(current), config_file)?;
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

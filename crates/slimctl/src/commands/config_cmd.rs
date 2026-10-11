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
    print!("{}", render_list(config_file)?);
    Ok(())
}

fn render_list(config_file: Option<&str>) -> Result<String> {
    let mut value = serde_yaml::to_value(load_config(config_file)?)?;
    redact_secrets(&mut value, None);
    Ok(serde_yaml::to_string(&value)?)
}

/// Printed by `config list` in place of a secret.
const REDACTED: &str = "(redacted)";

/// Mask the secrets in a serialized config: passwords, OIDC client secrets and
/// refresh tokens, header values, inline TLS private keys and inline JWT keys.
///
/// Goes by key name because, once serialized, a secret is a plain string
/// (`OpaqueString` serializes its cleartext), and a `${env:...}`/`${file:...}`
/// reference has already been replaced by its expansion.  `parent` is the key
/// under which `value` sits.
fn redact_secrets(value: &mut serde_yaml::Value, parent: Option<&str>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let inline_pem = map.get("type").and_then(|t| t.as_str()) == Some("pem");
            for (key, v) in map.iter_mut() {
                match key.as_str() {
                    Some("password" | "client_secret" | "refresh_token") => mask(v),
                    Some("headers") => {
                        if let serde_yaml::Value::Mapping(headers) = v {
                            headers.values_mut().for_each(mask);
                        }
                    }
                    // A TLS `source` of type `pem` carries the private key inline.
                    Some("key") if inline_pem => mask(v),
                    // A JWT key given inline: `key: {data: ...}`.
                    Some("data") if parent == Some("key") => mask(v),
                    k => redact_secrets(v, k),
                }
            }
        }
        serde_yaml::Value::Sequence(items) => items
            .iter_mut()
            .for_each(|item| redact_secrets(item, parent)),
        _ => {}
    }
}

/// Replace a set value with [`REDACTED`], leaving unset ones visible as unset.
fn mask(value: &mut serde_yaml::Value) {
    let unset = value.is_null() || value.as_str() == Some("");
    if !unset {
        *value = serde_yaml::Value::String(REDACTED.to_string());
    }
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

    #[test]
    fn list_redacts_a_basic_auth_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.yaml");
        let path_str = path.to_str().unwrap().to_string();
        run_set(
            &SetCommand::BasicAuthCreds {
                value: "alice:hunter2".to_string(),
            },
            Some(&path_str),
        )
        .unwrap();

        let out = render_list(Some(&path_str)).unwrap();
        assert!(!out.contains("hunter2"), "leaked: {out}");
        assert!(out.contains("alice"), "username should stay visible: {out}");
        assert!(out.contains(REDACTED), "{out}");
    }

    #[test]
    fn list_redacts_a_secret_read_from_a_file_reference() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "from-the-file").unwrap();
        let path = dir.path().join("cfg.yaml");
        std::fs::write(
            &path,
            format!(
                "endpoint: http://localhost:46358\n\
                 auth:\n  type: basic\n  username: alice\n  password: ${{file:{}}}\n",
                secret.display()
            ),
        )
        .unwrap();

        let out = render_list(path.to_str()).unwrap();
        assert!(!out.contains("from-the-file"), "leaked: {out}");
    }

    fn redacted(yaml: &str) -> serde_yaml::Value {
        let mut value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        redact_secrets(&mut value, None);
        value
    }

    #[test]
    fn redacts_every_kind_of_secret() {
        let value = redacted(
            r#"
headers: {authorization: Bearer abc}
proxy: {username: bob, password: p1, headers: {x-api-key: k1}}
auth: {type: oidc, client_secret: s1, refresh_token: r1, refresh_token_file: /rt}
tls: {source: {type: pem, cert: CERT, key: PRIVATE}}
jwt: {key: {type: encoding, algorithm: ES256, format: pem, key: {data: INLINE}}}
"#,
        );
        for path in [
            &["headers", "authorization"][..],
            &["proxy", "password"],
            &["proxy", "headers", "x-api-key"],
            &["auth", "client_secret"],
            &["auth", "refresh_token"],
            &["tls", "source", "key"],
            &["jwt", "key", "key", "data"],
        ] {
            let got = path.iter().fold(&value, |v, k| &v[*k]);
            assert_eq!(got.as_str(), Some(REDACTED), "{path:?} in {value:?}");
        }
        // Not secrets: who you are, where things are, public material.
        assert_eq!(value["proxy"]["username"].as_str(), Some("bob"));
        assert_eq!(value["auth"]["refresh_token_file"].as_str(), Some("/rt"));
        assert_eq!(value["tls"]["source"]["cert"].as_str(), Some("CERT"));
        assert_eq!(value["jwt"]["key"]["algorithm"].as_str(), Some("ES256"));
    }

    #[test]
    fn keeps_file_paths_and_unset_secrets_visible() {
        let value = redacted(
            r#"
tls: {source: {type: file, cert: /c.pem, key: /k.pem}, ca_source: {type: pem, data: CA}}
jwt: {key: {type: decoding, algorithm: ES256, format: pem, key: {file: /pub.pem}}}
proxy: {password: null}
auth: {type: oidc, client_secret: ""}
"#,
        );
        assert_eq!(value["tls"]["source"]["key"].as_str(), Some("/k.pem"));
        assert_eq!(value["tls"]["ca_source"]["data"].as_str(), Some("CA"));
        assert_eq!(
            value["jwt"]["key"]["key"]["file"].as_str(),
            Some("/pub.pem")
        );
        assert!(value["proxy"]["password"].is_null());
        assert_eq!(value["auth"]["client_secret"].as_str(), Some(""));
    }
}

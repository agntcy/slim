// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration module for the Channel Manager.
//!
//! Supports loading configuration from a YAML file with the following structure:
//!
//! ```yaml
//! channel-manager:
//!   slim-connection:
//!     endpoint: "http://127.0.0.1:46357"
//!     tls:
//!       insecure: true
//!   api-server:
//!     endpoint: "127.0.0.1:10356"
//!     tls:
//!       insecure: true
//!   local-name: "agntcy/otel/channel-manager"
//!   auth:
//!     type: shared_secret
//!     # id: "custom/identity/id"  # optional, defaults to local-name
//!     secret: "a-very-long-shared-secret"
//!   channels:
//!     - name: "agntcy/otel/channel"
//!       participants:
//!         - "agntcy/otel/exporter"
//!         - "agntcy/otel/receiver"
//!       mls-enabled: true
//! ```

use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;
use slim_config::auth::identity::{
    IdentityProviderConfig, IdentityVerifierConfig,
};
#[cfg(not(target_family = "windows"))]
use slim_config::auth::spire::SpireConfig;
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;

/// Authentication configuration for the SLIM app identity.
///
/// For `shared_secret`, an optional `id` can be provided. When omitted it
/// defaults to the `local-name` field of the manager configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthConfig {
    /// Shared secret authentication (symmetric key)
    SharedSecret {
        /// Identity id. Defaults to `local-name` when not provided.
        id: Option<String>,
        /// The shared secret value
        secret: String,
    },
    /// SPIRE-based identity (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    Spire(SpireConfig),
}

impl AuthConfig {
    /// Validate the auth configuration fields.
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            AuthConfig::SharedSecret { secret, .. } => {
                if secret.is_empty() {
                    bail!("auth.secret cannot be empty for shared_secret");
                }
            }
            #[cfg(not(target_family = "windows"))]
            AuthConfig::Spire(spire_config) => {
                if spire_config.socket_path.is_none() {
                    bail!("auth.socket_path must be set for spire");
                }
            }
        }
        Ok(())
    }

    /// Convert to IdentityProviderConfig + IdentityVerifierConfig pair.
    /// For shared_secret, uses the explicit `id` if provided, otherwise
    /// falls back to `local_name`.
    pub fn to_identity_configs(
        &self,
        local_name: &str,
    ) -> (IdentityProviderConfig, IdentityVerifierConfig) {
        match self {
            AuthConfig::SharedSecret { id, secret } => {
                let identity_id = id.as_deref().unwrap_or(local_name).to_string();
                (
                    IdentityProviderConfig::SharedSecret {
                        id: identity_id.clone(),
                        data: secret.clone(),
                    },
                    IdentityVerifierConfig::SharedSecret {
                        id: identity_id,
                        data: secret.clone(),
                    },
                )
            }
            #[cfg(not(target_family = "windows"))]
            AuthConfig::Spire(spire_config) => (
                IdentityProviderConfig::Spire(spire_config.clone()),
                IdentityVerifierConfig::Spire(spire_config.clone()),
            ),
        }
    }
}

/// Top-level configuration
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Channel manager settings
    #[serde(rename = "channel-manager")]
    pub manager: ManagerConfig,
}

/// Channel manager service configuration
#[derive(Debug, Deserialize)]
pub struct ManagerConfig {
    /// gRPC client configuration for connecting to the SLIM data-plane node.
    /// Supports TLS, auth, keepalive, proxy, etc. — same format as the SLIM data-plane ClientConfig.
    #[serde(rename = "slim-connection")]
    pub slim_connection: ClientConfig,

    /// gRPC server configuration for the channel management API.
    /// Supports TLS, auth, keepalive, unix sockets, etc. — same format as the SLIM data-plane ServerConfig.
    #[serde(rename = "api-server")]
    pub api_server: ServerConfig,

    /// Local name for the channel manager in SLIM (org/namespace/app format)
    #[serde(rename = "local-name")]
    pub local_name: String,

    /// Authentication configuration for the SLIM app identity.
    /// Required. Supports: shared_secret or spire.
    pub auth: AuthConfig,

    /// Channels to create on startup
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
}

/// Configuration for a single channel
#[derive(Debug, Deserialize)]
pub struct ChannelConfig {
    /// Channel name in SLIM format (org/namespace/channel)
    pub name: String,

    /// Participants to invite to this channel
    pub participants: Vec<String>,

    /// Whether MLS encryption is enabled for this channel (default: true)
    #[serde(rename = "mls-enabled", default = "default_mls_enabled")]
    pub mls_enabled: bool,
}

fn default_mls_enabled() -> bool {
    true
}

impl Config {
    /// Load configuration from a YAML file
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data =
            std::fs::read_to_string(path).with_context(|| format!("reading config file {:?}", path))?;
        let cfg: Config =
            serde_yaml::from_str(&data).with_context(|| "parsing config YAML")?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.manager.slim_connection.endpoint.is_empty() {
            bail!("slim-connection.endpoint cannot be empty");
        }
        if self.manager.api_server.endpoint.is_empty() {
            bail!("api-server.endpoint cannot be empty");
        }
        if self.manager.local_name.is_empty() {
            bail!("local-name cannot be empty");
        }

        // Validate SLIM name format (org/namespace/app)
        let parts: Vec<&str> = self.manager.local_name.split('/').collect();
        if parts.len() != 3 {
            bail!(
                "local-name must be in org/namespace/app format, got: {}",
                self.manager.local_name
            );
        }

        // Validate auth configuration
        self.manager.auth.validate()?;

        for (i, channel) in self.manager.channels.iter().enumerate() {
            if channel.name.is_empty() {
                bail!("channel[{}].name cannot be empty", i);
            }
            let parts: Vec<&str> = channel.name.split('/').collect();
            if parts.len() != 3 {
                bail!(
                    "channel[{}].name must be in org/namespace/channel format, got: {}",
                    i,
                    channel.name
                );
            }
            if channel.participants.is_empty() {
                bail!("channel[{}].participants cannot be empty", i);
            }
            for (j, participant) in channel.participants.iter().enumerate() {
                let parts: Vec<&str> = participant.split('/').collect();
                if parts.len() != 3 {
                    bail!(
                        "channel[{}].participants[{}] must be in org/namespace/app format, got: {}",
                        i,
                        j,
                        participant
                    );
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────

    fn base_yaml(overrides: &str) -> String {
        format!(
            r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "agntcy/otel/channel-manager"
  auth:
    type: shared_secret
    secret: "test-secret-0123456789-abcdefghijk"
{overrides}"#
        )
    }

    // ── Config parsing ───────────────────────────────────────────────────

    #[test]
    fn test_parse_valid_config() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "agntcy/otel/channel"
      participants:
        - "agntcy/otel/exporter"
        - "agntcy/otel/receiver"
      mls-enabled: true"#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.manager.channels.len(), 1);
        assert_eq!(cfg.manager.channels[0].participants.len(), 2);
        assert!(cfg.manager.channels[0].mls_enabled);
    }

    #[test]
    fn test_no_channels_is_valid() {
        let yaml = base_yaml("");
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(cfg.validate().is_ok());
        assert!(cfg.manager.channels.is_empty());
    }

    #[test]
    fn test_mls_enabled_by_default() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "a/b/c"
      participants:
        - "a/b/d""#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(cfg.manager.channels[0].mls_enabled);
    }

    #[test]
    fn test_missing_auth_fails_parsing() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "agntcy/otel/cm"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_auth_type_fails_parsing() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "a/b/c"
  auth:
    type: unknown_type
    secret: "secret"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    // ── Endpoint validation ──────────────────────────────────────────────

    #[test]
    fn test_empty_slim_connection_endpoint_fails() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: ""
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "agntcy/otel/channel-manager"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("slim-connection.endpoint"), "got: {err}");
    }

    #[test]
    fn test_empty_api_server_endpoint_fails() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: ""
    tls:
      insecure: true
  local-name: "agntcy/otel/channel-manager"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("api-server.endpoint"), "got: {err}");
    }

    // ── local-name validation ────────────────────────────────────────────

    #[test]
    fn test_empty_local_name_fails() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: ""
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("local-name"), "got: {err}");
    }

    #[test]
    fn test_invalid_name_format_two_parts() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "org/app"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("org/namespace/app"), "got: {err}");
    }

    #[test]
    fn test_invalid_name_format_four_parts() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "a/b/c/d"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("org/namespace/app"), "got: {err}");
    }

    #[test]
    fn test_invalid_name_format_no_slashes() {
        let yaml = r#"
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "flat-name"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    // ── Channel validation ───────────────────────────────────────────────

    #[test]
    fn test_empty_channel_name_fails() {
        let yaml = base_yaml(
            r#"  channels:
    - name: ""
      participants:
        - "a/b/c""#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("channel[0].name"), "got: {err}");
    }

    #[test]
    fn test_invalid_channel_name_format() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "bad-channel"
      participants:
        - "a/b/c""#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("channel[0].name"), "got: {err}");
    }

    #[test]
    fn test_empty_participants_fails() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "a/b/c"
      participants: []"#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("participants cannot be empty"), "got: {err}");
    }

    #[test]
    fn test_invalid_participant_name_format() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "a/b/c"
      participants:
        - "bad-participant""#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("participants[0]"),
            "got: {err}"
        );
    }

    #[test]
    fn test_second_channel_validation_error() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "a/b/c"
      participants:
        - "a/b/d"
    - name: "bad"
      participants:
        - "a/b/e""#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("channel[1]"), "got: {err}");
    }

    #[test]
    fn test_multiple_channels_valid() {
        let yaml = base_yaml(
            r#"  channels:
    - name: "a/b/c1"
      participants:
        - "a/b/p1"
    - name: "a/b/c2"
      participants:
        - "a/b/p2"
        - "a/b/p3"
      mls-enabled: true"#,
        );
        let cfg: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.manager.channels.len(), 2);
        assert!(cfg.manager.channels[0].mls_enabled); // default is true
        assert!(cfg.manager.channels[1].mls_enabled); // explicitly true
    }

    // ── AuthConfig validation ───────────────────────────────────────────

    #[test]
    fn test_shared_secret_empty_secret_fails() {
        let auth = AuthConfig::SharedSecret {
            id: None,
            secret: "".to_string(),
        };
        let err = auth.validate();
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("auth.secret cannot be empty"));
    }

    #[test]
    fn test_shared_secret_valid_secret_passes() {
        let auth = AuthConfig::SharedSecret {
            id: None,
            secret: "my-secret".to_string(),
        };
        assert!(auth.validate().is_ok());
    }

    // ── AuthConfig identity configs ──────────────────────────────────────

    #[test]
    fn test_shared_secret_identity_configs_no_id() {
        let auth = AuthConfig::SharedSecret {
            id: None,
            secret: "my-secret".to_string(),
        };
        let (provider, verifier) = auth.to_identity_configs("org/ns/app");

        match &provider {
            IdentityProviderConfig::SharedSecret { id, data } => {
                assert_eq!(id, "org/ns/app");
                assert_eq!(data, "my-secret");
            }
            _ => panic!("expected SharedSecret provider"),
        }

        match &verifier {
            IdentityVerifierConfig::SharedSecret { id, data } => {
                assert_eq!(id, "org/ns/app");
                assert_eq!(data, "my-secret");
            }
            _ => panic!("expected SharedSecret verifier"),
        }
    }

    #[test]
    fn test_shared_secret_defaults_to_local_name() {
        let auth = AuthConfig::SharedSecret {
            id: None,
            secret: "s".to_string(),
        };
        let (provider, _) = auth.to_identity_configs("different/local/name");

        match &provider {
            IdentityProviderConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "different/local/name");
            }
            _ => panic!("expected SharedSecret"),
        }
    }

    #[test]
    fn test_shared_secret_explicit_id_overrides_local_name() {
        let auth = AuthConfig::SharedSecret {
            id: Some("custom/identity/id".to_string()),
            secret: "s".to_string(),
        };
        let (provider, verifier) = auth.to_identity_configs("org/ns/app");

        match &provider {
            IdentityProviderConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "custom/identity/id");
            }
            _ => panic!("expected SharedSecret"),
        }
        match &verifier {
            IdentityVerifierConfig::SharedSecret { id, .. } => {
                assert_eq!(id, "custom/identity/id");
            }
            _ => panic!("expected SharedSecret"),
        }
    }

    // ── Config::load from file ───────────────────────────────────────────

    #[test]
    fn test_load_nonexistent_file() {
        let result = Config::load(Path::new("/nonexistent/path.yaml"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("reading config file"));
    }

    #[test]
    fn test_load_invalid_yaml_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("bad.yaml");
        std::fs::write(&file_path, "not: valid: yaml: [").unwrap();
        let result = Config::load(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("good.yaml");
        let yaml = base_yaml("");
        std::fs::write(&file_path, yaml).unwrap();
        let cfg = Config::load(&file_path).unwrap();
        assert_eq!(cfg.manager.local_name, "agntcy/otel/channel-manager");
    }
}

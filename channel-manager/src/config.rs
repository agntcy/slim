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
/// The `id` used for shared_secret provider/verifier is automatically
/// derived from the `local-name` field.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthConfig {
    /// Shared secret authentication (symmetric key)
    SharedSecret {
        /// The shared secret value
        secret: String,
    },
    /// SPIRE-based identity (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    Spire(SpireConfig),
}

impl AuthConfig {
    /// Convert to IdentityProviderConfig + IdentityVerifierConfig pair,
    /// using `local_name` as the identity id for shared_secret.
    pub fn to_identity_configs(
        &self,
        local_name: &str,
    ) -> (IdentityProviderConfig, IdentityVerifierConfig) {
        match self {
            AuthConfig::SharedSecret { secret } => (
                IdentityProviderConfig::SharedSecret {
                    id: local_name.to_string(),
                    data: secret.clone(),
                },
                IdentityVerifierConfig::SharedSecret {
                    id: local_name.to_string(),
                    data: secret.clone(),
                },
            ),
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

    /// Whether MLS encryption is enabled for this channel
    #[serde(rename = "mls-enabled", default)]
    pub mls_enabled: bool,
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

    #[test]
    fn test_parse_valid_config() {
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
  local-name: "agntcy/otel/channel-manager"
  auth:
    type: shared_secret
    secret: "test-secret-0123456789-abcdefghijk"
  channels:
    - name: "agntcy/otel/channel"
      participants:
        - "agntcy/otel/exporter"
        - "agntcy/otel/receiver"
      mls-enabled: true
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.manager.channels.len(), 1);
        assert_eq!(cfg.manager.channels[0].participants.len(), 2);
        assert!(cfg.manager.channels[0].mls_enabled);
    }

    #[test]
    fn test_empty_endpoint_fails_validation() {
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
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_invalid_name_format_fails() {
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
  local-name: "invalid-name"
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_no_channels_is_valid() {
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
  auth:
    type: shared_secret
    secret: "secret"
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_ok());
        assert!(cfg.manager.channels.is_empty());
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
}

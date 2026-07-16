// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Hierarchical SLIM node configuration discovery (FFI/UniFFI adapter).
//!
//! The actual config-loading logic lives in `agntcy-slim-service::node_config`.
//! This module keeps the UniFFI-exported record types (`SlimConfig`, `SlimAppConfig`)
//! and a thin `load_slim_config` wrapper that delegates to the core implementation
//! and converts the result back to FFI types.

use slim_service::node_config as core_node_config;

use crate::client_config::ClientConfig;
use crate::errors::SlimError;
use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};

// ============================================================================
// UniFFI-exported record types
// ============================================================================

/// App identity settings (from the `app:` section of `slim.yaml`)
#[derive(uniffi::Record, Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SlimAppConfig {
    /// Local agent name in `"org/namespace/app"` format
    #[serde(default)]
    pub name: String,
    /// Identity provider — which credential the app presents to the SLIM node.
    #[serde(default)]
    pub identity: IdentityProviderConfig,
    /// Identity verifier — how the app verifies credentials from the SLIM node.
    #[serde(default)]
    pub identity_verifier: IdentityVerifierConfig,
}

/// Resolved SLIM configuration returned by [`load_slim_config`].
///
/// Contains the merged result of all config sources (files + env vars) plus
/// resolved cache metadata.
#[derive(
    uniffi::Record, Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub struct SlimConfig {
    /// Node connection settings
    #[serde(default)]
    pub node: ClientConfig,
    /// App identity settings. Only set when `app.name` is configured.
    #[serde(default)]
    pub app: Option<SlimAppConfig>,
    /// Path of the config file that was used (for debugging / logging).
    #[serde(skip, default)]
    pub source_path: Option<String>,
    /// Cache directory path, derived from the config file location.
    #[serde(skip, default)]
    pub cache_dir: Option<String>,
}

// ============================================================================
// Conversions between FFI types and core types
// ============================================================================

fn ffi_app_to_core(app: SlimAppConfig) -> core_node_config::SlimAppConfig {
    use slim_config::auth::identity::IdentityProviderConfig as CoreIdP;
    use slim_config::auth::identity::IdentityVerifierConfig as CoreIdV;
    core_node_config::SlimAppConfig {
        name: app.name,
        identity: CoreIdP::from(IdentityProviderConfig::from(app.identity)),
        identity_verifier: CoreIdV::from(IdentityVerifierConfig::from(app.identity_verifier)),
    }
}

fn core_app_to_ffi(app: core_node_config::SlimAppConfig) -> SlimAppConfig {
    use slim_config::auth::identity::IdentityProviderConfig as CoreIdP;
    use slim_config::auth::identity::IdentityVerifierConfig as CoreIdV;
    SlimAppConfig {
        name: app.name,
        identity: IdentityProviderConfig::from(CoreIdP::from(app.identity)),
        identity_verifier: IdentityVerifierConfig::from(CoreIdV::from(app.identity_verifier)),
    }
}

impl From<SlimConfig> for core_node_config::SlimNodeConfig {
    fn from(ffi: SlimConfig) -> Self {
        core_node_config::SlimNodeConfig {
            node: ffi.node.into(),
            app: ffi.app.map(ffi_app_to_core),
            source_path: ffi.source_path,
            cache_dir: ffi.cache_dir,
        }
    }
}

impl From<core_node_config::SlimNodeConfig> for SlimConfig {
    fn from(core: core_node_config::SlimNodeConfig) -> Self {
        SlimConfig {
            node: core.node.into(),
            app: core.app.map(core_app_to_ffi),
            source_path: core.source_path,
            cache_dir: core.cache_dir,
        }
    }
}

// ============================================================================
// Public exported function
// ============================================================================

/// Load SLIM configuration using hierarchical file discovery plus env var overrides.
///
/// Delegates to [`slim_service::node_config::load_slim_node_config`] and converts
/// the result back to UniFFI-compatible FFI types.
///
/// **Search order** (highest priority first):
/// 1. Environment variables (`SLIM_NODE_ADDRESS`, `SLIM_APP_NAME`,
///    `SLIM_NODE_CERT_PATH`, `SLIM_IDENTITY_TOKEN`, `SLIM_IDENTITY_SHARED_SECRET`)
/// 2. `slim.yaml` found by walking up from the current working directory
/// 3. `~/.slim/config.yaml` as the user-level default
///
/// # Errors
///
/// Returns [`SlimError::ConfigError`] if no node address could be resolved from
/// any source, or if a specified config file cannot be read / parsed.
#[uniffi::export]
pub fn load_slim_config(config_path: Option<String>) -> Result<SlimConfig, SlimError> {
    let core_config =
        core_node_config::load_slim_node_config(config_path.as_deref()).map_err(|e| {
            SlimError::ConfigError {
                message: e.to_string(),
            }
        })?;
    Ok(SlimConfig::from(core_config))
}

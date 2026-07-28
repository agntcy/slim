// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Hierarchical SLIM node configuration discovery.
//!
//! Implements git-style config file search:
//! 1. Walk up from CWD looking for `slim.yaml`
//! 2. Fall back to `~/.slim/config.yaml` as user-level default
//! 3. Environment variables override any file-based value (highest priority)
//!
//! A `.slim-cache/` directory next to the discovered config file persists
//! the app instance UUID and Ed25519 signature key pair so the agent keeps
//! a stable identity across restarts.

use std::path::{Path, PathBuf};

use tracing::debug;

use slim_config::auth::identity::{IdentityProviderConfig, IdentityVerifierConfig};
use slim_config::auth::static_jwt::Config as StaticJwtConfig;
use slim_config::grpc::client::ClientConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::common::{CaSource, TlsSource};

use crate::errors::ServiceError;

// ============================================================================
// Public configuration types
// ============================================================================

/// App identity settings (from the `app:` section of `slim.yaml`).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SlimAppConfig {
    /// Local agent name in `"org/namespace/app"` format.
    #[serde(default)]
    pub name: String,
    /// Identity provider — which credential the app presents to the SLIM node.
    #[serde(default)]
    pub identity: IdentityProviderConfig,
    /// Identity verifier — how the app verifies credentials from the SLIM node.
    #[serde(default)]
    pub identity_verifier: IdentityVerifierConfig,
}

/// Resolved SLIM configuration returned by [`load_slim_node_config`].
///
/// Contains the merged result of all config sources (files + env vars) plus
/// resolved cache metadata.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SlimNodeConfig {
    /// Node connection settings.
    #[serde(default)]
    pub node: ClientConfig,
    /// App identity settings. Only set when `app.name` is configured.
    #[serde(default)]
    pub app: Option<SlimAppConfig>,
    /// Path of the config file that was used (for debugging / logging).
    #[serde(skip)]
    pub source_path: Option<String>,
    /// Cache directory path, derived from the config file location.
    #[serde(skip)]
    pub cache_dir: Option<String>,
}

// ============================================================================
// Config file search
// ============================================================================

fn find_slim_yaml(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let candidate = dir.join("slim.yaml");
        if candidate.is_file() {
            debug!(?candidate, "Found slim.yaml");
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn user_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".slim").join("config.yaml"))
}

fn cache_dir_for(config_path: &Path) -> PathBuf {
    if let (Some(config_dir), Some(home)) = (config_path.parent(), dirs::home_dir()) {
        let user_slim = home.join(".slim");
        if config_dir == user_slim {
            return user_slim.join("cache");
        }
    }
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".slim-cache")
}

fn load_config_silent(path: &Path) -> Option<SlimNodeConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&content).ok()
}

fn load_config(path: &Path) -> Result<SlimNodeConfig, ServiceError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| ServiceError::InvalidConfig(format!(
            "Failed to read {}: {}",
            path.display(),
            e
        )))?;
    serde_yaml::from_str(&content).map_err(|e| ServiceError::InvalidConfig(format!(
        "Failed to parse {}: {}",
        path.display(),
        e
    )))
}

// ============================================================================
// Merging and env var overrides
// ============================================================================

fn merge_configs(base: &mut SlimNodeConfig, overlay: &SlimNodeConfig) {
    if !overlay.node.endpoint.is_empty() {
        base.node = overlay.node.clone();
    }
    if overlay.app.is_some() {
        base.app = overlay.app.clone();
    }
}

fn default_slim_app() -> SlimAppConfig {
    SlimAppConfig {
        name: String::new(),
        identity: IdentityProviderConfig::None,
        identity_verifier: IdentityVerifierConfig::None,
    }
}

fn apply_env_overrides(cfg: &mut SlimNodeConfig) {
    if let Ok(v) = std::env::var("SLIM_NODE_ADDRESS") {
        cfg.node.endpoint = v;
    }
    if let Ok(v) = std::env::var("SLIM_APP_NAME") {
        cfg.app.get_or_insert_with(default_slim_app).name = v;
    }
    if let Ok(v) = std::env::var("SLIM_NODE_CERT_PATH") {
        let p = expand_tilde(&v);
        cfg.node.tls_setting.config.source = TlsSource::File {
            cert: p.clone(),
            key: p,
        };
    }
    if let Ok(v) = std::env::var("SLIM_IDENTITY_TOKEN") {
        let token_file = expand_tilde(&v);
        cfg.app.get_or_insert_with(default_slim_app).identity =
            IdentityProviderConfig::StaticJwt(
                StaticJwtConfig::with_file(&token_file)
                    .with_duration(std::time::Duration::from_secs(3600)),
            );
    }
    if let Ok(v) = std::env::var("SLIM_IDENTITY_SHARED_SECRET") {
        cfg.app.get_or_insert_with(default_slim_app).identity =
            IdentityProviderConfig::SharedSecret {
                id: String::new(),
                data: v,
            };
    }
}

// ============================================================================
// Tilde expansion
// ============================================================================

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}

fn expand_tilde_in_tls(tls: &mut TlsClientConfig) {
    if let TlsSource::File { cert, key } = &mut tls.config.source {
        *cert = expand_tilde(cert);
        *key = expand_tilde(key);
    }
    if let CaSource::File { path } = &mut tls.config.ca_source {
        *path = expand_tilde(path);
    }
}

// ============================================================================
// Public entry point
// ============================================================================

/// Load SLIM configuration using hierarchical file discovery plus env var overrides.
///
/// **Search order** (highest priority first):
/// 1. Environment variables (`SLIM_NODE_ADDRESS`, `SLIM_APP_NAME`,
///    `SLIM_NODE_CERT_PATH`, `SLIM_IDENTITY_TOKEN`, `SLIM_IDENTITY_SHARED_SECRET`)
/// 2. `slim.yaml` found by walking up from the current working directory
/// 3. `~/.slim/config.yaml` as the user-level default
///
/// # Errors
///
/// Returns [`ServiceError::InvalidConfig`] if no node address could be resolved from
/// any source, or if a specified config file cannot be read / parsed.
pub fn load_slim_node_config(config_path: Option<&str>) -> Result<SlimNodeConfig, ServiceError> {
    // 1. Load user-level config (lowest priority)
    let user_path = user_config_path();
    let mut merged: SlimNodeConfig = user_path
        .as_deref()
        .and_then(load_config_silent)
        .unwrap_or_default();

    // 2. Determine source config file
    let (source_path, cache_dir) = if let Some(explicit) = config_path {
        let p = Path::new(explicit);
        if !p.is_file() {
            return Err(ServiceError::InvalidConfig(format!(
                "Config file not found: {explicit}"
            )));
        }
        let cfg = load_config(p)?;
        merge_configs(&mut merged, &cfg);
        let cache = cache_dir_for(p);
        (
            Some(p.to_string_lossy().to_string()),
            Some(cache.to_string_lossy().to_string()),
        )
    } else {
        let cwd = std::env::current_dir().map_err(|e| {
            ServiceError::InvalidConfig(format!("Failed to get current directory: {e}"))
        })?;
        let local_path = find_slim_yaml(&cwd);
        if let Some(ref lp) = local_path {
            let local_cfg = load_config(lp)?;
            merge_configs(&mut merged, &local_cfg);
            let cache = cache_dir_for(lp);
            (
                Some(lp.to_string_lossy().to_string()),
                Some(cache.to_string_lossy().to_string()),
            )
        } else if let Some(ref up) = user_path {
            if up.is_file() {
                let cache = cache_dir_for(up);
                (
                    Some(up.to_string_lossy().to_string()),
                    Some(cache.to_string_lossy().to_string()),
                )
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    };

    // 3. Apply environment variable overrides (highest priority)
    apply_env_overrides(&mut merged);

    // 4. Fail if no endpoint was resolved
    if merged.node.endpoint.is_empty() {
        return Err(ServiceError::InvalidConfig(
            "No SLIM node address found. Set the SLIM_NODE_ADDRESS environment variable \
             or add 'node.endpoint' to slim.yaml (or ~/.slim/config.yaml)."
                .to_string(),
        ));
    }

    // 5. Tilde-expand TLS file paths; infer insecure from URL scheme when no tls block set
    expand_tilde_in_tls(&mut merged.node.tls_setting);
    if merged.node.tls_setting == TlsClientConfig::default() {
        merged.node.tls_setting.insecure = !merged.node.endpoint.starts_with("https://")
            && !merged.node.endpoint.starts_with("wss://");
    }

    // 6. Drop app if name is empty; fill SharedSecret id from app name when unset
    let app = merged
        .app
        .take()
        .filter(|a| !a.name.is_empty())
        .map(|mut a| {
            if let IdentityProviderConfig::SharedSecret { id, .. } = &mut a.identity {
                if id.is_empty() {
                    *id = a.name.clone();
                }
            }
            if let IdentityVerifierConfig::SharedSecret { id, .. } = &mut a.identity_verifier {
                if id.is_empty() {
                    *id = a.name.clone();
                }
            }
            a
        });

    debug!(?source_path, ?cache_dir, "SLIM config resolved");

    Ok(SlimNodeConfig {
        node: merged.node,
        app,
        source_path,
        cache_dir,
    })
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn clear_env() {
        for var in &[
            "SLIM_NODE_ADDRESS",
            "SLIM_APP_NAME",
            "SLIM_NODE_CERT_PATH",
            "SLIM_IDENTITY_TOKEN",
            "SLIM_IDENTITY_SHARED_SECRET",
        ] {
            // SAFETY: tests run in forked processes (test_fork::fork)
            unsafe { std::env::remove_var(var) };
        }
    }

    fn set_test_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: tests run in forked processes (test_fork::fork)
        unsafe { std::env::set_var(key, value) }
    }

    fn write_slim_yaml(dir: &Path, content: &str) {
        fs::write(dir.join("slim.yaml"), content).expect("write slim.yaml");
    }

    fn write_user_config(slim_dir: &Path, content: &str) {
        fs::create_dir_all(slim_dir).expect("create .slim dir");
        fs::write(slim_dir.join("config.yaml"), content).expect("write config.yaml");
    }

    #[test_fork::fork]
    #[test]
    fn test_env_overrides_local_file() {
        clear_env();
        let tmp = TempDir::new().unwrap();
        write_slim_yaml(
            tmp.path(),
            "node:\n  endpoint: \"https://file.example.com:46357\"\n",
        );
        std::env::set_current_dir(tmp.path()).unwrap();
        set_test_env("HOME", tmp.path());
        set_test_env("SLIM_NODE_ADDRESS", "https://env.example.com:9999");

        let config = load_slim_node_config(None).unwrap();
        assert_eq!(config.node.endpoint, "https://env.example.com:9999");
    }

    #[test_fork::fork]
    #[test]
    fn test_local_file_overrides_user_config() {
        clear_env();
        let tmp = TempDir::new().unwrap();
        let fake_home = tmp.path().join("home");
        let slim_dir = fake_home.join(".slim");
        fs::create_dir_all(&slim_dir).unwrap();
        write_user_config(
            &slim_dir,
            "node:\n  endpoint: \"https://user.example.com:46357\"\n",
        );

        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write_slim_yaml(
            &project,
            "node:\n  endpoint: \"https://project.example.com:46357\"\n",
        );

        set_test_env("HOME", &fake_home);
        std::env::set_current_dir(&project).unwrap();

        let config = load_slim_node_config(None).unwrap();
        assert_eq!(config.node.endpoint, "https://project.example.com:46357");
    }

    #[test_fork::fork]
    #[test]
    fn test_no_config_returns_error() {
        clear_env();
        let tmp = TempDir::new().unwrap();
        set_test_env("HOME", tmp.path());
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = load_slim_node_config(None);
        assert!(result.is_err());
        let ServiceError::InvalidConfig(msg) = result.unwrap_err() else {
            panic!("Expected InvalidConfig");
        };
        assert!(
            msg.contains("No SLIM node address"),
            "Unexpected message: {msg}"
        );
    }

    #[test_fork::fork]
    #[test]
    fn test_yaml_identity_shared_secret_defaults_id_to_app_name() {
        clear_env();
        let tmp = TempDir::new().unwrap();
        set_test_env("HOME", tmp.path());
        write_slim_yaml(
            tmp.path(),
            "node:\n  endpoint: \"https://example.com:46357\"\napp:\n  name: \"org/ns/alice\"\n  identity:\n    type: shared_secret\n    data: \"longlonglonglonglonglonglonglonglong-secret\"\n",
        );
        std::env::set_current_dir(tmp.path()).unwrap();

        let config = load_slim_node_config(None).unwrap();
        match config.app.unwrap().identity {
            IdentityProviderConfig::SharedSecret { id, data } => {
                assert_eq!(id, "org/ns/alice");
                assert_eq!(data, "longlonglonglonglonglonglonglonglong-secret");
            }
            other => panic!("Expected SharedSecret, got {other:?}"),
        }
    }
}

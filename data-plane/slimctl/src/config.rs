// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use duration_string::DurationString;

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::grpc::client::{AuthenticationConfig, BackoffConfig, ClientConfig};

pub(crate) const DEFAULT_TIMEOUT: &str = "15s";
pub(crate) const DEFAULT_NODE_ENDPOINT: &str = "127.0.0.1:46357";
pub(crate) const DEFAULT_CONTROLLER_ENDPOINT: &str = "127.0.0.1:50051";
pub(crate) const DEFAULT_CHANNEL_MANAGER_ENDPOINT: &str = "127.0.0.1:10356";

/// Merge a file-level `ClientConfig` with CLI overrides.
/// `default_endpoint` is the per-subcommand fallback when neither the file
/// nor the CLI specifies an endpoint.
#[allow(clippy::too_many_arguments)]
pub fn resolve_config(
    file_config: &ClientConfig,
    default_endpoint: &str,
    server: Option<&str>,
    timeout: Option<&str>,
    tls_insecure_skip_verify: bool,
    tls_ca_file: Option<&str>,
    tls_cert_file: Option<&str>,
    tls_key_file: Option<&str>,
    basic_auth_creds: Option<&str>,
) -> Result<ClientConfig> {
    let mut config = file_config.clone();

    // ── endpoint ────────────────────────────────────────────────────
    if let Some(s) = server {
        config.endpoint = s.to_string();
    }
    if config.endpoint.is_empty() {
        config.endpoint = default_endpoint.to_string();
    }

    // ── timeout ─────────────────────────────────────────────────────
    let timeout_str = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let timeout_dur = parse_duration(timeout_str)
        .with_context(|| format!("invalid timeout value: '{}'", timeout_str))?;
    if timeout.is_some() || config.request_timeout.as_secs() == 0 {
        config.connect_timeout = DurationString::from(timeout_dur);
        config.request_timeout = DurationString::from(timeout_dur);
    }

    // ── TLS overlay ─────────────────────────────────────────────────
    // Validate cert/key pairing
    if tls_cert_file.is_some() ^ tls_key_file.is_some() {
        bail!("both tls-cert-file and tls-key-file must be specified together");
    }

    let any_tls_flag =
        tls_insecure_skip_verify || tls_ca_file.is_some() || tls_cert_file.is_some();

    let mut tls = config.tls_setting.clone();

    // If any TLS flag is provided and the file config was insecure,
    // switch to secure mode.
    if any_tls_flag && tls.insecure {
        tls = tls.with_insecure(false);
    }

    if tls_insecure_skip_verify {
        tls = tls.with_insecure_skip_verify(true);
    }
    if let Some(ca) = tls_ca_file {
        tls = tls.with_ca_file(ca);
    }
    if let (Some(cert), Some(key)) = (tls_cert_file, tls_key_file) {
        tls = tls.with_cert_and_key_file(cert, key);
    }

    config.tls_setting = tls;

    // ── endpoint scheme ─────────────────────────────────────────────
    if !config.endpoint.starts_with("http://") && !config.endpoint.starts_with("https://") {
        let scheme = if config.tls_setting.insecure {
            "http"
        } else {
            "https"
        };
        config.endpoint = format!("{}://{}", scheme, config.endpoint);
    }

    // ── auth ────────────────────────────────────────────────────────
    if let Some(creds) = basic_auth_creds {
        let (user, pass) = creds
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("basic-auth-creds must be 'username:password'"))?;
        config.auth = AuthenticationConfig::Basic(BasicAuthConfig::new(user, pass));
    }

    // ── backoff (no retries by default for CLI) ─────────────────────
    config.backoff = BackoffConfig::new_fixed_interval(Duration::from_millis(0), 0);

    Ok(config)
}

/// Load configuration from the first existing candidate path:
/// 1. `config_file` if provided (via `--config` flag) — error if it does not exist
/// 2. `$HOME/.slimctl/config.yaml`
/// 3. `./config.yaml` (current directory)
///
/// Returns defaults if no file is found.
pub fn load_config(config_file: Option<&str>) -> Result<ClientConfig> {
    if let Some(path_str) = config_file {
        let path = PathBuf::from(path_str);
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        return serde_yaml::from_str(&data)
            .with_context(|| format!("failed to parse config file: {}", path.display()));
    }
    for path in config_search_paths() {
        if path.exists() {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config file: {}", path.display()))?;
            return serde_yaml::from_str(&data)
                .with_context(|| format!("failed to parse config file: {}", path.display()));
        }
    }
    Ok(ClientConfig::default())
}

/// Return candidate config file paths in priority order:
/// 1. `$HOME/.slimctl/config.yaml`
/// 2. `./config.yaml`
fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs_home() {
        paths.push(home.join(".slimctl").join("config.yaml"));
    }
    paths.push(PathBuf::from("config.yaml"));
    paths
}

/// Save configuration to `config_file` if provided, otherwise to `$HOME/.slimctl/config.yaml`.
pub fn save_config(config: &ClientConfig, config_file: Option<&str>) -> Result<()> {
    let path = match config_file {
        Some(p) => PathBuf::from(p),
        None => config_file_path()?,
    };
    let dir = path.parent().expect("config path must have a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
    let data = serde_yaml::to_string(config).context("failed to serialize config")?;
    std::fs::write(&path, data)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Return the default config file path: `$HOME/.slimctl/config.yaml`
pub fn config_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("config.yaml"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Parse a duration string like "15s", "1m", "500ms" into a std::time::Duration.
pub fn parse_duration(s: &str) -> Result<Duration> {
    Ok(DurationString::try_from(s.to_string())?.into())
}

/// Serializes all tests that mutate the `HOME` environment variable.
/// Shared across test modules to prevent races when multiple modules do HOME-dependent I/O.
#[cfg(test)]
#[allow(clippy::disallowed_types)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use slim_config::tls::client::TlsClientConfig;

    // ── parse_duration ──────────────────────────────────────────────────────

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration("15s").unwrap(), Duration::from_secs(15));
    }

    #[test]
    fn parse_duration_fractional_seconds() {
        assert_eq!(
            parse_duration("1500ms").unwrap(),
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
    }

    #[test]
    fn parse_duration_milliseconds() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration("not-a-duration").is_err());
    }

    #[test]
    fn parse_duration_empty() {
        assert!(parse_duration("").is_err());
    }

    // ── resolve_config ───────────────────────────────────────────────

    #[test]
    fn resolve_uses_defaults_when_no_cli_and_empty_file_config() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        // endpoint should be the default with scheme prepended
        assert!(opts.endpoint.contains(DEFAULT_NODE_ENDPOINT));
        assert_eq!(
            Duration::from(opts.request_timeout),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn resolve_cli_server_overrides_config() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            Some("custom:9999"),
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(opts.endpoint.contains("custom:9999"));
    }

    #[test]
    fn resolve_inherits_endpoint_from_file_config() {
        let file_config = ClientConfig::with_endpoint("from-config:1234")
            .with_tls_setting(TlsClientConfig::insecure());
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(opts.endpoint.contains("from-config:1234"));
    }

    #[test]
    fn resolve_invalid_timeout_returns_error() {
        let file_config = ClientConfig::default();
        assert!(resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            Some("not-a-duration"),
            false,
            None,
            None,
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn resolve_cli_timeout_overrides_config() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            Some("30s"),
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            Duration::from(opts.request_timeout),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn resolve_file_timeout_preserved_when_no_cli_flag() {
        let file_config = ClientConfig::default()
            .with_request_timeout(Duration::from_secs(10))
            .with_connect_timeout(Duration::from_secs(10));
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            Duration::from(opts.request_timeout),
            Duration::from_secs(10)
        );
        assert_eq!(
            Duration::from(opts.connect_timeout),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn resolve_tls_skip_verify_passed_through() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            true,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(opts.tls_setting.insecure_skip_verify);
    }

    #[test]
    fn resolve_cli_basic_auth_overrides_config() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            Some("cli:pass"),
        )
        .unwrap();
        assert!(matches!(opts.auth, AuthenticationConfig::Basic(_)));
    }

    #[test]
    fn resolve_cli_tls_files_applied() {
        let file_config = ClientConfig::default();
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            Some("/ca.pem"),
            Some("/cert.pem"),
            Some("/key.pem"),
            None,
        )
        .unwrap();
        // TLS should no longer be insecure since TLS flags were provided
        assert!(!opts.tls_setting.insecure);
    }

    #[test]
    fn resolve_cert_without_key_fails() {
        let file_config = ClientConfig::default();
        assert!(resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            Some("/cert.pem"),
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn resolve_key_without_cert_fails() {
        let file_config = ClientConfig::default();
        assert!(resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            Some("/key.pem"),
            None,
        )
        .is_err());
    }

    #[test]
    fn resolve_basic_auth_without_colon_fails() {
        let file_config = ClientConfig::default();
        assert!(resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            Some("usernameonly"),
        )
        .is_err());
    }

    #[test]
    fn resolve_insecure_config_gets_http_scheme() {
        let file_config =
            ClientConfig::with_endpoint("myhost:1234").with_tls_setting(TlsClientConfig::insecure());
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(opts.endpoint.starts_with("http://"));
    }

    #[test]
    fn resolve_secure_config_gets_https_scheme() {
        let file_config =
            ClientConfig::with_endpoint("myhost:1234").with_tls_setting(TlsClientConfig::new());
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(opts.endpoint.starts_with("https://"));
    }

    #[test]
    fn resolve_explicit_http_scheme_preserved() {
        let file_config = ClientConfig::with_endpoint("http://myhost:1234");
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(opts.endpoint, "http://myhost:1234");
    }

    #[test]
    fn resolve_explicit_https_scheme_preserved() {
        let file_config = ClientConfig::with_endpoint("https://myhost:1234");
        let opts = resolve_config(
            &file_config,
            DEFAULT_NODE_ENDPOINT,
            None,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(opts.endpoint, "https://myhost:1234");
    }

    // ── config_file_path ────────────────────────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn config_file_path_under_home() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let path = config_file_path().unwrap();
        assert!(path.starts_with(dir.path()));
        assert!(path.ends_with("config.yaml"));
        assert!(path.to_str().unwrap().contains("slimctl"));
    }

    // ── load_config / save_config ───────────────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn load_config_returns_default_when_no_file() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let config = load_config(None).unwrap();
        assert!(config.endpoint.is_empty());
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_and_load_config_roundtrip() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };

        let config = ClientConfig::with_endpoint("testhost:9999")
            .with_tls_setting(TlsClientConfig::insecure());
        save_config(&config, None).unwrap();

        let loaded = load_config(None).unwrap();
        assert_eq!(loaded.endpoint, "testhost:9999");
        assert!(loaded.tls_setting.insecure);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_config_creates_directories() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK; no other threads read HOME concurrently.
        unsafe { std::env::set_var("HOME", dir.path()) };
        let config = ClientConfig::default();
        save_config(&config, None).unwrap();
        assert!(config_file_path().unwrap().exists());
    }

    #[test]
    fn load_config_explicit_path_reads_that_file() {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::io::Write::write_all(&mut f, b"endpoint: explicit-host:1234\n").unwrap();
        let config = load_config(Some(f.path().to_str().unwrap())).unwrap();
        assert_eq!(config.endpoint, "explicit-host:1234");
    }

    #[test]
    fn load_config_explicit_path_missing_returns_error() {
        assert!(load_config(Some("/nonexistent/path/config.yaml")).is_err());
    }

    #[test]
    fn save_config_explicit_path_writes_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.yaml");
        let config = ClientConfig::with_endpoint("myhost:1234");
        save_config(&config, Some(path.to_str().unwrap())).unwrap();
        assert!(path.exists());
        let loaded = load_config(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(loaded.endpoint, "myhost:1234");
    }
}

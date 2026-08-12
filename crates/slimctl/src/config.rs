// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use duration_string::DurationString;
use serde::{Deserialize, Serialize};
use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::auth::static_jwt::Config as StaticJwtConfig;
use slim_config::grpc::client::{AuthenticationConfig, BackoffConfig, ClientConfig};
use slim_config::provider::ConfigResolver;
use slim_config::tls::client::TlsClientConfig;

/// Default timeout for gRPC requests when not specified in config or CLI.
pub(crate) const DEFAULT_TIMEOUT: &str = "15s";
/// Default endpoint for the `node` subcommand (SLIM node control API).
pub(crate) const DEFAULT_NODE_ENDPOINT: &str = "127.0.0.1:46358";
/// Default endpoint for the `controller` subcommand (controller north bound API).
pub(crate) const DEFAULT_CONTROLLER_ENDPOINT: &str = "127.0.0.1:50051";
/// Default endpoint for the `channel-manager` subcommand (channel manager gRPC API).
pub(crate) const DEFAULT_CHANNEL_MANAGER_ENDPOINT: &str = "127.0.0.1:10356";
/// Default listen address for starting a local SLIM node via the `slim` subcommand.
pub(crate) const DEFAULT_SLIM_ADDRESS: &str = "127.0.0.1:46357";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcCredentials {
    pub id_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub client_id: String,
    pub issuer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_endpoint: String,
}

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
    if timeout.is_some() || config.request_timeout.as_millis() == 0 {
        config.connect_timeout = DurationString::from(timeout_dur);
        config.request_timeout = DurationString::from(timeout_dur);
    }

    // ── TLS overlay ─────────────────────────────────────────────────
    // Validate cert/key pairing
    if tls_cert_file.is_some() ^ tls_key_file.is_some() {
        bail!("both tls-cert-file and tls-key-file must be specified together");
    }

    let any_tls_flag = tls_insecure_skip_verify || tls_ca_file.is_some() || tls_cert_file.is_some();

    // Track whether the endpoint came from the file config or from CLI/default
    let endpoint_from_file = !file_config.endpoint.is_empty() && server.is_none();

    let mut tls = config.tls_setting.clone();

    // When no TLS flags are given and the config still has the bare default
    // (insecure=false but no certs/CA configured), and the endpoint did NOT
    // come from a file config (which may have intentionally set secure TLS),
    // treat it as insecure so that a plain `host:port` endpoint defaults to http://.
    if !any_tls_flag && !endpoint_from_file && tls == TlsClientConfig::default() {
        tls = TlsClientConfig::insecure();
    }

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
    // if the endpoint already has a scheme, respect it and do not override based on TLS settings, since the user explicitly specified it.
    if config.endpoint.starts_with("http://") {
        config.tls_setting = config.tls_setting.with_insecure(true);
    } else if config.endpoint.starts_with("https://") {
        config.tls_setting = config.tls_setting.with_insecure(false);
    } else {
        // if the endpoint has no scheme, prepend one based on the TLS settings (http for insecure, https for secure)
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
    } else if config.auth == AuthenticationConfig::None
        && let Ok(token_path) = token_file_path()
        && token_path.exists()
    {
        // Configs written before `login` started recording an auth block, or
        // hand-written ones: fall back to the bearer token on disk.
        //
        // Deliberately *not* the refresh token, even when `credentials.yaml` has
        // one. Exchanging it invalidates the stored copy, so a CLI doing so on
        // every invocation would race any long-lived process seeded from the same
        // token. An expired access token is a prompt to re-run `slimctl login`.
        config.auth = AuthenticationConfig::StaticJwt(StaticJwtConfig::with_file(
            token_path.to_string_lossy(),
        ));
    }

    // ── backoff (no retries by default for CLI) ─────────────────────
    config.backoff = BackoffConfig::new_fixed_interval(Duration::from_millis(0), 0);

    Ok(config)
}

/// Parse config YAML, expanding `${env:VAR}` / `${file:/path}` references first.
///
/// Mirrors what the node, control-plane and channel-manager loaders do, so a
/// slimctl config can keep secrets out of the YAML itself.  Note that a value
/// must consist *only* of the reference — `"Bearer ${file:...}"` is not expanded.
fn parse_config_str(data: &str) -> Result<ClientConfig> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(data).context("invalid YAML")?;
    ConfigResolver::new()
        .resolve(&mut value)
        .context("failed to resolve ${env:...}/${file:...} reference")?;

    // `endpoint` is the only field `ClientConfig` requires, but slimctl writes
    // configs surgically (see `set_config_key`) so a file may legitimately hold
    // just an `auth` block. Supply the empty default rather than rejecting it —
    // `resolve_config` substitutes the per-subcommand endpoint anyway.
    if let serde_yaml::Value::Mapping(map) = &mut value {
        map.entry(serde_yaml::Value::String("endpoint".to_string()))
            .or_insert_with(|| serde_yaml::Value::String(String::new()));
    }

    serde_yaml::from_value(value).context("invalid configuration")
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
        return parse_config_str(&data)
            .with_context(|| format!("failed to parse config file: {}", path.display()));
    }
    for path in config_search_paths() {
        if path.exists() {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read config file: {}", path.display()))?;
            return parse_config_str(&data)
                .with_context(|| format!("failed to parse config file: {}", path.display()));
        }
    }
    Ok(ClientConfig::default())
}

/// Resolve which file a read-modify-write cycle should operate on: the explicit
/// `--config` path, else the first existing search path, else the default.
fn update_target_path(config_file: Option<&str>) -> Result<PathBuf> {
    match config_file {
        Some(p) => Ok(PathBuf::from(p)),
        None => match config_search_paths().into_iter().find(|p| p.exists()) {
            Some(p) => Ok(p),
            None => config_file_path(),
        },
    }
}

/// Load the config file as a raw YAML mapping for a read-modify-write cycle
/// (`config set`, `login`).  Returns an empty mapping when the file is absent.
///
/// Working at the mapping level rather than through `ClientConfig` keeps writes
/// surgical: only the keys a command actually sets are emitted, so a fresh login
/// does not litter the file with every defaulted field (`endpoint: ''`,
/// `connect_timeout: 0y`, a pinned random `link_id`, a dozen `null`s).
///
/// It also means `${env:...}`/`${file:...}` references survive verbatim — the
/// value is about to be written back, and resolving first would replace a
/// reference with its expansion, baking a secret read from a file into the YAML.
pub fn load_config_mapping(config_file: Option<&str>) -> Result<serde_yaml::Mapping> {
    let path = update_target_path(config_file)?;
    if !path.exists() {
        return Ok(serde_yaml::Mapping::new());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    match serde_yaml::from_str::<Option<serde_yaml::Value>>(&data)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?
    {
        None | Some(serde_yaml::Value::Null) => Ok(serde_yaml::Mapping::new()),
        Some(serde_yaml::Value::Mapping(m)) => Ok(m),
        Some(_) => bail!(
            "config file must contain a YAML mapping: {}",
            path.display()
        ),
    }
}

/// Write a raw config mapping back, creating parent directories as needed.
pub fn save_config_mapping(map: &serde_yaml::Mapping, config_file: Option<&str>) -> Result<()> {
    let path = update_target_path(config_file)?;
    let dir = path.parent().expect("config path must have a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
    let data = serde_yaml::to_string(map).context("failed to serialize config")?;
    std::fs::write(&path, data)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Set one top-level key in the config file, leaving every other key byte-identical.
pub fn set_config_key<T: Serialize>(
    key: &str,
    value: &T,
    config_file: Option<&str>,
) -> Result<PathBuf> {
    let mut map = load_config_mapping(config_file)?;
    let encoded =
        serde_yaml::to_value(value).with_context(|| format!("failed to serialize '{key}'"))?;
    map.insert(serde_yaml::Value::String(key.to_string()), encoded);
    save_config_mapping(&map, config_file)?;
    update_target_path(config_file)
}

/// Read one top-level key, deserialized into `T`, or `T::default()` when absent.
pub fn get_config_key<T: serde::de::DeserializeOwned + Default>(
    key: &str,
    config_file: Option<&str>,
) -> Result<T> {
    let map = load_config_mapping(config_file)?;
    match map.get(serde_yaml::Value::String(key.to_string())) {
        Some(v) => serde_yaml::from_value(v.clone())
            .with_context(|| format!("failed to parse '{key}' in config file")),
        None => Ok(T::default()),
    }
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

/// Write a whole `ClientConfig`, every field included.
///
/// Test-only: production writes are surgical (see [`set_config_key`]) so that a
/// login does not litter the file with defaulted fields. Tests use this to build
/// a fully-populated file and then assert those fields survive an update.
#[cfg(test)]
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

/// Path to the bare token file used by StaticJwt injection: `~/.slimctl/token`
pub fn credentials_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("credentials.yaml"))
}

pub fn token_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("token"))
}

/// Path to the bare refresh token referenced by the generated config:
/// `~/.slimctl/refresh_token`
pub fn refresh_token_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("refresh_token"))
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(data)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?
        .write_all(data)
}

pub fn save_credentials(creds: &OidcCredentials) -> Result<()> {
    let path = credentials_file_path()?;
    let dir = path.parent().expect("credentials path must have a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
    let data = serde_yaml::to_string(creds).context("failed to serialize credentials")?;
    write_private(&path, data.as_bytes())
        .with_context(|| format!("failed to write credentials: {}", path.display()))?;
    // Write bearer token for StaticJwt auto-injection; prefer access_token (longer TTL).
    let token = creds.access_token.as_deref().unwrap_or(&creds.id_token);
    let token_path = token_file_path()?;
    write_private(&token_path, token.as_bytes())
        .with_context(|| format!("failed to write token: {}", token_path.display()))?;
    Ok(())
}

/// Where `save_login_auth` wrote things.
pub struct LoginAuthWritten {
    /// Connection config now carrying a `static_jwt` block.
    pub config_path: PathBuf,
    /// Bearer token slimctl itself authenticates with.
    pub access_token_path: PathBuf,
    /// Refresh token, written for long-lived processes to seed from.
    /// `None` when the IdP issued no refresh token.
    pub refresh_token_path: Option<PathBuf>,
}

/// Record the credential established by `slimctl login` in the connection config,
/// so subsequent commands authenticate with no extra flags.
///
/// Only the `auth` section is touched — `endpoint`, `tls` and everything else in
/// an existing config are preserved, `${env:...}`/`${file:...}` references
/// included (see [`load_config_for_update`]).
///
/// Secrets are never inlined into the config; it only names the files holding
/// them.  slimctl authenticates with the access token alone:
///
/// ```yaml
/// auth:
///   type: static_jwt
///   file: /home/user/.slimctl/token
/// ```
///
/// The refresh token, when the IdP issued one, is written to
/// `~/.slimctl/refresh_token` (mode 0600) for a *long-lived* process to seed
/// from — typically a data-plane node with `refresh_token_file` set, plus its own
/// `refresh_token_out_file` so its rotation chain doesn't clobber this seed.
/// slimctl never spends it: the access token expiring is a prompt to re-run
/// `slimctl login` (see [`auth_failure_hint`]).
pub fn save_login_auth(
    creds: &OidcCredentials,
    config_file: Option<&str>,
    server: Option<&str>,
) -> Result<LoginAuthWritten> {
    // Record the endpoint when `--server` was passed, so `slimctl --server host
    // login` leaves behind a config that points somewhere. Without it the file
    // holds only auth and the endpoint comes from `--server` / the per-subcommand
    // default on each invocation, as before.
    if let Some(server) = server {
        set_config_key("endpoint", &server, config_file)?;
    }
    // slimctl authenticates with the access token *only*.  It deliberately does
    // not consume the refresh token: a refresh token serves one process at a
    // time, and every exchange invalidates the copy on disk.  A CLI that spent a
    // rotation per invocation would race any long-lived process seeded from the
    // same file — and would still have to re-authenticate constantly, since it
    // exits long before background renewal could help.  When the access token
    // expires the user re-runs `slimctl login`; see `auth_failure_hint`.
    let access_token_path = token_file_path()?;
    let auth = AuthenticationConfig::StaticJwt(StaticJwtConfig::with_file(
        access_token_path.to_string_lossy(),
    ));
    let config_path = set_config_key("auth", &auth, config_file)?;

    // The refresh token is written for *other* processes — a data-plane node
    // configured with `refresh_token_file` seeds its rotation chain from here.
    let refresh_token_path = match creds.refresh_token.as_deref() {
        Some(refresh_token) => {
            let path = refresh_token_file_path()?;
            let dir = path
                .parent()
                .expect("refresh token path must have a parent");
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create config directory: {}", dir.display()))?;
            write_private(&path, refresh_token.as_bytes())
                .with_context(|| format!("failed to write refresh token: {}", path.display()))?;
            Some(path)
        }
        None => None,
    };

    Ok(LoginAuthWritten {
        config_path,
        access_token_path,
        refresh_token_path,
    })
}

/// Hint appended to authentication failures, telling the user how to recover.
///
/// A `static_jwt` config carries a fixed access token that simply expires, and a
/// refresh token can be revoked or rotated out from under us; in both cases the
/// fix is the same.
pub fn auth_failure_hint() -> &'static str {
    "authentication failed — your session may have expired; re-run `slimctl login`"
}

/// True when `status` indicates the server rejected our credentials.
pub fn is_auth_failure(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied
    )
}

/// Return the default config file path: `$HOME/.slimctl/config.yaml`
pub fn config_file_path() -> Result<PathBuf> {
    let home = dirs_home().context("could not determine home directory")?;
    Ok(home.join(".slimctl").join("config.yaml"))
}

fn dirs_home() -> Option<PathBuf> {
    // Unix: $HOME. Windows: $USERPROFILE (Git for Windows also sets $HOME, but
    // $USERPROFILE is the canonical variable the OS guarantees).
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
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

    /// Point HOME at a fresh temp directory for the duration of the caller's
    /// scope.  Caller must hold [`HOME_LOCK`].
    #[allow(clippy::disallowed_methods)]
    fn setup_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK held by the caller.
        unsafe { std::env::set_var("HOME", dir.path()) };
        dir
    }

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
        assert!(
            resolve_config(
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
            .is_err()
        );
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
        assert!(
            resolve_config(
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
            .is_err()
        );
    }

    #[test]
    fn resolve_key_without_cert_fails() {
        let file_config = ClientConfig::default();
        assert!(
            resolve_config(
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
            .is_err()
        );
    }

    #[test]
    fn resolve_basic_auth_without_colon_fails() {
        let file_config = ClientConfig::default();
        assert!(
            resolve_config(
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
            .is_err()
        );
    }

    #[test]
    fn resolve_insecure_config_gets_http_scheme() {
        let file_config = ClientConfig::with_endpoint("myhost:1234")
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

    // ── ${env:...} / ${file:...} resolution ─────────────────────────────────

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn load_config_resolves_env_reference() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by HOME_LOCK; no other threads read this var.
        unsafe { std::env::set_var("SLIMCTL_TEST_ENDPOINT", "env-host:7777") };
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::io::Write::write_all(&mut f, b"endpoint: ${env:SLIMCTL_TEST_ENDPOINT}\n").unwrap();
        let config = load_config(Some(f.path().to_str().unwrap())).unwrap();
        assert_eq!(config.endpoint, "env-host:7777");
    }

    #[test]
    fn load_config_resolves_file_reference() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("endpoint.txt");
        std::fs::write(&secret, "file-host:8888").unwrap();
        let cfg = dir.path().join("config.yaml");
        std::fs::write(&cfg, format!("endpoint: ${{file:{}}}\n", secret.display())).unwrap();
        let config = load_config(Some(cfg.to_str().unwrap())).unwrap();
        assert_eq!(config.endpoint, "file-host:8888");
    }

    #[test]
    fn load_config_unresolvable_reference_returns_error() {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::io::Write::write_all(&mut f, b"endpoint: ${file:/nonexistent/token}\n").unwrap();
        assert!(load_config(Some(f.path().to_str().unwrap())).is_err());
    }

    #[test]
    fn load_config_mapping_returns_empty_for_missing_file() {
        let map = load_config_mapping(Some("/nonexistent/path/config.yaml")).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn load_config_mapping_rejects_a_non_mapping_document() {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::io::Write::write_all(&mut f, b"- just\n- a list\n").unwrap();
        assert!(load_config_mapping(Some(f.path().to_str().unwrap())).is_err());
    }

    /// An empty file is a valid starting point, not an error.
    #[test]
    fn load_config_mapping_treats_empty_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "").unwrap();
        assert!(
            load_config_mapping(Some(path.to_str().unwrap()))
                .unwrap()
                .is_empty()
        );
    }

    /// The point of writing at the mapping level: a `${...}` reference in an
    /// untouched key survives, where a deserialize-modify-serialize cycle would
    /// have replaced it with the file's contents.
    #[test]
    fn set_config_key_preserves_references_in_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("endpoint.txt");
        std::fs::write(&secret, "file-host:8888").unwrap();
        let cfg = dir.path().join("config.yaml");
        let reference = format!("${{file:{}}}", secret.display());
        std::fs::write(&cfg, format!("endpoint: {reference}\n")).unwrap();

        set_config_key("request_timeout", &"42s", Some(cfg.to_str().unwrap())).unwrap();

        let raw = std::fs::read_to_string(&cfg).unwrap();
        assert!(raw.contains(&reference), "reference was expanded: {raw}");
        assert!(!raw.contains("file-host:8888"));
        assert!(raw.contains("42s"), "new key missing: {raw}");
    }

    /// Only the requested key is written — no defaulted fields tag along.
    #[test]
    fn set_config_key_writes_only_that_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        set_config_key("endpoint", &"myhost:50051", Some(path.to_str().unwrap())).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.trim(), "endpoint: myhost:50051");
    }

    #[test]
    fn set_config_key_leaves_unrelated_keys_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "endpoint: keep-me:1234\nrequest_timeout: 9s\n").unwrap();

        set_config_key("endpoint", &"changed:1", Some(path.to_str().unwrap())).unwrap();

        let map = load_config_mapping(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(
            map.get(serde_yaml::Value::String("request_timeout".into()))
                .and_then(|v| v.as_str()),
            Some("9s")
        );
        assert_eq!(
            map.get(serde_yaml::Value::String("endpoint".into()))
                .and_then(|v| v.as_str()),
            Some("changed:1")
        );
    }

    #[test]
    fn get_config_key_defaults_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "endpoint: x:1\n").unwrap();
        let tls: TlsClientConfig = get_config_key("tls", Some(path.to_str().unwrap())).unwrap();
        assert_eq!(tls, TlsClientConfig::default());
    }

    // ── save_login_auth ─────────────────────────────────────────────────────

    fn creds_with(refresh_token: Option<&str>) -> OidcCredentials {
        OidcCredentials {
            id_token: "the-id-token".to_string(),
            access_token: Some("the-access-token".to_string()),
            refresh_token: refresh_token.map(str::to_owned),
            client_id: "myclient".to_string(),
            issuer: "https://issuer.example.com".to_string(),
            token_endpoint: "https://issuer.example.com/token".to_string(),
        }
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    /// slimctl authenticates with the access token only; the refresh token is
    /// written to disk for other processes but never referenced by its config.
    fn save_login_auth_uses_access_token_only() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let creds = creds_with(Some("the-refresh-token"));
        save_credentials(&creds).unwrap();
        let written = save_login_auth(&creds, Some(path_str), None).expect("save_login_auth");

        assert_eq!(written.config_path, path);
        assert_eq!(written.access_token_path, token_file_path().unwrap());
        assert_eq!(
            written.refresh_token_path,
            Some(refresh_token_file_path().unwrap())
        );

        let loaded = load_config(Some(path_str)).unwrap();
        let AuthenticationConfig::StaticJwt(jwt) = loaded.auth else {
            panic!("expected static_jwt auth, got {:?}", loaded.auth);
        };
        assert_eq!(
            jwt.source().file,
            token_file_path().unwrap().to_str().unwrap()
        );

        // Neither token may appear in the config, and it must not point at the
        // refresh token in any form.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("the-refresh-token"), "leaked: {raw}");
        assert!(!raw.contains("the-access-token"), "leaked: {raw}");
        assert!(!raw.contains("refresh_token"), "references refresh: {raw}");
    }

    /// `slimctl --server host login` should leave behind a config that points
    /// somewhere, not just an auth block.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_records_server_when_given() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let creds = creds_with(Some("the-refresh-token"));
        save_credentials(&creds).unwrap();
        save_login_auth(&creds, Some(path_str), Some("ctrl.example.com:50051")).unwrap();

        let loaded = load_config(Some(path_str)).unwrap();
        assert_eq!(loaded.endpoint, "ctrl.example.com:50051");
        assert!(matches!(loaded.auth, AuthenticationConfig::StaticJwt(_)));
    }

    /// Recording a bare `host:port` means the endpoint now comes from the file,
    /// which `resolve_config` treats as intentionally secure — so it resolves to
    /// https. Including a scheme in `--server` is how a plaintext endpoint is
    /// recorded.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn recorded_endpoint_scheme_follows_what_was_passed() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let creds = creds_with(Some("rt"));
        save_credentials(&creds).unwrap();

        for (passed, expected) in [
            ("ctrl.example.com:50051", "https://ctrl.example.com:50051"),
            (
                "http://ctrl.example.com:50051",
                "http://ctrl.example.com:50051",
            ),
        ] {
            let path = home.path().join(format!("cfg-{}.yaml", expected.len()));
            let path_str = path.to_str().unwrap();
            save_login_auth(&creds, Some(path_str), Some(passed)).unwrap();
            let opts = resolve_config(
                &load_config(Some(path_str)).unwrap(),
                DEFAULT_CONTROLLER_ENDPOINT,
                None,
                None,
                false,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(opts.endpoint, expected, "for --server {passed}");
        }
    }

    /// Without `--server` the endpoint is left alone rather than blanked.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_preserves_existing_endpoint_without_server() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, "endpoint: already-set:1234\n").unwrap();

        let creds = creds_with(Some("the-refresh-token"));
        save_credentials(&creds).unwrap();
        save_login_auth(&creds, Some(path_str), None).unwrap();

        let loaded = load_config(Some(path_str)).unwrap();
        assert_eq!(loaded.endpoint, "already-set:1234");
    }

    /// …and `--server` replaces one that was already there.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_server_overrides_existing_endpoint() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, "endpoint: old-host:1\n").unwrap();

        let creds = creds_with(Some("the-refresh-token"));
        save_credentials(&creds).unwrap();
        save_login_auth(&creds, Some(path_str), Some("new-host:2")).unwrap();

        assert_eq!(load_config(Some(path_str)).unwrap().endpoint, "new-host:2");
    }

    /// The refresh token is still persisted — a data-plane node seeds from it.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_writes_refresh_token_for_other_processes() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");

        save_login_auth(
            &creds_with(Some("the-refresh-token")),
            Some(path.to_str().unwrap()),
            None,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(refresh_token_file_path().unwrap()).unwrap(),
            "the-refresh-token"
        );
    }

    #[test]
    #[cfg(unix)]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_refresh_token_file_is_private() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        save_login_auth(
            &creds_with(Some("the-refresh-token")),
            Some(path.to_str().unwrap()),
            None,
        )
        .unwrap();

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(refresh_token_file_path().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "got mode {:o}", mode & 0o777);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_without_refresh_token_uses_static_jwt() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let creds = creds_with(None);
        // login always writes the bearer token file first.
        save_credentials(&creds).unwrap();

        let written = save_login_auth(&creds, Some(path_str), None).expect("save_login_auth");
        assert_eq!(written.access_token_path, token_file_path().unwrap());
        // No refresh token was issued, so none is persisted.
        assert!(written.refresh_token_path.is_none());
        assert!(!refresh_token_file_path().unwrap().exists());

        let loaded = load_config(Some(path_str)).unwrap();
        let AuthenticationConfig::StaticJwt(jwt) = loaded.auth else {
            panic!("expected static_jwt auth, got {:?}", loaded.auth);
        };
        assert_eq!(
            jwt.source().file,
            token_file_path().unwrap().to_str().unwrap()
        );
        // The token itself is only ever in the token file.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("the-access-token"), "token leaked: {raw}");
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_preserves_existing_settings() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let existing = ClientConfig::with_endpoint("myhost:50051")
            .with_tls_setting(TlsClientConfig::new().with_ca_file("/etc/ca.pem"))
            .with_request_timeout(Duration::from_secs(42));
        save_config(&existing, Some(path_str)).unwrap();

        save_login_auth(&creds_with(Some("the-refresh-token")), Some(path_str), None).unwrap();

        let loaded = load_config(Some(path_str)).unwrap();
        assert_eq!(loaded.endpoint, "myhost:50051");
        assert!(!loaded.tls_setting.insecure);
        assert_eq!(
            Duration::from(loaded.request_timeout),
            Duration::from_secs(42)
        );
        assert!(matches!(loaded.auth, AuthenticationConfig::StaticJwt(_)));
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_creates_config_when_absent() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("nested").join("config.yaml");
        save_login_auth(
            &creds_with(Some("the-refresh-token")),
            Some(path.to_str().unwrap()),
            None,
        )
        .unwrap();
        assert!(path.exists());
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_overwrites_previous_auth() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let mut existing = ClientConfig::with_endpoint("myhost:50051");
        existing.auth = AuthenticationConfig::Basic(BasicAuthConfig::new("u", "p"));
        save_config(&existing, Some(path_str)).unwrap();

        save_login_auth(&creds_with(Some("the-refresh-token")), Some(path_str), None).unwrap();
        let loaded = load_config(Some(path_str)).unwrap();
        assert!(matches!(loaded.auth, AuthenticationConfig::StaticJwt(_)));
    }

    /// Re-running login must not accumulate stale references or leak the old
    /// secret once resolution is in play.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn save_login_auth_is_idempotent_across_relogin() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        save_login_auth(&creds_with(Some("first-token")), Some(path_str), None).unwrap();
        save_login_auth(&creds_with(Some("second-token")), Some(path_str), None).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("first-token"), "old token baked in: {raw}");
        assert!(!raw.contains("second-token"), "new token baked in: {raw}");

        // The file the config points at holds the latest token.
        assert_eq!(
            std::fs::read_to_string(refresh_token_file_path().unwrap()).unwrap(),
            "second-token"
        );
        let loaded = load_config(Some(path_str)).unwrap();
        assert!(matches!(loaded.auth, AuthenticationConfig::StaticJwt(_)));
    }

    /// A login-written config must take effect through `resolve_config` without
    /// depending on the implicit credentials-file fallback.
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn resolve_config_uses_login_written_auth() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        let path = home.path().join("config.yaml");
        let path_str = path.to_str().unwrap();
        save_login_auth(&creds_with(Some("the-refresh-token")), Some(path_str), None).unwrap();

        let file_config = load_config(Some(path_str)).unwrap();
        let opts = resolve_config(
            &file_config,
            DEFAULT_CONTROLLER_ENDPOINT,
            Some("myhost:50051"),
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let AuthenticationConfig::StaticJwt(jwt) = opts.auth else {
            panic!("expected static_jwt auth to survive resolve_config");
        };
        assert_eq!(
            jwt.source().file,
            token_file_path().unwrap().to_str().unwrap()
        );
    }

    // ── auth failure hint ───────────────────────────────────────────────────

    #[test]
    fn is_auth_failure_detects_rejected_credentials() {
        assert!(is_auth_failure(&tonic::Status::unauthenticated("nope")));
        assert!(is_auth_failure(&tonic::Status::permission_denied("nope")));
        assert!(!is_auth_failure(&tonic::Status::unavailable("down")));
        assert!(!is_auth_failure(&tonic::Status::internal("boom")));
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

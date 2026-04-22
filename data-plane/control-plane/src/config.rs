// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

use slim_config::grpc::server::ServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_tracing::TracingConfiguration;

/// Top-level control-plane configuration.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Northbound gRPC API (management / ControlPlaneService).
    pub northbound: ServerConfig,
    /// Southbound gRPC API (node registration / ControllerService).
    pub southbound: ServerConfig,
    /// Settings for the route and link reconcilers.
    pub reconciler: ReconcilerConfig,
    /// Database backend configuration.
    pub database: DatabaseConfig,
    /// Tracing / logging configuration.
    pub tracing: TracingConfiguration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            northbound: ServerConfig {
                endpoint: "0.0.0.0:50051".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            },
            southbound: ServerConfig {
                endpoint: "0.0.0.0:50052".to_string(),
                tls_setting: TlsServerConfig::insecure(),
                ..Default::default()
            },
            reconciler: ReconcilerConfig::default(),
            database: DatabaseConfig::default(),
            tracing: TracingConfiguration::default(),
        }
    }
}

impl Config {
    /// Apply environment-variable overrides, mirroring the Go `OverrideFromEnv` method.
    ///
    /// Supported variables:
    /// - `NB_API_HTTP_HOST` / `NB_API_HTTP_PORT` — override northbound endpoint host/port
    /// - `SB_API_HTTP_HOST` / `SB_API_HTTP_PORT` — override southbound endpoint host/port
    /// - `API_LOG_LEVEL`     — override log level (trace/debug/info/warn/error)
    /// - `DATABASE_FILEPATH` — switch to SQLite at the given path
    pub fn apply_env_overrides(&mut self) {
        apply_endpoint_env(
            &mut self.northbound.endpoint,
            "NB_API_HTTP_HOST",
            "NB_API_HTTP_PORT",
        );
        apply_endpoint_env(
            &mut self.southbound.endpoint,
            "SB_API_HTTP_HOST",
            "SB_API_HTTP_PORT",
        );

        if let Ok(level) = std::env::var("API_LOG_LEVEL")
            && !level.is_empty()
        {
            self.tracing = std::mem::take(&mut self.tracing).with_log_level(level);
        }
        if let Ok(path) = std::env::var("DATABASE_FILEPATH")
            && !path.is_empty()
        {
            self.database = DatabaseConfig::Sqlite { path };
        }
    }
}

/// Rewrite `endpoint` ("host:port") using env-var overrides for host and/or port.
fn apply_endpoint_env(endpoint: &mut String, host_var: &str, port_var: &str) {
    // Parse existing endpoint into host + port (best-effort).
    let (mut host, mut port) = split_endpoint(endpoint);

    if let Ok(h) = std::env::var(host_var)
        && !h.is_empty()
    {
        host = h;
    }
    if let Ok(p) = std::env::var(port_var)
        && !p.is_empty()
    {
        port = p;
    }

    *endpoint = format!("{host}:{port}");
}

/// Split "host:port" into (host, port). Falls back gracefully on parse errors.
fn split_endpoint(endpoint: &str) -> (String, String) {
    if let Some(pos) = endpoint.rfind(':') {
        let host = endpoint[..pos].to_string();
        let port = endpoint[pos + 1..].to_string();
        return (host, port);
    }
    (endpoint.to_string(), String::new())
}

/// Database backend selection.
#[derive(Debug, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseConfig {
    /// Pure in-memory store (default). All state is lost on restart.
    #[default]
    InMemory,
    /// SQLite-backed persistent store.
    Sqlite {
        /// Path to the SQLite database file.
        path: String,
    },
}

/// Reconciler tuning parameters.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ReconcilerConfig {
    /// Maximum number of times a failed reconcile request is requeued.
    pub max_requeues: usize,
    /// Maximum number of route/link reconciles running concurrently.
    pub max_parallel_reconciles: usize,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            max_requeues: 15,
            max_parallel_reconciles: 1000,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // set_var / remove_var are acceptable in tests
mod tests {
    use super::*;

    #[test]
    fn split_endpoint_host_port() {
        let (h, p) = split_endpoint("localhost:50051");
        assert_eq!(h, "localhost");
        assert_eq!(p, "50051");
    }

    #[test]
    fn split_endpoint_no_port() {
        let (h, p) = split_endpoint("localhost");
        assert_eq!(h, "localhost");
        assert_eq!(p, "");
    }

    #[test]
    fn split_endpoint_ipv6() {
        let (h, p) = split_endpoint("[::1]:9000");
        assert_eq!(h, "[::1]");
        assert_eq!(p, "9000");
    }

    #[test]
    fn reconciler_config_defaults() {
        let c = ReconcilerConfig::default();
        assert_eq!(c.max_requeues, 15);
        assert_eq!(c.max_parallel_reconciles, 1000);
    }

    #[test]
    fn config_defaults() {
        let c = Config::default();
        assert_eq!(c.northbound.endpoint, "0.0.0.0:50051");
        assert_eq!(c.southbound.endpoint, "0.0.0.0:50052");
    }

    #[test]
    fn apply_env_overrides_host_port() {
        // Use unique env var names to avoid cross-test pollution.
        // SAFETY: single-threaded test context; no concurrent env access.
        unsafe {
            std::env::set_var("TEST_NB_HOST", "0.0.0.0");
            std::env::set_var("TEST_NB_PORT", "9999");
        }
        let mut endpoint = "localhost:50051".to_string();
        apply_endpoint_env(&mut endpoint, "TEST_NB_HOST", "TEST_NB_PORT");
        assert_eq!(endpoint, "0.0.0.0:9999");
        unsafe {
            std::env::remove_var("TEST_NB_HOST");
            std::env::remove_var("TEST_NB_PORT");
        }
    }

    #[test]
    fn apply_env_overrides_only_host() {
        unsafe {
            std::env::set_var("TEST_ONLY_HOST", "myhost");
        }
        let mut endpoint = "localhost:1234".to_string();
        apply_endpoint_env(&mut endpoint, "TEST_ONLY_HOST", "NON_EXISTENT_PORT_VAR");
        assert_eq!(endpoint, "myhost:1234");
        unsafe {
            std::env::remove_var("TEST_ONLY_HOST");
        }
    }

    #[test]
    fn apply_env_overrides_database_path() {
        // SAFETY: single-threaded test context; no concurrent env access.
        unsafe {
            std::env::remove_var("NB_API_HTTP_HOST");
            std::env::remove_var("NB_API_HTTP_PORT");
            std::env::remove_var("SB_API_HTTP_HOST");
            std::env::remove_var("SB_API_HTTP_PORT");
            std::env::remove_var("API_LOG_LEVEL");
            std::env::set_var("DATABASE_FILEPATH", "/tmp/test.db");
        }
        let mut cfg = Config::default();
        cfg.apply_env_overrides();
        match cfg.database {
            DatabaseConfig::Sqlite { path } => assert_eq!(path, "/tmp/test.db"),
            _ => panic!("expected Sqlite variant"),
        }
        unsafe {
            std::env::remove_var("DATABASE_FILEPATH");
        }
    }

    #[test]
    fn apply_env_overrides_empty_vars_are_ignored() {
        unsafe {
            std::env::set_var("NB_API_HTTP_HOST", "");
            std::env::set_var("NB_API_HTTP_PORT", "");
        }
        let mut cfg = Config::default();
        cfg.apply_env_overrides();
        // Endpoint must not become ":"
        assert!(!cfg.northbound.endpoint.starts_with(':'));
        unsafe {
            std::env::remove_var("NB_API_HTTP_HOST");
            std::env::remove_var("NB_API_HTTP_PORT");
        }
    }
}

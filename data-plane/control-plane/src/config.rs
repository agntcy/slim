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
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReconcilerConfig {
    /// Maximum number of times a failed reconcile request is requeued.
    pub max_requeues: usize,
    /// Base delay in milliseconds for the first retry. Subsequent retries use
    /// exponential backoff (base * 2^(attempt-1)) capped at 30 s.
    pub base_retry_delay_ms: u64,
    /// How often (in seconds) all connected nodes are re-enqueued for full
    /// reconciliation as a background health-check sweep. Set to 0 to disable.
    pub reconcile_period_secs: u64,
    /// When true, the link reconciler will delete outgoing connections found on
    /// a data-plane node whose link_id is not present in the control-plane DB.
    ///
    /// Disable this (the default) when data-plane nodes may have connections
    /// that were established outside the control plane (e.g. connections created
    /// by a previous CP instance, or manually configured connections). Enabling
    /// it is useful in greenfield deployments where the CP is the sole source of
    /// truth for all data-plane connections.
    pub enable_orphan_detection: bool,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            max_requeues: 15,
            base_retry_delay_ms: 200,
            reconcile_period_secs: 60,
            enable_orphan_detection: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciler_config_defaults() {
        let c = ReconcilerConfig::default();
        assert_eq!(c.max_requeues, 15);
        assert_eq!(c.base_retry_delay_ms, 200);
    }

    #[test]
    fn config_defaults() {
        let c = Config::default();
        assert_eq!(c.northbound.endpoint, "0.0.0.0:50051");
        assert_eq!(c.southbound.endpoint, "0.0.0.0:50052");
    }
}

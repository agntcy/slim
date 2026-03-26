// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! WASM-compatible tracing configuration.
//! Uses console-based tracing subscriber instead of OpenTelemetry.

use serde::Deserialize;
use thiserror::Error;
use tracing::Level;
use tracing_subscriber::{Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("error setting up tracing subscriber")]
    TracingSetupError(#[from] tracing_subscriber::util::TryInitError),
}

#[derive(Clone, Debug, Deserialize)]
pub struct TracingConfiguration {
    #[serde(default = "default_log_level")]
    log_level: String,

    #[serde(default = "default_filter")]
    filters: Vec<String>,
}

impl Default for TracingConfiguration {
    fn default() -> Self {
        TracingConfiguration {
            log_level: default_log_level(),
            filters: default_filter(),
        }
    }
}

fn default_log_level() -> String { "info".to_string() }

fn default_filter() -> Vec<String> {
    vec![
        "slim_datapath".to_string(), "slim_service".to_string(),
        "slim_controller".to_string(), "slim_auth".to_string(),
        "slim_config".to_string(), "slim_mls".to_string(),
        "slim_session".to_string(), "slim_signal".to_string(),
        "slim_tracing".to_string(), "_slim_bindings".to_string(),
        "slim".to_string(),
    ]
}

fn resolve_level(level: &str) -> Level {
    match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Guard type for compatibility with the native API.
/// In WASM there is nothing to shut down.
pub struct OtelGuard;

impl TracingConfiguration {
    pub fn with_log_level(self, log_level: String) -> Self {
        TracingConfiguration { log_level, ..self }
    }

    pub fn with_filter(self, filter: Vec<String>) -> Self {
        TracingConfiguration { filters: filter, ..self }
    }

    pub fn log_level(&self) -> &str { &self.log_level }
    pub fn filter(&self) -> &Vec<String> { &self.filters }

    pub fn setup_tracing_subscriber(&self) -> Result<OtelGuard, ConfigError> {
        let fmt_layer = fmt::layer()
            .with_ansi(false)
            .without_time()
            .with_line_number(true)
            .with_filter(tracing_subscriber::filter::filter_fn(
                |metadata: &tracing::Metadata| {
                    !metadata.fields().iter().any(|field| field.name() == "telemetry")
                },
            ));

        let level = resolve_level(&self.log_level);
        let level_filter = tracing_subscriber::filter::LevelFilter::from_level(level);

        tracing_subscriber::registry()
            .with(level_filter)
            .with(fmt_layer)
            .try_init()?;

        Ok(OtelGuard)
    }
}

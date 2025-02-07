// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use tracing::Level;

pub mod opaque;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TracingConfiguration {
    #[serde(default = "default_log_level")]
    log_level: String,

    #[serde(default = "default_display_thread_names")]
    display_thread_names: bool,

    #[serde(default = "default_display_thread_ids")]
    display_thread_ids: bool,

    #[serde(default = "default_filter")]
    filter: String,
}

// default implementation for TracingConfiguration
impl Default for TracingConfiguration {
    fn default() -> Self {
        TracingConfiguration {
            log_level: default_log_level(),
            display_thread_names: default_display_thread_names(),
            display_thread_ids: default_display_thread_ids(),
            filter: default_filter(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_display_thread_names() -> bool {
    true
}

fn default_display_thread_ids() -> bool {
    false
}

fn default_filter() -> String {
    "info".to_string()
}

// function to convert string tracing level to tracing::Level
fn resolve_level(level: &str) -> tracing::Level {
    let level = level.to_lowercase();
    match level.as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO, // default level
    }
}

impl TracingConfiguration {
    pub fn with_log_level(self, log_level: String) -> Self {
        TracingConfiguration { log_level, ..self }
    }

    pub fn with_display_thread_names(self, display_thread_names: bool) -> Self {
        TracingConfiguration {
            display_thread_names,
            ..self
        }
    }

    pub fn with_display_thread_ids(self, display_thread_ids: bool) -> Self {
        TracingConfiguration {
            display_thread_ids,
            ..self
        }
    }

    pub fn with_filter(self, filter: String) -> Self {
        TracingConfiguration { filter, ..self }
    }

    pub fn log_level(&self) -> &str {
        &self.log_level
    }

    pub fn display_thread_names(&self) -> bool {
        self.display_thread_names
    }

    pub fn display_thread_ids(&self) -> bool {
        self.display_thread_ids
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Set up a subscriber that logs to stdout
    pub fn setup_tracing_subscriber(&self) {
        tracing_subscriber::fmt::Subscriber::builder()
            .with_max_level(resolve_level(&self.log_level)) // Set the max log level
            .with_thread_names(self.display_thread_names)
            .with_thread_ids(self.display_thread_ids)
            .init()
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tracing_configuration() {
        let config = TracingConfiguration::default();
        assert_eq!(config.log_level, default_log_level());
        assert_eq!(config.display_thread_names, default_display_thread_names());
        assert_eq!(config.display_thread_ids, default_display_thread_ids());
        assert_eq!(config.filter, default_filter());
    }

    #[test]
    fn test_resolve_level() {
        assert_eq!(resolve_level("trace"), Level::TRACE);
        assert_eq!(resolve_level("debug"), Level::DEBUG);
        assert_eq!(resolve_level("info"), Level::INFO);
        assert_eq!(resolve_level("warn"), Level::WARN);
        assert_eq!(resolve_level("error"), Level::ERROR);
        assert_eq!(resolve_level("invalid"), Level::INFO);
    }
}

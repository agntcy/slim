// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! UniFFI-compatible configuration wrappers for initialization
//!
//! This module provides wrapper types for RuntimeConfiguration, TracingConfiguration,
//! and ServiceConfiguration that can be passed through UniFFI bindings.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use slim::runtime::RuntimeConfiguration as CoreRuntimeConfiguration;
use slim_service::ServiceConfiguration as CoreServiceConfiguration;
use slim_tracing::TracingConfiguration as CoreTracingConfiguration;

use crate::service::DataplaneConfig;

/// Runtime configuration for the SLIM bindings
///
/// Controls the Tokio runtime behavior including thread count, naming, and shutdown timeout.
#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Number of cores to use for the runtime (0 = use all available cores)
    pub n_cores: u64,

    /// Thread name prefix for the runtime
    pub thread_name: String,

    /// Timeout duration for draining services during shutdown
    pub drain_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let core = CoreRuntimeConfiguration::default();
        RuntimeConfig {
            n_cores: core.n_cores() as u64,
            thread_name: core.thread_name().to_string(),
            drain_timeout: core.drain_timeout(),
        }
    }
}

impl From<RuntimeConfig> for CoreRuntimeConfiguration {
    fn from(config: RuntimeConfig) -> Self {
        CoreRuntimeConfiguration {
            n_cores: config.n_cores as usize,
            thread_name: config.thread_name,
            drain_timeout: config.drain_timeout.into(),
        }
    }
}

impl From<CoreRuntimeConfiguration> for RuntimeConfig {
    fn from(config: CoreRuntimeConfiguration) -> Self {
        RuntimeConfig {
            n_cores: config.n_cores() as u64,
            thread_name: config.thread_name().to_string(),
            drain_timeout: config.drain_timeout(),
        }
    }
}

/// Tracing/logging configuration for the SLIM bindings
///
/// Controls logging behavior including log level, thread name/ID display, and filters.
#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize)]
pub struct TracingConfig {
    /// Log level (e.g., "debug", "info", "warn", "error")
    pub log_level: String,

    /// Whether to display thread names in logs
    pub display_thread_names: bool,

    /// Whether to display thread IDs in logs
    pub display_thread_ids: bool,

    /// List of tracing filter directives (e.g., ["slim=debug", "tokio=info"])
    pub filters: Vec<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        let core = CoreTracingConfiguration::default();
        TracingConfig {
            log_level: core.log_level().to_string(),
            display_thread_names: core.display_thread_names(),
            display_thread_ids: core.display_thread_ids(),
            filters: core.filter().to_vec(),
        }
    }
}

impl From<TracingConfig> for CoreTracingConfiguration {
    fn from(config: TracingConfig) -> Self {
        CoreTracingConfiguration::default()
            .with_log_level(config.log_level)
            .with_display_thread_names(config.display_thread_names)
            .with_display_thread_ids(config.display_thread_ids)
            .with_filter(config.filters)
    }
}

impl From<CoreTracingConfiguration> for TracingConfig {
    fn from(config: CoreTracingConfiguration) -> Self {
        TracingConfig {
            log_level: config.log_level().to_string(),
            display_thread_names: config.display_thread_names(),
            display_thread_ids: config.display_thread_ids(),
            filters: config.filter().to_vec(),
        }
    }
}

/// Service configuration for the SLIM bindings
///
/// Controls service behavior including node ID, group name, and dataplane settings.
#[derive(uniffi::Record, Clone)]
pub struct ServiceConfig {
    /// Optional node ID for the service
    pub node_id: Option<String>,

    /// Optional group name for the service
    pub group_name: Option<String>,

    /// DataPlane configuration (servers and clients)
    pub dataplane: DataplaneConfig,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        let core = CoreServiceConfiguration::default();
        ServiceConfig {
            node_id: core.node_id.clone(),
            group_name: core.group_name.clone(),
            dataplane: DataplaneConfig::from(core.dataplane.clone()),
        }
    }
}

impl From<ServiceConfig> for CoreServiceConfiguration {
    fn from(config: ServiceConfig) -> Self {
        CoreServiceConfiguration {
            node_id: config.node_id,
            group_name: config.group_name,
            dataplane: config.dataplane.into(),
            ..Default::default()
        }
    }
}

impl From<CoreServiceConfiguration> for ServiceConfig {
    fn from(config: CoreServiceConfiguration) -> Self {
        ServiceConfig {
            node_id: config.node_id,
            group_name: config.group_name,
            dataplane: DataplaneConfig::from(config.dataplane),
        }
    }
}

// Constructor functions for UniFFI

/// Create a new BindingsRuntimeConfig with default values
#[uniffi::export]
pub fn new_runtime_config() -> RuntimeConfig {
    RuntimeConfig::default()
}

/// Create a new BindingsRuntimeConfig with custom values
#[uniffi::export]
pub fn new_runtime_config_with(
    n_cores: u64,
    thread_name: String,
    drain_timeout: Duration,
) -> RuntimeConfig {
    RuntimeConfig {
        n_cores,
        thread_name,
        drain_timeout,
    }
}

/// Create a new BindingsTracingConfig with default values
#[uniffi::export]
pub fn new_tracing_config() -> TracingConfig {
    TracingConfig::default()
}

/// Create a new BindingsTracingConfig with custom values
#[uniffi::export]
pub fn new_tracing_config_with(
    log_level: String,
    display_thread_names: bool,
    display_thread_ids: bool,
    filters: Vec<String>,
) -> TracingConfig {
    TracingConfig {
        log_level,
        display_thread_names,
        display_thread_ids,
        filters,
    }
}

/// Create a new BindingsServiceConfig with default values
#[uniffi::export]
pub fn new_service_config() -> ServiceConfig {
    ServiceConfig::default()
}

/// Create a new BindingsServiceConfig with custom values
#[uniffi::export]
pub fn new_service_config_with(
    node_id: Option<String>,
    group_name: Option<String>,
    dataplane: DataplaneConfig,
) -> ServiceConfig {
    ServiceConfig {
        node_id,
        group_name,
        dataplane,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_configuration_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.n_cores, 0); // 0 means use all cores
        assert_eq!(config.thread_name, "slim");
        assert_eq!(config.drain_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_runtime_configuration_roundtrip() {
        let config = RuntimeConfig {
            n_cores: 4,
            thread_name: "test-runtime".to_string(),
            drain_timeout: Duration::from_secs(30),
        };

        let core: CoreRuntimeConfiguration = config.clone().into();
        let back: RuntimeConfig = core.into();

        assert_eq!(back.n_cores, config.n_cores);
        assert_eq!(back.thread_name, config.thread_name);
        assert_eq!(back.drain_timeout, config.drain_timeout);
    }

    #[test]
    fn test_tracing_configuration_default() {
        let config = TracingConfig::default();
        assert_eq!(config.log_level, "info");
        // Note: display_thread_names, display_thread_ids, and filters defaults come from core
        // Just verify we can read them
        let _ = config.display_thread_names;
        let _ = config.display_thread_ids;
        let _ = &config.filters;
    }

    #[test]
    fn test_tracing_configuration_roundtrip() {
        let config = TracingConfig {
            log_level: "debug".to_string(),
            display_thread_names: true,
            display_thread_ids: true,
            filters: vec!["slim=debug".to_string(), "tokio=info".to_string()],
        };

        let core: CoreTracingConfiguration = config.clone().into();
        let back: TracingConfig = core.into();

        assert_eq!(back.log_level, config.log_level);
        assert_eq!(back.display_thread_names, config.display_thread_names);
        assert_eq!(back.display_thread_ids, config.display_thread_ids);
        assert_eq!(back.filters, config.filters);
    }

    #[test]
    fn test_service_configuration_default() {
        let config = ServiceConfig::default();
        assert!(config.node_id.is_none());
        assert!(config.group_name.is_none());
    }

    #[test]
    fn test_service_configuration_roundtrip() {
        let config = ServiceConfig {
            node_id: Some("test-node".to_string()),
            group_name: Some("test-group".to_string()),
            dataplane: DataplaneConfig::default(),
        };

        let core: CoreServiceConfiguration = config.clone().into();
        let back: ServiceConfig = core.into();

        assert_eq!(back.node_id, config.node_id);
        assert_eq!(back.group_name, config.group_name);
    }

    #[test]
    fn test_constructor_functions() {
        let runtime = new_runtime_config();
        assert_eq!(runtime.n_cores, 0);

        let runtime_custom =
            new_runtime_config_with(4, "custom".to_string(), Duration::from_secs(20));
        assert_eq!(runtime_custom.n_cores, 4);
        assert_eq!(runtime_custom.thread_name, "custom");
        assert_eq!(runtime_custom.drain_timeout, Duration::from_secs(20));

        let tracing = new_tracing_config();
        assert_eq!(tracing.log_level, "info");

        let tracing_custom = new_tracing_config_with(
            "debug".to_string(),
            true,
            true,
            vec!["slim=debug".to_string()],
        );
        assert_eq!(tracing_custom.log_level, "debug");
        assert!(tracing_custom.display_thread_names);

        let service = new_service_config();
        assert!(service.node_id.is_none());

        let service_custom = new_service_config_with(
            Some("node-1".to_string()),
            Some("group-1".to_string()),
            DataplaneConfig::default(),
        );
        assert_eq!(service_custom.node_id, Some("node-1".to_string()));
        assert_eq!(service_custom.group_name, Some("group-1".to_string()));
    }
}

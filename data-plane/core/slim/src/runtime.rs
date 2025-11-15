// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_string::DurationString;
use serde::{Deserialize, Serialize};
use std::time;
use tokio::runtime::{Builder, Runtime};
use tracing::{info, warn};

use slim_config::component::configuration::ConfigurationError;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeConfiguration {
    /// the number of cores to use for this runtime
    #[serde(default = "default_n_cores")]
    n_cores: usize,

    /// the thread name for the runtime
    #[serde(default = "default_thread_name")]
    thread_name: String,

    /// the timeout for draining the services
    #[serde(default = "default_drain_timeout")]
    drain_timeout: DurationString,
}

impl Default for RuntimeConfiguration {
    fn default() -> Self {
        RuntimeConfiguration {
            n_cores: default_n_cores(),
            thread_name: default_thread_name(),
            drain_timeout: default_drain_timeout(),
        }
    }
}

fn default_n_cores() -> usize {
    // 0 means use all available cores
    0
}

fn default_thread_name() -> String {
    "slim".to_string()
}

fn default_drain_timeout() -> DurationString {
    time::Duration::from_secs(10).into()
}

impl RuntimeConfiguration {
    pub fn new() -> Self {
        RuntimeConfiguration::default()
    }

    pub fn with_cores(n_cores: usize) -> Self {
        RuntimeConfiguration {
            n_cores,
            ..RuntimeConfiguration::default()
        }
    }

    pub fn with_thread_name(thread_name: &str) -> Self {
        RuntimeConfiguration {
            thread_name: thread_name.to_string(),
            ..RuntimeConfiguration::default()
        }
    }

    pub fn with_drain_timeout(drain_timeout: time::Duration) -> Self {
        RuntimeConfiguration {
            drain_timeout: drain_timeout.into(),
            ..RuntimeConfiguration::default()
        }
    }

    pub fn n_cores(&self) -> usize {
        self.n_cores
    }

    pub fn thread_name(&self) -> &str {
        &self.thread_name
    }

    pub fn drain_timeout(&self) -> time::Duration {
        self.drain_timeout.into()
    }
}

pub struct SlimRuntime {
    // Configuration field
    pub config: RuntimeConfiguration,

    // The actual runtime
    pub runtime: Runtime,
}

pub fn build(config: &RuntimeConfiguration) -> Result<SlimRuntime, ConfigurationError> {
    let n_cpu = num_cpus::get();
    debug_assert!(n_cpu > 0, "failed to get number of CPUs");

    tracing::debug!(%n_cpu, "Number of available CPU cores");

    let cores = if config.n_cores > n_cpu {
        warn!(
            "Requested number of cores ({}) is greater than available cores ({}). Using all available cores",
            config.n_cores, n_cpu
        );
        n_cpu
    } else if config.n_cores == 0 {
        info!(
            %n_cpu,
            "Using all available cores",
        );
        n_cpu
    } else {
        config.n_cores
    };

    let runtime = match cores {
        1 => {
            info!("Using single-threaded runtime");
            Builder::new_current_thread()
                .enable_all()
                .thread_name(config.thread_name.as_str())
                .build()
                .expect("failed to build single-thread runtime!")
        }
        _ => {
            info!(%cores, "Using multi-threaded runtime");
            Builder::new_multi_thread()
                .enable_all()
                .thread_name(config.thread_name.as_str())
                .worker_threads(cores)
                .max_blocking_threads(cores)
                .build()
                .expect("failed to build threaded runtime!")
        }
    };

    Ok(SlimRuntime {
        config: config.clone(),
        runtime,
    })
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_runtime_configuration() {
        let config = RuntimeConfiguration::default();
        assert_eq!(config.n_cores, 0);
        assert_eq!(config.thread_name, "slim");
        assert_eq!(config.drain_timeout, time::Duration::from_secs(10));

        let config = RuntimeConfiguration {
            n_cores: 1,
            thread_name: "test".to_string(),
            drain_timeout: time::Duration::from_secs(5).into(),
        };
        assert_eq!(config.n_cores, 1);
        assert_eq!(config.thread_name, "test");
        assert_eq!(config.drain_timeout, time::Duration::from_secs(5));
    }

    #[test]
    fn test_runtime_builder() {
        let config = RuntimeConfiguration::default();
        let runtime = build(&config).unwrap();
        assert_eq!(runtime.config.n_cores, 0);
    }

    #[test]
    fn test_runtime_builder_with_cores() {
        let config = RuntimeConfiguration {
            n_cores: 3,
            thread_name: "test".to_string(),
            drain_timeout: time::Duration::from_secs(10).into(),
        };
        let runtime = build(&config).unwrap();
        assert_eq!(runtime.config.n_cores, 3);
        assert_eq!(config.drain_timeout, time::Duration::from_secs(10));
    }

    #[test]
    fn test_runtime_builder_with_invalid_cores() {
        let config = RuntimeConfiguration {
            n_cores: 100,
            thread_name: "test".to_string(),
            drain_timeout: time::Duration::from_secs(10).into(),
        };
        let runtime = build(&config).unwrap();
        assert_eq!(runtime.config.n_cores, 100);
        assert_eq!(config.drain_timeout, time::Duration::from_secs(10));
    }

    #[test]
    fn test_runtime_configuration_valid_drain_timeout_deserialize() {
        let json = r#"{ "drain_timeout": "1m30s" }"#; // 90 seconds
        let cfg: RuntimeConfiguration =
            serde_json::from_str(json).expect("valid duration should deserialize");
        assert_eq!(cfg.drain_timeout, time::Duration::from_secs(90));

        let json = r#"{ "drain_timeout": "2h3m4s" }"#; // 7384 seconds
        let cfg: RuntimeConfiguration =
            serde_json::from_str(json).expect("complex duration should deserialize");
        assert_eq!(
            cfg.drain_timeout,
            time::Duration::from_secs(2 * 3600 + 3 * 60 + 4)
        );
    }

    #[test]
    fn test_runtime_configuration_invalid_drain_timeout_deserialize() {
        let invalid_cases = [
            r#"{ "drain_timeout": "abc" }"#,
            r#"{ "drain_timeout": "10x" }"#,
            r#"{ "drain_timeout": "-5s" }"#,
        ];
        for js in invalid_cases {
            let res: Result<RuntimeConfiguration, _> = serde_json::from_str(js);
            assert!(res.is_err(), "expected error for json: {}", js);
        }
    }

    #[test]
    fn test_runtime_configuration_drain_timeout_roundtrip() {
        let cfg = RuntimeConfiguration {
            n_cores: 0,
            thread_name: "roundtrip".to_string(),
            drain_timeout: time::Duration::from_millis(1250).into(),
        };
        let ser = serde_json::to_string(&cfg).expect("serialize");
        let de: RuntimeConfiguration = serde_json::from_str(&ser).expect("deserialize");
        assert_eq!(de.drain_timeout, time::Duration::from_millis(1250));
        assert_eq!(de.thread_name, "roundtrip");
    }
}

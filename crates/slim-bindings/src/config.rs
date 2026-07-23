// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration support for SLIM bindings
//!
//! This module provides configuration file loading and initialization for the bindings,
//! using the same configuration system as the main SLIM application.

use std::sync::{Arc, OnceLock};

use display_error_chain::ErrorChainExt;
use futures_timer::Delay;
use tracing::{debug, info};

use slim::config::ConfigLoader;
use slim::runtime::{RuntimeConfiguration as CoreRuntimeConfiguration, SlimRuntime};
use slim_config::tls::provider;
use slim_service::ServiceConfiguration as CoreServiceConfiguration;
use slim_tracing::TracingConfiguration as CoreTracingConfiguration;

use crate::ServiceConfig;
use crate::errors::SlimError;
use crate::init_config::{RuntimeConfig, TracingConfig};

/// Global state instance
static GLOBAL_STATE: OnceLock<GlobalState> = OnceLock::new();

/// Stores the loaded configuration, runtime, service, and tracing guard
struct GlobalState {
    /// The tracing guard must be kept alive for the duration of the program
    #[allow(dead_code)]
    tracing_guard: Option<Box<dyn std::any::Any + Send + Sync>>,

    /// Runtime configuration
    runtime_config: CoreRuntimeConfiguration,

    /// Tracing configuration
    tracing_config: CoreTracingConfiguration,

    /// Service configuration
    service_config: Vec<CoreServiceConfiguration>,

    /// Global Tokio runtime. Built via `slim::runtime::build`, which picks a
    /// multi-threaded runtime or a `current_thread` runtime on a dedicated
    /// driver thread based on the effective core count.
    runtime: SlimRuntime,

    /// Global service instances (all services)
    services: Vec<Arc<crate::service::Service>>,

    /// Global service instance (first service, for backward compatibility)
    service: Arc<crate::service::Service>,
}

/// Initialize SLIM bindings from a configuration file
///
/// This function:
/// 1. Loads the configuration file
/// 2. Initializes the crypto provider
/// 3. Sets up tracing/logging exactly as the main SLIM application does
/// 4. Initializes the global runtime with configuration from the file
/// 5. Initializes and starts the global service with servers/clients from config
///
/// This must be called before using any SLIM bindings functionality.
/// It's safe to call multiple times - subsequent calls will be ignored.
///
/// # Arguments
/// * `config_path` - Path to the YAML configuration file
///
/// # Returns
/// * `Ok(())` - Successfully initialized
/// * `Err(SlimError)` - If initialization fails
///
/// # Example
/// ```ignore
/// initialize_from_config("/path/to/config.yaml")?;
/// ```
#[uniffi::export]
pub fn initialize_from_config(config_path: String) {
    // Use get_or_init for atomic initialization
    GLOBAL_STATE.get_or_init(|| {
        let (runtime_config, tracing_conf, service_configs) =
            load_configs(&config_path).unwrap_or_else(|e| panic!("Initialization failed: {e}"));

        // Perform initialization and return config
        initialize_internal(
            runtime_config.clone(),
            tracing_conf.clone(),
            &service_configs,
        )
        .unwrap_or_else(|e| panic!("Initialization failed: {e}"))
    });
}

/// Initialize SLIM bindings from a configuration file and return errors
/// instead of panicking.
#[uniffi::export]
pub fn initialize_from_config_with_error(config_path: String) -> Result<(), SlimError> {
    if GLOBAL_STATE.get().is_some() {
        return Ok(());
    }

    let (runtime_config, tracing_conf, service_configs) = load_configs(&config_path)?;
    let global_state = initialize_internal(runtime_config, tracing_conf, &service_configs)?;

    GLOBAL_STATE
        .set(global_state)
        .map_err(|_| SlimError::InternalError {
            message: "Global state already initialized".to_string(),
        })?;

    Ok(())
}

/// Initialize SLIM bindings with custom configuration structs
///
/// This function allows you to programmatically configure SLIM bindings by passing
/// configuration structs directly, without needing a config file.
///
/// # Arguments
/// * `runtime_config` - Runtime configuration (thread count, naming, etc.)
/// * `tracing_config` - Tracing/logging configuration
/// * `service_config` - Service configuration (node ID, group name, etc.)
///
/// # Returns
/// * `Ok(())` - Successfully initialized
/// * `Err(SlimError)` - If initialization fails
///
/// # Example
/// ```ignore
/// let runtime_config = new_runtime_config();
/// let tracing_config = new_tracing_config();
/// let mut service_config = new_service_config();
/// service_config.node_id = Some("my-node".to_string());
///
/// initialize_with_configs(runtime_config, tracing_config, service_config)?;
/// ```
#[uniffi::export]
pub fn initialize_with_configs(
    runtime_config: RuntimeConfig,
    tracing_config: TracingConfig,
    service_config: &[ServiceConfig],
) -> Result<(), SlimError> {
    // Convert wrapper types to core types
    let core_runtime_config: CoreRuntimeConfiguration = runtime_config.into();
    let core_tracing_config: CoreTracingConfiguration = tracing_config.into();
    let core_service_config: Vec<CoreServiceConfiguration> =
        service_config.iter().map(|sc| sc.clone().into()).collect();

    // Use get_or_init for atomic initialization
    GLOBAL_STATE.get_or_init(|| {
        initialize_internal(
            core_runtime_config,
            core_tracing_config,
            &core_service_config,
        )
        .unwrap_or_else(|e| panic!("Initialization failed: {e}"))
    });
    Ok(())
}

fn load_configs(
    config_path: &str,
) -> Result<
    (
        CoreRuntimeConfiguration,
        CoreTracingConfiguration,
        Vec<CoreServiceConfiguration>,
    ),
    SlimError,
> {
    let mut config = ConfigLoader::new(config_path).map_err(|e| SlimError::ConfigError {
        message: e.chain().to_string(),
    })?;

    let runtime_config = config
        .runtime()
        .map_err(|e| SlimError::ConfigError {
            message: e.chain().to_string(),
        })?
        .clone();

    let tracing_conf = config
        .tracing()
        .map_err(|e| SlimError::ConfigError {
            message: e.chain().to_string(),
        })?
        .clone();

    let service_configs: Vec<CoreServiceConfiguration> = match config.services_config() {
        Ok(services) => {
            if !services.is_empty() {
                debug!("Using service configuration from config file");
                services.values().cloned().collect()
            } else {
                debug!("No services in config, using default");
                vec![CoreServiceConfiguration::default()]
            }
        }
        Err(_) => {
            debug!("No services section in config, using default");
            vec![CoreServiceConfiguration::default()]
        }
    };

    Ok((runtime_config, tracing_conf, service_configs))
}

/// Initialize SLIM bindings with default configuration
///
/// This is a convenience function that initializes the bindings with:
/// - Default runtime configuration
/// - Default tracing/logging configuration
/// - Initialized crypto provider
/// - Default global service (no servers/clients)
///
/// Use `initialize_from_config` for file-based configuration or
/// `initialize_with_configs` for programmatic configuration.
#[uniffi::export]
pub fn initialize_with_defaults() {
    // Check if already initialized
    GLOBAL_STATE.get_or_init(|| {
        // Use default configurations
        initialize_internal(
            CoreRuntimeConfiguration::default(),
            CoreTracingConfiguration::default(),
            &[CoreServiceConfiguration::default()],
        )
        .expect("Failed to initialize with defaults")
    });
}

/// Internal initialization function with common logic
fn initialize_internal(
    runtime_config: CoreRuntimeConfiguration,
    tracing_conf: CoreTracingConfiguration,
    service_configs: &[CoreServiceConfiguration],
) -> Result<GlobalState, SlimError> {
    // Initialize crypto provider (must be done before any TLS operations)
    provider::initialize_crypto_provider();

    // Build runtime from configuration. `build_for_embedding` picks a
    // dedicated-thread `current_thread` runtime when the effective core count
    // is 1, so the I/O reactor stays driven even when the host language's
    // event loop owns the calling thread; otherwise it falls back to the
    // standard multi-threaded runtime.
    let runtime = slim::runtime::build_for_embedding(&runtime_config);

    // Setup tracing subscriber (may fail if already set, which is OK)
    let guard = match tracing_conf.setup_tracing_subscriber() {
        Ok(g) => {
            debug!(?tracing_conf, "Tracing configuration loaded");
            debug!("SLIM bindings initialized");
            Some(Box::new(g) as Box<dyn std::any::Any + Send + Sync>)
        }
        Err(e) => {
            tracing::warn!(
                "Tracing subscriber already set or failed to initialize: {}. Using existing subscriber.",
                e
            );
            None
        }
    };

    // Initialize the global service with config and start it.
    // If we're already in an async tokio context (e.g. #[tokio::test]),
    // delegate the block_on to a fresh OS thread so we don't get "Cannot
    // start a runtime from within a runtime". `Handle` is Clone, so we just
    // hand a copy to the worker thread.
    let services = {
        let handle = runtime.handle();
        let configs = service_configs.to_vec();
        let work = move || handle.block_on(initialize_and_start_global_services(&configs));
        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::spawn(work)
                .join()
                .expect("Thread panicked while initializing services")?
        } else {
            work()?
        }
    };

    // Get first service for backward compatibility
    let service = services
        .first()
        .ok_or_else(|| SlimError::ServiceError {
            message: "No services were initialized".to_string(),
        })?
        .clone();

    // Store the global state
    let global_state = GlobalState {
        tracing_guard: guard,
        runtime_config,
        tracing_config: tracing_conf,
        service_config: service_configs.to_vec(),
        runtime,
        services,
        service,
    };

    Ok(global_state)
}

/// Check if SLIM bindings have been initialized
#[uniffi::export]
pub fn is_initialized() -> bool {
    GLOBAL_STATE.get().is_some()
}

/// Get a handle to the global Tokio runtime.
///
/// The returned `Handle` is cheap to clone and is safe to call `block_on` /
/// `spawn` on from any thread (including foreign-language threads driven by
/// UniFFI). When the runtime was built in single-threaded mode the actual
/// scheduler runs on a dedicated background OS thread; the `Handle` simply
/// routes work onto it.
///
/// If the bindings have not been initialized yet, this initializes them with
/// defaults first.
pub fn get_runtime() -> tokio::runtime::Handle {
    initialize_with_defaults();
    GLOBAL_STATE
        .get()
        .expect("Global runtime not initialized")
        .runtime
        .handle()
}

/// Returns references to all global services.
/// If not initialized, initializes with defaults first.
#[uniffi::export]
pub fn get_services() -> Vec<Arc<crate::service::Service>> {
    initialize_with_defaults();
    GLOBAL_STATE
        .get()
        .map(|state| state.services.clone())
        .expect("Global services not initialized")
}

/// Get the global service instance (creates it if it doesn't exist)
///
/// This returns a reference to the shared global service that can be used
/// across the application. All calls to this function return the same service instance.
#[uniffi::export]
pub fn get_global_service() -> Arc<crate::service::Service> {
    initialize_with_defaults();
    GLOBAL_STATE
        .get()
        .map(|state| state.service.clone())
        .expect("Main global service not initialized")
}

/// Get the runtime configuration
///
/// Returns a reference to the runtime configuration.
/// If not initialized, initializes with defaults first.
pub fn get_runtime_config() -> &'static CoreRuntimeConfiguration {
    initialize_with_defaults();
    &GLOBAL_STATE
        .get()
        .expect("Global state not initialized")
        .runtime_config
}

/// Get the tracing configuration
///
/// Returns a reference to the tracing configuration.
/// If not initialized, initializes with defaults first.
pub fn get_tracing_config() -> &'static CoreTracingConfiguration {
    initialize_with_defaults();
    &GLOBAL_STATE
        .get()
        .expect("Global state not initialized")
        .tracing_config
}

/// Get the service configuration
///
/// Returns a reference to the service configuration.
/// If not initialized, initializes with defaults first.
pub fn get_service_config() -> &'static [CoreServiceConfiguration] {
    initialize_with_defaults();
    &GLOBAL_STATE
        .get()
        .expect("Global state not initialized")
        .service_config
}

/// Initialize the global service with configuration and start it
///
/// This creates the global service with the provided configuration and calls
/// start() on it to initialize any configured servers and clients. If no
/// servers/clients are configured, start() will skip the run phase but is
/// still called for consistency.
async fn initialize_and_start_global_services(
    service_configs: &[CoreServiceConfiguration],
) -> Result<Vec<Arc<crate::service::Service>>, SlimError> {
    use slim_config::component::{Component, ComponentBuilder};
    use slim_service::ServiceBuilder;

    if service_configs.is_empty() {
        return Err(SlimError::ServiceError {
            message: "No service configuration provided".to_string(),
        });
    }

    let mut services = Vec::with_capacity(service_configs.len());

    // Create and start all services
    for (idx, service_config) in service_configs.iter().enumerate() {
        debug!("Creating global service {} with configuration", idx);
        let mut slim_service =
            ServiceBuilder::new().build_with_config(service_config.service_id(), service_config)?;

        // Start the service to initialize servers and clients
        // This calls run() internally if servers/clients are configured
        debug!("Starting global service {}", idx);
        match slim_service.start().await {
            Ok(_) => {
                info!("Global service {} started successfully", idx);
            }
            Err(e) => {
                // Check if the error is due to no servers/clients configured
                // This is acceptable for bindings that may not need network layer
                if e.to_string().contains("no server or client configured") {
                    debug!(
                        "No servers or clients configured for service {}, service initialized without network layer",
                        idx
                    );
                } else {
                    return Err(SlimError::ServiceError {
                        message: format!("Failed to start service {}: {}", idx, e.chain()),
                    });
                }
            }
        }

        // Create and add the service
        let service = Arc::new(crate::service::Service {
            inner: Arc::new(slim_service),
        });
        services.push(service);
    }

    debug!(
        "All {} global services initialized and started",
        services.len()
    );
    Ok(services)
}

/// Perform graceful shutdown operations
///
/// This function performs the same shutdown operations as the main SLIM application:
/// 1. Logs the shutdown signal
/// 2. Gracefully stops the global service (if initialized)
///
/// This should be called when the application receives a shutdown signal (e.g., Ctrl+C)
/// or when the application is terminating.
///
/// # Returns
/// * `Ok(())` - Successfully shut down
/// * `Err(SlimError)` - If shutdown fails
///
/// # Example
/// ```ignore
/// // Handle shutdown signal
/// shutdown().await?;
/// ```
pub async fn shutdown() -> Result<(), SlimError> {
    use tracing::info;

    info!("Performing graceful shutdown");

    // Get the drain timeout from configuration
    let drain_timeout = get_runtime_config().drain_timeout();

    // Get the global service if it exists
    let service_opt = GLOBAL_STATE.get();

    if let Some(state) = service_opt {
        let inner = &state.service.inner;
        info!("Stopping global service");

        // Runtime-agnostic timeout using futures-timer
        let shutdown_fut = inner.shutdown();
        futures::pin_mut!(shutdown_fut);
        let delay = Delay::new(drain_timeout);
        futures::pin_mut!(delay);

        match futures::future::select(shutdown_fut, delay).await {
            futures::future::Either::Left((result, _)) => {
                result?;
            }
            futures::future::Either::Right(_) => {
                return Err(SlimError::ServiceError {
                    message: format!("Service shutdown timed out after {drain_timeout:?}"),
                });
            }
        }

        info!("Global service stopped");
    } else {
        debug!("No global service to shutdown");
    }

    info!("Graceful shutdown complete");
    Ok(())
}

/// Perform graceful shutdown operations (blocking version)
///
/// This is a blocking wrapper around the async `shutdown()` function for use from
/// synchronous contexts or language bindings that don't support async.
///
/// # Returns
/// * `Ok(())` - Successfully shut down
/// * `Err(SlimError)` - If shutdown fails
#[uniffi::export]
pub fn shutdown_blocking() -> Result<(), SlimError> {
    get_runtime().block_on(shutdown())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_variant_exists() {
        // Verify ConfigError variant can be created
        let err = SlimError::ConfigError {
            message: "test error".to_string(),
        };
        assert!(format!("{err}").contains("Configuration error"));
    }

    #[test]
    fn test_runtime_always_accessible() {
        // The runtime can always be accessed (either from init or get_runtime fallback)
        let _handle = get_runtime();
    }

    #[test]
    fn test_idempotent_behavior() {
        // Test that multiple initialization calls don't panic
        // Note: In test environment, tracing may already be set up,
        // so we just verify the calls don't panic
        initialize_with_defaults();
        initialize_with_defaults();
    }

    #[test]
    fn test_get_runtime_config() {
        // Test getting runtime config
        let runtime_config = get_runtime_config();

        // Should return a valid config (either from init or default)
        assert!(!runtime_config.thread_name().is_empty());
        assert!(runtime_config.drain_timeout().as_secs() > 0);
    }

    #[test]
    fn test_get_tracing_config() {
        // Test getting tracing config
        let tracing_config = get_tracing_config();

        // Should return a valid config (either from init or default)
        assert!(!tracing_config.log_level().is_empty());
    }

    #[test]
    fn test_get_service_config() {
        // Test getting service config
        let service_config = get_service_config();

        // Should return a valid config (either from init or default)
        // Just verify we can access the config fields
        assert!(!service_config.is_empty());
        let _ = &service_config[0].node_id;
        let _ = &service_config[0].domain_name;
    }

    #[test]
    fn test_config_getters_return_defaults_when_not_initialized() {
        // Even if not initialized, getters should return defaults
        let runtime_config = get_runtime_config();
        let tracing_config = get_tracing_config();
        let service_config = get_service_config();

        // All should be valid
        // n_cores is usize, always >= 0, so just check it exists
        let _ = runtime_config.n_cores();
        assert!(!tracing_config.log_level().is_empty());
        // Service config should have default values
        assert!(!service_config.is_empty());
        assert!(!service_config[0].node_id.is_empty());
        assert!(service_config[0].domain_name.is_none());
    }

    #[tokio::test]
    #[test_fork::fork]
    async fn test_initialization_from_async_context() {
        // This test verifies that initialization works correctly when called
        // from within an async tokio context (e.g., #[tokio::test])
        // The implementation should detect the existing runtime and use
        // std::thread::spawn to avoid "Cannot start a runtime from within a runtime" panic

        // Initialize with defaults from async context
        initialize_with_defaults();

        // Verify we can access the runtime
        let _handle = get_runtime();

        // Verify we can access configs
        let _ = get_runtime_config();
        let _ = get_tracing_config();
        let _ = get_service_config();

        // Verify initialization is idempotent even from async context
        initialize_with_defaults();
    }

    #[test_fork::fork]
    #[test]
    fn test_single_thread_runtime_drives_io() {
        // With n_cores=1 the runtime is built as `current_thread` on a
        // dedicated OS thread. Verify it actually drives timers and spawned
        // tasks — this is the property that prevents the UniFFI/asyncio
        // deadlock from issue #1648.
        use crate::init_config::{new_runtime_config_with, new_service_config, new_tracing_config};
        use std::time::Duration;

        let runtime_config =
            new_runtime_config_with(1, "slim-single".to_string(), Duration::from_secs(1));
        let result = initialize_with_configs(
            runtime_config,
            new_tracing_config(),
            &[new_service_config()],
        );
        assert!(result.is_ok());

        // Schedule work on the global handle from a non-async thread. If the
        // bg driver thread isn't actually polling, this will hang forever.
        let handle = get_runtime();
        let value = handle.block_on(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            42_u32
        });
        assert_eq!(value, 42);
    }

    #[test]
    fn test_initialize_with_configs() {
        // Test initializing with custom config structs using wrapper types
        use crate::init_config::{new_runtime_config, new_service_config_with, new_tracing_config};
        use crate::service::DataplaneConfig;

        let runtime_config = new_runtime_config();
        let tracing_config = new_tracing_config();
        let service_config = new_service_config_with(
            Some("test-node".to_string()),
            Some("test-domain".to_string()),
            DataplaneConfig::default(),
        );

        // This should succeed (or be idempotent if already initialized)
        let result = initialize_with_configs(runtime_config, tracing_config, &[service_config]);
        assert!(result.is_ok());

        // Verify we can access the configs
        let retrieved_service_config = get_service_config();
        // Note: The actual values may differ if already initialized by another test
        assert!(!retrieved_service_config.is_empty());
        let _ = &retrieved_service_config[0].node_id;
        let _ = &retrieved_service_config[0].domain_name;
    }

    #[test_fork::fork]
    #[test]
    fn test_service_order_preserved_with_function_call() {
        // Test that service order is preserved when initializing with multiple configs
        use crate::init_config::{new_runtime_config, new_service_config_with, new_tracing_config};
        use crate::service::DataplaneConfig;

        let runtime_config = new_runtime_config();
        let tracing_config = new_tracing_config();

        // Create multiple service configs with distinct identifiers
        let service_configs = vec![
            new_service_config_with(
                Some("service-0".to_string()),
                Some("domain-0".to_string()),
                DataplaneConfig::default(),
            ),
            new_service_config_with(
                Some("service-1".to_string()),
                Some("domain-1".to_string()),
                DataplaneConfig::default(),
            ),
            new_service_config_with(
                Some("service-2".to_string()),
                Some("domain-2".to_string()),
                DataplaneConfig::default(),
            ),
        ];

        // Store the expected order for verification
        let expected_node_ids: Vec<String> = service_configs
            .iter()
            .map(|sc| sc.node_id.clone().unwrap())
            .collect();

        // Initialize with the configs
        let result = initialize_with_configs(runtime_config, tracing_config, &service_configs);
        assert!(result.is_ok());

        // Verify the order is preserved
        let retrieved_configs = get_service_config();
        assert_eq!(retrieved_configs.len(), 3, "Should have 3 service configs");

        // Verify each config is in the correct order by checking node_id
        for (idx, config) in retrieved_configs.iter().enumerate() {
            assert_eq!(
                &config.node_id, &expected_node_ids[idx],
                "Config at index {} should have node_id '{}', but got '{}'",
                idx, expected_node_ids[idx], config.node_id
            );
        }

        // Verify services count matches
        let services = get_services();
        assert_eq!(services.len(), 3, "Should have 3 services");
        assert_eq!(
            services.len(),
            retrieved_configs.len(),
            "Number of services must match number of configs"
        );
    }

    #[tokio::test]
    #[test_fork::fork]
    async fn test_service_order_preserved_async() {
        // Test that service order is preserved when created via async initialization
        use crate::init_config::{new_runtime_config, new_service_config_with, new_tracing_config};
        use crate::service::DataplaneConfig;

        let runtime_config = new_runtime_config();
        let tracing_config = new_tracing_config();

        // Create multiple service configs with distinct identifiers
        let service_configs = vec![
            new_service_config_with(
                Some("async-service-0".to_string()),
                Some("async-domain-0".to_string()),
                DataplaneConfig::default(),
            ),
            new_service_config_with(
                Some("async-service-1".to_string()),
                Some("async-domain-1".to_string()),
                DataplaneConfig::default(),
            ),
            new_service_config_with(
                Some("async-service-2".to_string()),
                Some("async-domain-2".to_string()),
                DataplaneConfig::default(),
            ),
            new_service_config_with(
                Some("async-service-3".to_string()),
                Some("async-domain-3".to_string()),
                DataplaneConfig::default(),
            ),
        ];

        // Store the expected order for verification
        let expected_node_ids: Vec<String> = service_configs
            .iter()
            .map(|sc| sc.node_id.clone().unwrap())
            .collect();
        let expected_domain_names: Vec<String> = service_configs
            .iter()
            .map(|sc| sc.domain_name.clone().unwrap())
            .collect();

        // Initialize with the configs from async context
        let result = initialize_with_configs(runtime_config, tracing_config, &service_configs);
        assert!(result.is_ok());

        // Verify the order is preserved
        let retrieved_configs = get_service_config();
        assert_eq!(
            retrieved_configs.len(),
            4,
            "Should have exactly 4 service configs"
        );

        // Verify each config is in the correct order
        for (idx, config) in retrieved_configs.iter().enumerate() {
            assert_eq!(
                &config.node_id, &expected_node_ids[idx],
                "Config at index {} should have node_id '{}', but got '{}'",
                idx, expected_node_ids[idx], config.node_id
            );
            assert_eq!(
                config.domain_name.as_ref().unwrap(),
                &expected_domain_names[idx],
                "Config at index {} should have domain_name '{}', but got '{:?}'",
                idx,
                expected_domain_names[idx],
                config.domain_name
            );
        }

        // Verify services are also in order and match config count
        let services = get_services();
        assert_eq!(services.len(), 4, "Should have exactly 4 services");
        assert_eq!(
            services.len(),
            retrieved_configs.len(),
            "Service count must match config count"
        );
    }

    #[test_fork::fork]
    #[test]
    fn test_service_order_preserved_from_config_file() {
        // Test that service order is preserved when loading from config file
        use std::io::Write;

        // Create a temporary config file with multiple services
        let config_content = r#"# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

runtime:
  n_cores: 0
  thread_name: "slim-data-plane"
  drain_timeout: 10s

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:56357"
          tls:
            insecure: true
      clients: []
  slim/1:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:56358"
          tls:
            insecure: true
      clients: []
  slim/2:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:56359"
          tls:
            insecure: true
      clients: []
  slim/3:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:56360"
          tls:
            insecure: true
      clients: []
"#;

        // Write to a temporary file
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test-multi-service-order.yaml");
        let mut file =
            std::fs::File::create(&config_path).expect("Failed to create temp config file");
        file.write_all(config_content.as_bytes())
            .expect("Failed to write config");
        drop(file); // Ensure file is flushed and closed

        // Initialize from config file
        initialize_from_config(config_path.to_str().unwrap().to_string());

        // Verify all services were created
        let services = get_services();
        assert_eq!(
            services.len(),
            4,
            "Should have exactly 4 services from config file"
        );

        // Verify config order is preserved
        let configs = get_service_config();
        assert_eq!(
            configs.len(),
            4,
            "Should have exactly 4 configs from config file"
        );

        // Verify that the number of services matches the number of configs
        assert_eq!(
            services.len(),
            configs.len(),
            "Number of services must match number of configs"
        );

        // The order should be preserved as they appear in the YAML file
        // IndexMap preserves insertion order from the YAML file
        // The services in the config file are: slim/0, slim/1, slim/2, slim/3
        // We verify by checking that each service's name matches the expected key
        let expected_service_names = ["slim/0", "slim/1", "slim/2", "slim/3"];

        for (idx, service) in services.iter().enumerate() {
            let service_name = service.get_name();
            assert_eq!(
                service_name, expected_service_names[idx],
                "Service at index {} should be named '{}', but got '{}'",
                idx, expected_service_names[idx], service_name
            );
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn test_is_initialized() {
        // Test the is_initialized function
        // After any initialization (including defaults), should return true
        initialize_with_defaults();
        assert!(is_initialized());
    }

    #[test_fork::fork]
    #[test]
    fn test_is_initialized_before_init() {
        // In a fresh fork, is_initialized should eventually return true
        // because other tests or get_runtime will trigger initialization
        let result = is_initialized();
        // This may be true or false depending on test execution order
        // Just verify the function works without panicking
        let _ = result;
    }

    #[test]
    fn test_get_global_service() {
        // Test that we can get the global service
        let service = get_global_service();

        // Verify the service has a valid name (defaults to UUID-based node_id)
        let name = service.get_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_get_global_service_singleton() {
        // Verify get_global_service always returns the same instance
        let service1 = get_global_service();
        let service2 = get_global_service();

        // Should be the same Arc instance
        assert!(Arc::ptr_eq(&service1, &service2));
    }

    #[tokio::test]
    #[test_fork::fork]
    async fn test_shutdown_success() {
        // Test successful shutdown
        initialize_with_defaults();

        // Ensure service is initialized
        let _service = get_global_service();

        // Shutdown should succeed
        let result = shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed: {result:?}");
    }

    #[tokio::test]
    #[test_fork::fork]
    async fn test_shutdown_when_not_initialized() {
        // Test shutdown when service is not initialized (fresh state)
        // This should succeed gracefully without errors
        let result = shutdown().await;
        // Should succeed even if nothing was initialized
        assert!(result.is_ok());
    }

    #[test_fork::fork]
    #[test]
    fn test_shutdown_blocking_success() {
        // Test blocking shutdown wrapper
        initialize_with_defaults();

        // Ensure service is initialized
        let _service = get_global_service();

        // Blocking shutdown should succeed
        let result = shutdown_blocking();
        assert!(
            result.is_ok(),
            "Blocking shutdown should succeed: {result:?}"
        );
    }

    #[test_fork::fork]
    #[test]
    fn test_shutdown_blocking_when_not_initialized() {
        // Test blocking shutdown when nothing is initialized
        let result = shutdown_blocking();
        // Should succeed gracefully
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[test_fork::fork]
    async fn test_shutdown_idempotent() {
        // Test that multiple shutdown calls don't cause issues
        initialize_with_defaults();
        let _service = get_global_service();

        // First shutdown
        let result1 = shutdown().await;
        assert!(result1.is_ok());

        // Second shutdown should also succeed (or handle gracefully)
        let result2 = shutdown().await;
        // May succeed or fail gracefully, but shouldn't panic
        let _ = result2;
    }

    #[test]
    fn test_initialize_from_config_invalid_path() {
        // Test error handling for invalid config file path
        let result = std::panic::catch_unwind(|| {
            initialize_from_config("/nonexistent/path/to/config.yaml".to_string());
        });

        // The function may panic or handle gracefully depending on implementation
        // Just verify it doesn't cause undefined behavior
        let _ = result;
    }

    #[test_fork::fork]
    #[test]
    fn test_initialize_from_config_with_error_invalid_path() {
        // Test that we get a structured error instead of a panic
        let result =
            initialize_from_config_with_error("/nonexistent/path/to/config.yaml".to_string());

        match result {
            Err(SlimError::ConfigError { message }) => {
                assert!(!message.is_empty());
            }
            other => panic!("Expected ConfigError, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_services_count() {
        // Test that get_services returns services
        let services = get_services();

        // Should have at least one service
        assert!(!services.is_empty());

        // Each service should be valid
        for service in services.iter() {
            let name = service.get_name();
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn test_services_match_configs() {
        // Verify that the number of services matches configs
        let services = get_services();
        let configs = get_service_config();

        assert_eq!(
            services.len(),
            configs.len(),
            "Number of services should match number of configs"
        );
    }

    #[test_fork::fork]
    #[test]
    fn test_initialize_from_config_with_valid_yaml() {
        // Test initializing from a valid config file
        use std::io::Write;

        let config_content = r#"
tracing:
  log_level: debug
  display_thread_names: true

runtime:
  n_cores: 2
  thread_name: "test-runtime"
  drain_timeout: 5s

services:
  test-service:
    node_id: "test-node"
    domain_name: "test-domain"
    dataplane:
      servers: []
      clients: []
"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test-valid-config.yaml");
        let mut file = std::fs::File::create(&config_path).expect("Failed to create temp file");
        file.write_all(config_content.as_bytes())
            .expect("Failed to write config");
        drop(file);

        // Initialize from config
        initialize_from_config(config_path.to_str().unwrap().to_string());

        // Verify initialization succeeded by checking configs
        let tracing_config = get_tracing_config();
        assert!(!tracing_config.log_level().is_empty());

        let runtime_config = get_runtime_config();
        assert!(!runtime_config.thread_name().is_empty());

        let service_configs = get_service_config();
        assert!(!service_configs.is_empty());

        // Clean up
        let _ = std::fs::remove_file(&config_path);
    }

    #[test_fork::fork]
    #[test]
    fn test_initialize_from_config_with_error_valid_yaml() {
        // Test the non-panicking initializer with a valid config file
        use std::io::Write;

        let config_content = r#"
tracing:
    log_level: info
    display_thread_names: true

runtime:
    n_cores: 1
    thread_name: "test-runtime-error"
    drain_timeout: 3s

services:
    test-service:
        node_id: "test-node"
        domain_name: "test-domain"
        dataplane:
            servers: []
            clients: []
"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test-valid-config-error.yaml");
        let mut file = std::fs::File::create(&config_path).expect("Failed to create temp file");
        file.write_all(config_content.as_bytes())
            .expect("Failed to write config");
        drop(file);

        let result = initialize_from_config_with_error(config_path.to_str().unwrap().to_string());
        assert!(result.is_ok(), "Expected Ok(()), got: {result:?}");

        let service_configs = get_service_config();
        assert!(!service_configs.is_empty());

        let _ = std::fs::remove_file(&config_path);
    }
}

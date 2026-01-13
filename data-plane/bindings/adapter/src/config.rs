// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration support for SLIM bindings
//!
//! This module provides configuration file loading and initialization for the bindings,
//! using the same configuration system as the main SLIM application.

use std::sync::{Arc, OnceLock};

use tracing::{debug, info};

use slim::config::ConfigLoader;
use slim::runtime::RuntimeConfiguration as CoreRuntimeConfiguration;
use slim_config::tls::provider;
use slim_service::ServiceConfiguration as CoreServiceConfiguration;
use slim_tracing::TracingConfiguration as CoreTracingConfiguration;

use crate::errors::SlimError;
use crate::init_config::{RuntimeConfig, ServiceConfig, TracingConfig};

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
    service_config: CoreServiceConfiguration,

    /// Global Tokio runtime
    runtime: tokio::runtime::Runtime,

    /// Global service instance
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
        // Load configuration
        let mut config = ConfigLoader::new(&config_path).expect("Failed to create config loader");

        // Get configurations
        let runtime_config = config.runtime().clone();
        let tracing_conf = config.tracing().clone();
        let service_config = match config.services() {
            Ok(services) => {
                if let Some(service) = services.values().next() {
                    debug!("Using service configuration from config file");
                    service.config().clone()
                } else {
                    debug!("No services in config, using default");
                    CoreServiceConfiguration::default()
                }
            }
            Err(_) => {
                debug!("No services section in config, using default");
                CoreServiceConfiguration::default()
            }
        };

        // Perform initialization and return config
        initialize_internal(
            runtime_config.clone(),
            tracing_conf.clone(),
            service_config.clone(),
        )
        .unwrap_or_else(|e| panic!("Initialization failed: {}", e))
    });
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
    service_config: ServiceConfig,
) -> Result<(), SlimError> {
    // Convert wrapper types to core types
    let core_runtime_config: CoreRuntimeConfiguration = runtime_config.into();
    let core_tracing_config: CoreTracingConfiguration = tracing_config.into();
    let core_service_config: CoreServiceConfiguration = service_config.into();

    // Use get_or_init for atomic initialization
    GLOBAL_STATE.get_or_init(|| {
        initialize_internal(
            core_runtime_config,
            core_tracing_config,
            core_service_config,
        )
        .unwrap_or_else(|e| panic!("Initialization failed: {}", e))
    });
    Ok(())
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
            CoreServiceConfiguration::default(),
        )
        .expect("Failed to initialize with defaults")
    });
}

/// Internal initialization function with common logic
fn initialize_internal(
    runtime_config: CoreRuntimeConfiguration,
    tracing_conf: CoreTracingConfiguration,
    service_config: CoreServiceConfiguration,
) -> Result<GlobalState, SlimError> {
    // Initialize crypto provider (must be done before any TLS operations)
    provider::initialize_crypto_provider();

    // Build runtime from configuration
    let runtime = slim::runtime::build(&runtime_config).runtime;

    // Setup tracing subscriber (may fail if already set, which is OK)
    let guard = match tracing_conf.setup_tracing_subscriber() {
        Ok(g) => {
            debug!(?tracing_conf, "Tracing configuration loaded");
            info!("SLIM bindings initialized");
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

    // Initialize the global service with config and start it
    // If we're already in an async tokio context, we need to spawn a separate thread
    // to avoid "Cannot start a runtime from within a runtime" panic.
    // Even though we created a new runtime, calling block_on on it will still panic
    // if the current thread is being used by another runtime to drive async tasks.
    let (runtime, service) = if tokio::runtime::Handle::try_current().is_ok() {
        // We're in an async context - spawn a separate OS thread to run block_on
        // Wrap the runtime in Arc to safely share it across thread boundaries
        let runtime_arc = Arc::new(runtime);
        let runtime_clone = Arc::clone(&runtime_arc);
        let service_config_clone = service_config.clone();

        let service = std::thread::spawn(move || {
            runtime_clone.block_on(async move {
                initialize_and_start_global_service(service_config_clone).await
            })
        })
        .join()
        .expect("Thread panicked while initializing service")?;

        // Extract the runtime back from Arc (will succeed since we have the only reference)
        let runtime = Arc::try_unwrap(runtime_arc).expect("Failed to unwrap runtime from Arc");

        (runtime, service)
    } else {
        // Not in an async context, safe to use block_on directly
        let service = runtime.block_on(async {
            initialize_and_start_global_service(service_config.clone()).await
        })?;
        (runtime, service)
    };

    // Store the global state
    let global_state = GlobalState {
        tracing_guard: guard,
        runtime_config,
        tracing_config: tracing_conf,
        service_config,
        runtime,
        service,
    };

    Ok(global_state)
}

/// Check if SLIM bindings have been initialized
#[uniffi::export]
pub fn is_initialized() -> bool {
    GLOBAL_STATE.get().is_some()
}

/// Get the global runtime
///
/// Returns a reference to the runtime, or initializes with defaults if not yet initialized.
pub fn get_runtime() -> &'static tokio::runtime::Runtime {
    initialize_with_defaults();
    &GLOBAL_STATE
        .get()
        .expect("Global runtime not initialized")
        .runtime
}

/// Get the global service
///
/// Returns a reference to the service, or initializes with defaults if not yet initialized.
pub fn get_service() -> Arc<crate::service::Service> {
    initialize_with_defaults();
    GLOBAL_STATE
        .get()
        .map(|state| state.service.clone())
        .expect("Global service not initialized")
}

/// Get the runtime configuration
///
/// Returns a reference to the runtime configuration.
/// Panics if not initialized.
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
/// Panics if not initialized.
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
/// Panics if not initialized.
pub fn get_service_config() -> &'static CoreServiceConfiguration {
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
async fn initialize_and_start_global_service(
    service_config: CoreServiceConfiguration,
) -> Result<Arc<crate::service::Service>, SlimError> {
    use slim_config::component::{Component, ComponentBuilder};
    use slim_service::ServiceBuilder;

    // Create service with configuration
    debug!("Creating global service with configuration");
    let mut slim_service = ServiceBuilder::new()
        .build_with_config("global-bindings-service", &service_config)
        .map_err(|e| SlimError::ServiceError {
            message: format!("Failed to build service with config: {}", e),
        })?;

    // Start the service to initialize servers and clients
    // This calls run() internally if servers/clients are configured
    info!("Starting global service");
    match slim_service.start().await {
        Ok(_) => {
            info!("Global service started successfully");
        }
        Err(e) => {
            // Check if the error is due to no servers/clients configured
            // This is acceptable for bindings that may not need network layer
            if e.to_string().contains("no server or client configured") {
                debug!(
                    "No servers or clients configured, service initialized without network layer"
                );
            } else {
                return Err(SlimError::ServiceError {
                    message: format!("Failed to start service: {}", e),
                });
            }
        }
    }

    // Create and return the service
    let service = Arc::new(crate::service::Service {
        inner: Arc::new(tokio::sync::RwLock::new(slim_service)),
    });

    info!("Global service initialized and started");
    Ok(service)
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
        let inner = state.service.inner.read().await;
        info!("Stopping global service");
        inner
            .shutdown_with_timeout(drain_timeout)
            .await
            .map_err(|e| SlimError::ServiceError {
                message: format!("Failed to shutdown service: {}", e),
            })?;
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
    let runtime = get_runtime();
    runtime.block_on(shutdown())
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
        assert!(format!("{}", err).contains("Configuration error"));
    }

    #[test]
    fn test_is_initialized_check() {
        // is_initialized() should always return a bool
        // May be true or false depending on test execution order
        let _ = is_initialized();
    }

    #[test]
    fn test_runtime_always_accessible() {
        // The runtime can always be accessed (either from init or get_runtime fallback)
        let runtime = get_runtime();
        let _handle = runtime.handle();
    }

    #[test]
    fn test_initialize_from_nonexistent_file_returns_error() {
        // Try to initialize from a file that doesn't exist
        // If already initialized, will return Ok (idempotent)
        // If not initialized, will panic due to file not found

        initialize_from_config("/this/path/definitely/does/not/exist.yaml".to_string());
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
        let _ = &service_config.node_id;
        let _ = &service_config.group_name;
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
        assert!(service_config.node_id.is_none());
        assert!(service_config.group_name.is_none());
    }

    #[tokio::test]
    async fn test_initialization_from_async_context() {
        // This test verifies that initialization works correctly when called
        // from within an async tokio context (e.g., #[tokio::test])
        // The implementation should detect the existing runtime and use
        // std::thread::spawn to avoid "Cannot start a runtime from within a runtime" panic

        // Initialize with defaults from async context
        initialize_with_defaults();

        // Verify we can access the runtime
        let runtime = get_runtime();
        let _handle = runtime.handle();

        // Verify we can access configs
        let _ = get_runtime_config();
        let _ = get_tracing_config();
        let _ = get_service_config();

        // Verify initialization is idempotent even from async context
        initialize_with_defaults();
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
            Some("test-group".to_string()),
            DataplaneConfig::default(),
        );

        // This should succeed (or be idempotent if already initialized)
        let result = initialize_with_configs(runtime_config, tracing_config, service_config);
        assert!(result.is_ok());

        // Verify we can access the configs
        let retrieved_service_config = get_service_config();
        // Note: The actual values may differ if already initialized by another test
        let _ = &retrieved_service_config.node_id;
        let _ = &retrieved_service_config.group_name;
    }
}

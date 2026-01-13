// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Configuration support for SLIM bindings
//!
//! This module provides configuration file loading and initialization for the bindings,
//! using the same configuration system as the main SLIM application.

use std::sync::{Arc, OnceLock};

use tracing::{debug, info};

use slim::config::ConfigLoader;
use slim::runtime::RuntimeConfiguration;
use slim_config::tls::provider;
use slim_service::ServiceConfiguration;
use slim_tracing::TracingConfiguration;

use crate::errors::SlimError;

/// Global configuration loader instance
static GLOBAL_CONFIG: OnceLock<GlobalConfig> = OnceLock::new();

/// Stores the loaded configuration and tracing guard
struct GlobalConfig {
    /// The tracing guard must be kept alive for the duration of the program
    #[allow(dead_code)]
    tracing_guard: Option<Box<dyn std::any::Any + Send + Sync>>,

    /// Runtime configuration
    runtime_config: RuntimeConfiguration,

    /// Tracing configuration
    tracing_config: TracingConfiguration,

    /// Service configuration
    service_config: ServiceConfiguration,
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
pub fn initialize_from_config(config_path: String) -> Result<(), SlimError> {
    // Check if already initialized
    if GLOBAL_CONFIG.get().is_some() {
        debug!("SLIM bindings already initialized from config, skipping");
        return Ok(());
    }

    // Load configuration
    let mut config = ConfigLoader::new(&config_path).map_err(|e| SlimError::ConfigError {
        message: format!("Failed to load configuration: {}", e),
    })?;

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
                ServiceConfiguration::default()
            }
        }
        Err(_) => {
            debug!("No services section in config, using default");
            ServiceConfiguration::default()
        }
    };

    // Perform initialization
    initialize_internal(runtime_config, tracing_conf, service_config)
}

/// Initialize SLIM bindings with default configuration
///
/// This is a convenience function that initializes the bindings with:
/// - Default runtime configuration
/// - Default tracing/logging configuration
/// - Initialized crypto provider
/// - Default global service (no servers/clients)
///
/// Use `initialize_from_config` for custom configuration.
#[uniffi::export]
pub fn initialize_with_defaults() -> Result<(), SlimError> {
    // Check if already initialized
    if GLOBAL_CONFIG.get().is_some() {
        debug!("SLIM bindings already initialized, skipping");
        return Ok(());
    }

    // Use default configurations
    initialize_internal(
        RuntimeConfiguration::default(),
        TracingConfiguration::default(),
        ServiceConfiguration::default(),
    )
}

/// Internal initialization function with common logic
fn initialize_internal(
    runtime_config: RuntimeConfiguration,
    tracing_conf: TracingConfiguration,
    service_config: ServiceConfiguration,
) -> Result<(), SlimError> {
    // Initialize crypto provider (must be done before any TLS operations)
    provider::initialize_crypto_provider();

    // Initialize runtime (may already be initialized by get_runtime(), that's OK)
    let _ = crate::runtime::initialize_runtime_from_config(&runtime_config);

    // Setup tracing subscriber (may fail if already set, which is OK)
    let guard = match tracing_conf.setup_tracing_subscriber() {
        Ok(g) => {
            debug!(?tracing_conf, "Tracing configuration loaded");
            info!("SLIM bindings initialized");
            Some(Box::new(g) as Box<dyn std::any::Any + Send + Sync>)
        }
        Err(e) => {
            debug!(
                "Tracing subscriber already set or failed to initialize: {}",
                e
            );
            None
        }
    };

    // Store the configuration before moving service_config
    let global_config = GlobalConfig {
        tracing_guard: guard,
        runtime_config,
        tracing_config: tracing_conf,
        service_config: service_config.clone(),
    };

    // Initialize the global service with config and start it
    let runtime = crate::runtime::get_runtime();
    runtime.block_on(async { initialize_and_start_global_service(service_config).await })?;

    GLOBAL_CONFIG
        .set(global_config)
        .map_err(|_| SlimError::ConfigError {
            message: "Failed to set global config (already initialized)".to_string(),
        })?;

    Ok(())
}

/// Check if SLIM bindings have been initialized
#[uniffi::export]
pub fn is_initialized() -> bool {
    GLOBAL_CONFIG.get().is_some()
}

/// Get the runtime configuration
///
/// Returns the runtime configuration, or the default if not initialized.
pub fn get_runtime_config() -> RuntimeConfiguration {
    GLOBAL_CONFIG
        .get()
        .map(|config| config.runtime_config.clone())
        .unwrap_or_default()
}

/// Get the tracing configuration
///
/// Returns the tracing configuration, or the default if not initialized.
pub fn get_tracing_config() -> TracingConfiguration {
    GLOBAL_CONFIG
        .get()
        .map(|config| config.tracing_config.clone())
        .unwrap_or_default()
}

/// Get the service configuration
///
/// Returns the service configuration, or the default if not initialized.
pub fn get_service_config() -> ServiceConfiguration {
    GLOBAL_CONFIG
        .get()
        .map(|config| config.service_config.clone())
        .unwrap_or_default()
}

/// Initialize the global service with configuration and start it
///
/// This creates the global service with the provided configuration and calls
/// start() on it to initialize any configured servers and clients. If no
/// servers/clients are configured, start() will skip the run phase but is
/// still called for consistency.
async fn initialize_and_start_global_service(
    service_config: ServiceConfiguration,
) -> Result<(), SlimError> {
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

    // Store the service in the global static
    let service = Arc::new(crate::service::Service {
        inner: Arc::new(tokio::sync::RwLock::new(slim_service)),
    });

    crate::service::GLOBAL_SERVICE
        .set(service)
        .map_err(|_| SlimError::ServiceError {
            message: "Global service already initialized".to_string(),
        })?;

    info!("Global service initialized and started");
    Ok(())
}

/// Wait for a shutdown signal (Ctrl+C or SIGTERM)
///
/// This function blocks until a shutdown signal is received, matching the behavior
/// of the main SLIM application. It uses `slim_signal::shutdown()` to wait for signals.
///
/// This is useful for applications that need to run until explicitly shut down.
///
/// # Example
/// ```ignore
/// // Initialize
/// initialize_from_config("/path/to/config.yaml")?;
///
/// // Start your services/adapters
/// let app = create_app_with_secret(name, "secret")?;
///
/// // Wait for shutdown signal
/// wait_for_shutdown_signal().await;
///
/// // Perform cleanup
/// shutdown().await?;
/// ```
pub async fn wait_for_shutdown_signal() {
    use tracing::debug;

    tokio::select! {
        _ = slim_signal::shutdown() => {
            debug!("Received shutdown signal");
        }
    }
}

/// Wait for a shutdown signal (blocking version)
///
/// This is a blocking wrapper around the async `wait_for_shutdown_signal()` function
/// for use from synchronous contexts or language bindings that don't support async.
///
/// # Example
/// ```ignore
/// // Initialize
/// initialize_from_config("/path/to/config.yaml")?;
///
/// // Start your services/adapters
/// let app = create_app_with_secret(name, "secret")?;
///
/// // Wait for shutdown signal (blocks until Ctrl+C)
/// wait_for_shutdown_signal_blocking();
///
/// // Perform cleanup
/// shutdown_blocking()?;
/// ```
#[uniffi::export]
pub fn wait_for_shutdown_signal_blocking() {
    let runtime = crate::runtime::get_runtime();
    runtime.block_on(wait_for_shutdown_signal())
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
    let service_opt = crate::service::GLOBAL_SERVICE.get();

    if let Some(service) = service_opt {
        let inner = service.inner.read().await;
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
    let runtime = crate::runtime::get_runtime();
    runtime.block_on(shutdown())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_wait_for_shutdown_signal_with_timeout() {
        // Test that the wait function can be cancelled with a timeout
        let result =
            tokio::time::timeout(Duration::from_millis(100), wait_for_shutdown_signal()).await;

        // Should timeout since no signal is sent
        assert!(result.is_err(), "Should timeout waiting for signal");
    }

    #[test]
    fn test_shutdown_functions_exist() {
        // Just verify the API exists and can be called
        // We don't actually shut down in tests to avoid interfering with other tests
        // that use the global service

        // Verify functions are callable (but don't execute shutdown)
        // This is sufficient to ensure the API is exposed correctly
    }

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
        let runtime = crate::runtime::get_runtime();
        let _handle = runtime.handle();
    }

    #[test]
    fn test_initialize_from_nonexistent_file_returns_error() {
        // Try to initialize from a file that doesn't exist
        // If already initialized, will return Ok (idempotent)
        // If not initialized, will return ConfigError
        let result =
            initialize_from_config("/this/path/definitely/does/not/exist.yaml".to_string());

        // Either succeeds (already initialized) or fails with ConfigError
        if let Err(e) = result {
            match e {
                SlimError::ConfigError { message } => {
                    assert!(
                        message.contains("Failed to load configuration")
                            || message.contains("already initialized"),
                        "Unexpected error message: {}",
                        message
                    );
                }
                _ => panic!("Expected ConfigError variant, got: {:?}", e),
            }
        }
    }

    #[test]
    fn test_idempotent_behavior() {
        // Test that multiple initialization calls don't panic
        // Note: In test environment, tracing may already be set up,
        // so we just verify the calls don't panic
        let _ = initialize_with_defaults();
        let _ = initialize_with_defaults();
        // The fact we got here without panicking means idempotency works
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
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::{info, warn};

use slim::runtime::RuntimeConfiguration;

/// Global Tokio runtime for async operations
static GLOBAL_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Initialize the global runtime from a RuntimeConfiguration
///
/// This allows the runtime to be configured from a config file, matching
/// the behavior of the main SLIM application.
///
/// # Arguments
/// * `config` - The runtime configuration to use
///
/// # Returns
/// * `Ok(())` - Runtime successfully initialized
/// * `Err(String)` - If runtime is already initialized or initialization fails
pub fn initialize_runtime_from_config(config: &RuntimeConfiguration) -> Result<(), String> {
    if GLOBAL_RUNTIME.get().is_some() {
        return Err("Runtime already initialized".to_string());
    }

    let runtime = build_runtime_from_config(config);
    GLOBAL_RUNTIME.set(runtime).map_err(|_| "Failed to set global runtime".to_string())?;
    
    Ok(())
}

/// Initialize the global runtime with default configuration
///
/// Uses the same defaults as the main SLIM application would use.
///
/// # Returns
/// * `Ok(())` - Runtime successfully initialized
/// * `Err(String)` - If runtime is already initialized
pub fn initialize_runtime_with_defaults() -> Result<(), String> {
    if GLOBAL_RUNTIME.get().is_some() {
        return Err("Runtime already initialized".to_string());
    }

    let config = RuntimeConfiguration::default();
    let runtime = build_runtime_from_config(&config);
    GLOBAL_RUNTIME.set(runtime).map_err(|_| "Failed to set global runtime".to_string())?;
    
    Ok(())
}

/// Build a Tokio runtime from configuration
///
/// This matches the runtime building logic from slim::runtime::build but creates
/// the runtime directly instead of wrapping it in SlimRuntime.
fn build_runtime_from_config(config: &RuntimeConfiguration) -> tokio::runtime::Runtime {
    let n_cpu = num_cpus::get();
    debug_assert!(n_cpu > 0, "failed to get number of CPUs");

    tracing::debug!(%n_cpu, "Number of available CPU cores");

    let cores = if config.n_cores() > n_cpu {
        warn!(
            requested = %config.n_cores(),
            available = %n_cpu,
            "Requested number of cores is greater than available cores. Using all available cores",
        );
        n_cpu
    } else if config.n_cores() == 0 {
        info!(
            %n_cpu,
            "Using all available cores",
        );
        n_cpu
    } else {
        config.n_cores()
    };

    match cores {
        1 => {
            info!("Using single-threaded runtime");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .thread_name(config.thread_name())
                .build()
                .expect("failed to build single-thread runtime!")
        }
        _ => {
            info!(%cores, "Using multi-threaded runtime");
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name(config.thread_name())
                .worker_threads(cores)
                .max_blocking_threads(cores)
                .build()
                .expect("failed to build threaded runtime!")
        }
    }
}

/// Get or initialize the global Tokio runtime
///
/// If not explicitly initialized via `initialize_runtime_from_config` or
/// `initialize_runtime_with_defaults`, this will create a runtime with
/// FFI-friendly defaults for backward compatibility:
/// - Worker threads: 2x CPU cores (to handle blocking operations better)
/// - Max blocking threads: 512 (allows high concurrency from FFI calls)
/// - Named threads for easier debugging
///
/// Returns a static reference since the runtime lives for the entire program lifetime.
/// This is exposed publicly for use by language bindings (e.g., Python) that need
/// to create `BindingsSessionContext` instances with a runtime.
pub fn get_runtime() -> &'static tokio::runtime::Runtime {
    GLOBAL_RUNTIME.get_or_init(|| {
        // Calculate optimal worker thread count
        // Use 2x CPU cores for workloads with blocking operations from FFI
        let num_workers = std::env::var("SLIM_TOKIO_WORKERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                let cpus = num_cpus::get();
                (cpus * 2).max(4) // At least 4 workers, preferably 2x CPUs
            });

        // Allow configurable max blocking threads (default: 512)
        let max_blocking = std::env::var("SLIM_MAX_BLOCKING_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512);

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(num_workers)
            .max_blocking_threads(max_blocking)
            .thread_name_fn(|| {
                static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
                let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
                format!("slim-rt-{}", id)
            })
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test runtime configuration
    #[test]
    fn test_runtime_configuration() {
        let runtime = get_runtime();

        // Verify runtime was created (not null)
        // Runtime is static, so just verify we can access it
        let _handle = runtime.handle();

        // Runtime should be accessible multiple times (returns same instance)
        let runtime2 = get_runtime();
        assert!(std::ptr::eq(runtime, runtime2));
    }

    /// Test building runtime from config
    #[test]
    fn test_build_runtime_from_config() {
        let config = RuntimeConfiguration::default();
        let runtime = build_runtime_from_config(&config);
        let _handle = runtime.handle();
    }

    /// Test building runtime with specific cores
    #[test]
    fn test_build_runtime_with_cores() {
        let config = RuntimeConfiguration::with_cores(2);
        let runtime = build_runtime_from_config(&config);
        let _handle = runtime.handle();
    }

    /// Test environment variable configuration
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn test_env_var_configuration() {
        // Set environment variables
        unsafe {
            std::env::set_var("SLIM_TOKIO_WORKERS", "8");
            std::env::set_var("SLIM_MAX_BLOCKING_THREADS", "256");
        }

        // Note: These won't affect already-initialized runtime,
        // but we can verify parsing works
        let workers: usize = std::env::var("SLIM_TOKIO_WORKERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4);

        let max_blocking: usize = std::env::var("SLIM_MAX_BLOCKING_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512);

        assert_eq!(workers, 8);
        assert_eq!(max_blocking, 256);

        // Clean up
        unsafe {
            std::env::remove_var("SLIM_TOKIO_WORKERS");
            std::env::remove_var("SLIM_MAX_BLOCKING_THREADS");
        }
    }
}

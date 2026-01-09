// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Global Tokio runtime for async operations
static GLOBAL_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Get or initialize the global Tokio runtime
///
/// Configured for FFI workloads with:
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

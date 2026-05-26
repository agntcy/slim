// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use duration_string::DurationString;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time;
use tokio::runtime::{Builder, Handle, Runtime};
use tracing::{info, warn};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfiguration {
    /// the number of cores to use for this runtime
    #[serde(default = "default_n_cores")]
    pub n_cores: usize,

    /// the thread name for the runtime
    #[serde(default = "default_thread_name")]
    pub thread_name: String,

    /// the timeout for draining the services
    #[serde(default = "default_drain_timeout")]
    pub drain_timeout: DurationString,
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
    /// Configuration this runtime was built from.
    pub config: RuntimeConfiguration,

    /// The underlying driver — either a runtime owned and driven inline by
    /// the caller, or a `current_thread` runtime owned by a dedicated OS
    /// thread (see [`RuntimeDriver`] and [`build_for_embedding`]).
    driver: RuntimeDriver,
}

/// Backing flavor for [`SlimRuntime`].
///
/// SLIM is embeddable in two distinct ways:
///
/// * As a standalone daemon, where the caller (`runner::run`, `slimctl`,
///   integration tests) drives the runtime itself by calling `block_on` on
///   the main thread. This is the [`Inline`] variant — it works for both
///   `multi_thread` and `current_thread` runtimes.
///
/// * Through language bindings, where Rust `async fn`s are exposed to a host
///   language (Python, Kotlin, Swift, …) and the host language's own event
///   loop polls the outer future. In that mode Tokio's I/O reactor and timer
///   wheel only advance while some thread is inside a Tokio runtime context;
///   if every worker is idle because the host loop is the active driver,
///   socket readiness events and timers never fire and I/O-bound futures
///   hang indefinitely.
///
///   With a multi-worker runtime the embedded case is usually masked: at
///   least one worker stays parked on the reactor while another services
///   spawned tasks, so I/O keeps flowing. With a single worker the runtime
///   collapses to one thread that has to alternate between reactor polling
///   and task execution, and once the host loop takes over polling the
///   chain breaks. The worker count is itself environment-dependent —
///   auto-detection follows the process's CPU quota, so a container with a
///   fractional CPU limit, a pinned cpuset, or a small machine can silently
///   drop to one worker.
///
///   To make the single-worker embedded case behave consistently, use
///   [`build_for_embedding`]. It returns the [`Dedicated`] variant: a
///   `current_thread` runtime owned by a dedicated OS thread sitting in
///   `rt.block_on(...)`. That thread is always inside the runtime context,
///   so readiness and timer events are processed continuously regardless of
///   how the host language drives outer futures. External callers interact
///   with the runtime only through a [`Handle`], which routes work onto
///   that thread.
///
/// [`Inline`]: RuntimeDriver::Inline
/// [`Dedicated`]: RuntimeDriver::Dedicated
enum RuntimeDriver {
    /// Runtime owned directly. The caller drives it via `block_on` on
    /// whatever thread it likes (its own thread for `current_thread`, or
    /// any thread for `multi_thread`).
    Inline(Runtime),
    /// `current_thread` runtime owned by a dedicated OS thread that sits in
    /// `rt.block_on(...)` for its entire lifetime.
    Dedicated {
        handle: Handle,
        /// Sent on `Drop` to wake the driver thread out of `block_on`.
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        /// The OS thread that owns and drives the runtime.
        thread: Option<std::thread::JoinHandle<()>>,
    },
}

impl SlimRuntime {
    /// Return a cloneable handle to this runtime. Cheap; safe to call from any
    /// thread.
    pub fn handle(&self) -> Handle {
        match &self.driver {
            RuntimeDriver::Inline(rt) => rt.handle().clone(),
            RuntimeDriver::Dedicated { handle, .. } => handle.clone(),
        }
    }

    /// Run a future to completion on this runtime, blocking the calling
    /// thread. Must not be called from inside an existing Tokio context.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        match &self.driver {
            RuntimeDriver::Inline(rt) => rt.block_on(future),
            RuntimeDriver::Dedicated { handle, .. } => handle.block_on(future),
        }
    }
}

impl Drop for SlimRuntime {
    fn drop(&mut self) {
        if let RuntimeDriver::Dedicated {
            shutdown_tx,
            thread,
            ..
        } = &mut self.driver
        {
            if let Some(tx) = shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(handle) = thread.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Build a runtime owned and driven inline by the caller.
///
/// At `n_cores > 1` this gives you a standard multi-thread runtime. At
/// `n_cores == 1` it gives you a `current_thread` runtime — the calling
/// thread will own it for the duration of `block_on`. This is the right
/// shape for the daemon entry points.
///
/// Embedders whose calling thread is owned by a foreign event loop (e.g.
/// language bindings driven by Python `asyncio`) should use
/// [`build_for_embedding`] instead.
pub fn build(config: &RuntimeConfiguration) -> SlimRuntime {
    let cores = resolve_cores(config);
    let runtime = build_inline_runtime(config, cores);
    SlimRuntime {
        config: config.clone(),
        driver: RuntimeDriver::Inline(runtime),
    }
}

/// Build a runtime suitable for embedding inside a host language.
///
/// Identical to [`build`] except that at `n_cores == 1` it dedicates a
/// background OS thread to driving a `current_thread` runtime. See
/// [`RuntimeDriver`] for the motivation. At `n_cores > 1` this is identical
/// to [`build`] — a multi-thread runtime already keeps the I/O reactor
/// alive without help.
pub fn build_for_embedding(config: &RuntimeConfiguration) -> SlimRuntime {
    let cores = resolve_cores(config);
    let driver = if cores == 1 {
        build_dedicated_driver(config)
    } else {
        RuntimeDriver::Inline(build_inline_runtime(config, cores))
    };
    SlimRuntime {
        config: config.clone(),
        driver,
    }
}

/// Resolve the effective worker count, matching the historical semantics:
/// `0` means "all available", and oversubscription is clamped with a warning.
fn resolve_cores(config: &RuntimeConfiguration) -> usize {
    let n_cpu = num_cpus::get();
    debug_assert!(n_cpu > 0, "failed to get number of CPUs");

    tracing::debug!(%n_cpu, "Number of available CPU cores");

    if config.n_cores > n_cpu {
        warn!(
            requested = %config.n_cores,
            available = %n_cpu,
            "Requested number of cores is greater than available cores. Using all available cores",
        );
        n_cpu
    } else if config.n_cores == 0 {
        info!(%n_cpu, "Using all available cores");
        n_cpu
    } else {
        config.n_cores
    }
}

fn build_inline_runtime(config: &RuntimeConfiguration, cores: usize) -> Runtime {
    if cores == 1 {
        info!("Using single-threaded (current_thread) runtime");
        Builder::new_current_thread()
            .enable_all()
            .thread_name(config.thread_name.as_str())
            .build()
            .expect("failed to build current_thread runtime!")
    } else {
        info!(%cores, "Using multi-threaded runtime");
        Builder::new_multi_thread()
            .enable_all()
            .thread_name(config.thread_name.as_str())
            .worker_threads(cores)
            .build()
            .expect("failed to build threaded runtime!")
    }
}

fn build_dedicated_driver(config: &RuntimeConfiguration) -> RuntimeDriver {
    info!("Using single-threaded runtime driven on a dedicated OS thread");

    let (handle_tx, handle_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let thread_name = config.thread_name.clone();

    let thread = std::thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            let rt = Builder::new_current_thread()
                .enable_all()
                .thread_name(thread_name)
                .build()
                .expect("failed to build current_thread runtime!");

            // Publish the handle before parking the thread inside block_on.
            handle_tx
                .send(rt.handle().clone())
                .expect("failed to publish Tokio handle to initializer");

            // Drive the reactor until shutdown is signaled. The oneshot
            // doubles as a heartbeat that never completes on its own, so
            // I/O and timers keep being polled for the lifetime of the
            // runtime.
            let _ = rt.block_on(shutdown_rx);
        })
        .expect("failed to spawn Tokio driver thread");

    let handle = handle_rx
        .recv()
        .expect("Tokio driver thread panicked during initialization");

    RuntimeDriver::Dedicated {
        handle,
        shutdown_tx: Some(shutdown_tx),
        thread: Some(thread),
    }
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
        let runtime = build(&config);
        assert_eq!(runtime.config.n_cores, 0);
    }

    #[test]
    fn test_single_thread_runtime_drives_io() {
        // Inline `current_thread` runtime — the calling thread drives it via
        // block_on. Verify timers and async work actually progress.
        let config = RuntimeConfiguration {
            n_cores: 1,
            thread_name: "slim-single".to_string(),
            drain_timeout: time::Duration::from_secs(1).into(),
        };
        let runtime = build(&config);

        let value = runtime.block_on(async {
            tokio::time::sleep(time::Duration::from_millis(20)).await;
            42_u32
        });
        assert_eq!(value, 42);
    }

    #[test]
    fn test_build_for_embedding_dedicates_thread() {
        // With `build_for_embedding` and cores == 1, the runtime runs on a
        // dedicated OS thread. The calling thread is *not* the one driving
        // it, which is the property bindings rely on when the host
        // language's event loop owns the calling thread. Spawning a task
        // and awaiting it via the handle proves the dedicated thread is
        // alive and polling.
        let config = RuntimeConfiguration {
            n_cores: 1,
            thread_name: "slim-embed".to_string(),
            drain_timeout: time::Duration::from_secs(1).into(),
        };
        let runtime = build_for_embedding(&config);
        let handle = runtime.handle();

        let join = handle.spawn(async {
            tokio::time::sleep(time::Duration::from_millis(20)).await;
            7_u32
        });
        let value = runtime.block_on(async move { join.await.unwrap() });
        assert_eq!(value, 7);
    }

    #[test]
    fn test_runtime_builder_with_cores() {
        let config = RuntimeConfiguration {
            n_cores: 3,
            thread_name: "test".to_string(),
            drain_timeout: time::Duration::from_secs(10).into(),
        };
        let runtime = build(&config);
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
        let runtime = build(&config);
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

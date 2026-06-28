// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Runtime-agnostic task spawning.
//!
//! The data-plane core spawns long-lived tasks (per-stream processors,
//! reconnect loops, subscription forwarders). On native targets these run on
//! the multi-threaded tokio runtime and must be `Send`. In the browser the
//! futures are `!Send` (they hold the `gloo_net` JS `WebSocket`) and the
//! runtime is single-threaded; there `tokio` is `tokio_with_wasm` (a drop-in
//! that schedules onto the JS event loop via `wasm_bindgen_futures`), whose
//! `task::spawn` yields the same [`tokio::task::JoinHandle`] without the
//! `Send` bound and needs no `LocalSet`. The caller-facing API is identical on
//! both targets.

/// Spawn a runtime-appropriate task, returning a [`tokio::task::JoinHandle`].
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

/// Spawn a task on the browser event loop (`tokio_with_wasm`).
///
/// Schedules onto the JS microtask queue via `wasm_bindgen_futures`; no
/// `LocalSet` or explicit runtime is required, and the future need not be
/// `Send` (it may hold the `gloo_net` JS `WebSocket`).
#[cfg(target_arch = "wasm32")]
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    tokio::task::spawn(future)
}

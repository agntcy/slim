// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Runtime-agnostic seams that let the session layer share a single source tree
//! across the native (tokio, multi-threaded) build and the wasm32 browser build.

/// Await the expression on wasm32 (where MLS is async) and evaluate it directly
/// on every other target (where MLS is synchronous).
///
/// The enclosing function must be `async` for the wasm32 branch to compile; all
/// current call sites already live in async session methods.
macro_rules! maybe_await {
    ($e:expr) => {{
        #[cfg(target_arch = "wasm32")]
        {
            ($e).await
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            $e
        }
    }};
}

pub(crate) use maybe_await;

/// Spawn a session task, returning a [`tokio::task::JoinHandle`].
///
/// On native targets this is `tokio::spawn` (multi-threaded runtime, requires a
/// `Send` future). On wasm32 the session futures are `!Send` (they await the
/// SubtleCrypto-backed MLS provider) and the runtime is single-threaded, so we
/// use `tokio::task::spawn_local`, which yields the same `JoinHandle` type
/// without the `Send` requirement. Both keep the caller-facing API identical.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    tokio::task::spawn_local(future)
}

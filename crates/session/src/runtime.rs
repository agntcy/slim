// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Runtime-agnostic seams that let the session layer share a single source tree
//! across the native (tokio, multi-threaded) build and the wasm32 browser build.

use std::pin::Pin;
use std::time::Duration;

/// A `tokio::time::Sleep` primed to fire effectively never. Used to seed a
/// `select!` shutdown deadline that is only armed for real on graceful shutdown
/// (see [`reset_shutdown_deadline`]).
///
/// Native tokio accepts `Duration::MAX`. In the browser the timer is backed by
/// JS `setTimeout`, whose delay is a 32-bit millisecond value, so we cap at
/// `i32::MAX` ms (~24.8 days) to avoid silent truncation/overflow.
pub(crate) fn infinite_sleep() -> tokio::time::Sleep {
    #[cfg(not(target_arch = "wasm32"))]
    let duration = Duration::MAX;
    #[cfg(target_arch = "wasm32")]
    let duration = Duration::from_millis(i32::MAX as u64);
    tokio::time::sleep(duration)
}

/// Re-arm a pinned shutdown-deadline `Sleep` to fire `timeout` from now.
///
/// Native tokio's `Sleep` is resettable via a deadline `Instant`. The browser
/// (`tokio_with_wasm`) `Sleep` has no `reset`/`Instant`, so we replace the
/// pinned future in place with a fresh timeout instead.
pub(crate) fn reset_shutdown_deadline(
    shutdown_deadline: &mut Pin<&mut tokio::time::Sleep>,
    timeout: Duration,
) {
    #[cfg(not(target_arch = "wasm32"))]
    shutdown_deadline
        .as_mut()
        .reset(tokio::time::Instant::now() + timeout);
    #[cfg(target_arch = "wasm32")]
    shutdown_deadline.set(tokio::time::sleep(timeout));
}

/// A delay future used as an ACK timeout in `select`.
///
/// On native this uses [`futures_timer::Delay`] rather than
/// `tokio::time::timeout` so it works outside a Tokio runtime with the time
/// driver enabled (e.g. UniFFI async bindings). On wasm `futures_timer`'s
/// fallback panics on `Instant::now`, so we use `tokio_with_wasm`'s `sleep`,
/// which is driven by `setTimeout` on the JS event loop.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ack_timeout_delay(timeout: Duration) -> impl std::future::Future<Output = ()> {
    futures_timer::Delay::new(timeout)
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn ack_timeout_delay(timeout: Duration) -> impl std::future::Future<Output = ()> {
    tokio::time::sleep(timeout)
}

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
pub(crate) use slim_datapath::runtime::spawn;

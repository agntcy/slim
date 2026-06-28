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
pub(crate) use slim_datapath::runtime::spawn;

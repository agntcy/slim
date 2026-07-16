// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Process-global Tokio runtime for the synchronous convenience wrappers.
//!
//! Async callers should prefer the `*_async` methods and drive them on their own
//! runtime; the blocking wrappers exist only for callers without an ambient one.

use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Returns a handle to the process-global multi-threaded runtime, building it on
/// first use.
pub fn get_runtime() -> Handle {
    RUNTIME
        .get_or_init(|| Runtime::new().expect("failed to build the slimrpc runtime"))
        .handle()
        .clone()
}

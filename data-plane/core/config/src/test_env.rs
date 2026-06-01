// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Test-only synchronization for process environment mutations.

use std::sync::Mutex;

/// Serializes tests that set or clear `http_proxy` / related env vars.
pub(crate) static PROXY_ENV_LOCK: Mutex<()> = Mutex::new(());

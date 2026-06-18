// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Test-only synchronization for process environment mutations.

/// Serializes tests that set or clear `http_proxy` / related env vars.
pub(crate) static PROXY_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

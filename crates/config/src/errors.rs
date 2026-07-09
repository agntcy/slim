// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `ConfigError` is target-specific: the native build carries the full
//! gRPC/TLS/auth/native-WebSocket surface, while the browser build only needs
//! the errors reachable from `ClientConfig` validation and the gloo-net
//! connect. Each lives in its own file (file-split rather than an in-line
//! `cfg_if!` so "what exists on wasm" is answerable by `ls`).

#[cfg(not(target_arch = "wasm32"))]
#[path = "errors/native.rs"]
mod imp;
#[cfg(target_arch = "wasm32")]
#[path = "errors/wasm.rs"]
mod imp;

pub use imp::ConfigError;

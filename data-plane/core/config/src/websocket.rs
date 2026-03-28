// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// When both native and wasm features are enabled (e.g. via --all-features),
// native takes precedence via the not(feature = "native") guards below.

#[cfg(feature = "native")]
#[path = "websocket/client.rs"]
pub mod client;
#[cfg(all(feature = "wasm", not(feature = "native")))]
#[path = "websocket/client_wasm.rs"]
pub mod client;

#[cfg(feature = "native")]
#[path = "websocket/common.rs"]
pub mod common;
#[cfg(all(feature = "wasm", not(feature = "native")))]
#[path = "websocket/common_wasm.rs"]
pub mod common;

#[cfg(feature = "native")]
pub mod server;

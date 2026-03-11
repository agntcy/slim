// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(all(feature = "websocket-native", feature = "websocket-wasm"))]
compile_error!("Enable only one websocket transport feature at a time.");

#[cfg(feature = "websocket-native")]
#[path = "websocket/client.rs"]
pub mod client;
#[cfg(all(feature = "websocket-wasm", not(feature = "websocket-native")))]
#[path = "websocket/client_wasm.rs"]
pub mod client;

#[cfg(feature = "websocket-native")]
#[path = "websocket/common.rs"]
pub mod common;
#[cfg(all(feature = "websocket-wasm", not(feature = "websocket-native")))]
#[path = "websocket/common_wasm.rs"]
pub mod common;

#[cfg(feature = "websocket-native")]
pub mod server;

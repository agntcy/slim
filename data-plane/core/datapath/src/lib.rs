// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod api;
pub mod errors;
pub mod messages;
pub mod peer_discovery;
pub mod sync;
pub mod tables;

// TODO(wasm32): provide a wasm-friendly transport (gloo-net WS + tonic-web-wasm-client)
// and re-enable the forwarder / header_mac / negotiation modules behind it.
#[cfg(not(target_arch = "wasm32"))]
pub mod connection;
#[cfg(not(target_arch = "wasm32"))]
pub mod forwarder;
#[cfg(not(target_arch = "wasm32"))]
mod header_mac;
#[cfg(not(target_arch = "wasm32"))]
mod link_ecdh;
#[cfg(not(target_arch = "wasm32"))]
pub mod message_processing;
#[cfg(not(target_arch = "wasm32"))]
mod negotiation;
#[cfg(all(feature = "otel_tracing", not(target_arch = "wasm32")))]
mod otel_tracing;
#[cfg(not(target_arch = "wasm32"))]
mod websocket;

#[cfg(not(target_arch = "wasm32"))]
pub use tonic::Status;

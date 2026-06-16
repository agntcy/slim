// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod api;
pub mod errors;
pub mod messages;
pub mod tables;

cfg_if::cfg_if! {
if #[cfg(not(target_arch = "wasm32"))] {
pub mod connection;
pub mod forwarder;
mod header_mac;
mod link_ecdh;
pub mod message_processing;
mod negotiation;
#[cfg(feature = "otel_tracing")]
mod otel_tracing;
pub mod peer_discovery;
pub mod sync;
mod websocket;

pub use tonic::Status;
}}

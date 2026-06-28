// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod api;
pub mod errors;
pub mod messages;
pub mod runtime;
pub mod tables;

// Most transport-neutral data-plane logic is shared between native and wasm32:
// connection state, forwarding, negotiation, synchronization, and message
// processing. wasm32 supports outgoing WebSocket connections only. The gRPC
// transport, native fastwebsockets implementation, inbound WebSocket server
// handling, peer discovery, OpenTelemetry integration, and native
// peer-discovery lifecycle management are native-only.
pub mod connection;
pub mod forwarder;
// Public link-integrity primitives for external transports. Built-in clients
// use them indirectly through MessageProcessor::connect and internal link
// negotiation.
pub mod header_mac;
pub mod link_ecdh;
pub mod message_processing;
mod negotiation;
pub mod sync;
pub mod websocket;

// These complete modules have no wasm32 implementation and are excluded here.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        #[cfg(feature = "otel_tracing")]
        mod otel_tracing;
        pub mod peer_discovery;
    }
}

pub use tonic::Status;

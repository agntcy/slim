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
// handling, peer discovery, and native peer-discovery lifecycle management are
// native-only. OpenTelemetry span export (OTLP) is native-only; with the
// `otel_tracing` feature, wasm32 participates in distributed trace propagation
// via SLIM message metadata (see `slim_tracing` browser init).
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

#[cfg(feature = "otel_tracing")]
mod otel_tracing;

// Peer discovery has no wasm32 implementation and is excluded here.
#[cfg(not(target_arch = "wasm32"))]
pub mod peer_discovery;

pub use tonic::Status;

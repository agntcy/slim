// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod api;
pub mod errors;
pub mod messages;
pub mod tables;

// TODO(wasm32): provide a wasm-friendly transport (gloo-net WS + tonic-web-wasm-client)
// and re-enable the forwarder / header_mac / negotiation / sync modules behind it.
// `peer_discovery` and `sync` (peer/remote subscription synchronization) drive the
// native connection runtime (client config, tokio tasks, drain, connection handles),
// so they stay gated for now.
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
    }
}

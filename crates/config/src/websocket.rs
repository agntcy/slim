// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod common;

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        pub mod client;
        pub mod query_token_layer;
        pub mod server;
    } else {
        // The gloo-net browser client is re-exported as `client` so call sites
        // (`crate::websocket::client::…`) are transport-agnostic.
        pub mod client_wasm;
        pub use client_wasm as client;
    }
}

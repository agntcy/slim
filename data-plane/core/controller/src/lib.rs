// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// TODO(wasm32): port the control-plane client to a browser-friendly transport.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        pub mod api;
        pub mod config;
        pub mod errors;
        pub mod service;
    }
}

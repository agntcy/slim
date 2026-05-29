// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// TODO(wasm32): provide a lighter browser runtime entrypoint.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        pub mod args;
        pub mod build_info;
        pub mod config;
        pub mod runner;
        pub mod runtime;
    }
}

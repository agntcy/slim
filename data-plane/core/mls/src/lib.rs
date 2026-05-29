// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// TODO(wasm32): wire mls-rs-crypto-webcrypto so MLS works in the browser.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        pub mod errors;
        pub mod identity_provider;
        pub mod mls;
    }
}

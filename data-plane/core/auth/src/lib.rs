// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// TODO(wasm32): port slim_auth (shared_secret, JWT/OIDC/SPIFFE, middleware) to the browser.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        pub mod errors;
        pub mod identity_claims;
        pub mod metadata;
        pub mod shared_secret;
        pub mod traits;

        pub mod auth_provider;
        pub mod builder;
        pub mod file_watcher;
        pub mod jwt;
        pub mod jwt_middleware;
        pub mod oidc;
        pub mod resolver;
        #[cfg(not(target_family = "windows"))]
        pub mod spire;
        pub mod utils;
    }
}

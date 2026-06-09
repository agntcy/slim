// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Core modules available on every target (including wasm32): error types,
// identity claims, metadata maps, the SharedSecret token provider and the
// auth traits. SharedSecret uses a pure-Rust HMAC backend on wasm32.
pub mod errors;
pub mod identity_claims;
pub(crate) mod mac;
pub mod metadata;
pub mod shared_secret;
pub mod traits;

// Native-only modules: JWT/OIDC/SPIFFE, middleware, file watching and the
// AWS-LC-backed MLS key generation helpers. These pull in tokio, reqwest,
// jsonwebtoken and aws-lc-rs, none of which build for wasm32-unknown-unknown.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
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

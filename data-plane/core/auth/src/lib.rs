// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Core modules available on every target
pub mod errors;
pub mod identity_claims;
pub(crate) mod mac;
pub mod metadata;
pub mod shared_secret;
pub mod traits;

// Native-only modules
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

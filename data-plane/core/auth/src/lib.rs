// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "native")]
pub mod auth_provider;
#[cfg(feature = "native")]
pub mod builder;
pub mod errors;
#[cfg(feature = "native")]
pub mod file_watcher;
pub mod identity_claims;
#[cfg(feature = "native")]
pub mod jwt;
#[cfg(feature = "native")]
pub mod jwt_middleware;
pub mod metadata;
#[cfg(feature = "native")]
pub mod oidc;
#[cfg(feature = "native")]
pub mod resolver;
pub mod shared_secret;
#[cfg(all(feature = "native", not(target_family = "windows")))]
pub mod spire;
pub mod traits;
#[cfg(feature = "native")]
pub mod utils;

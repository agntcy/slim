// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! MLS crypto provider selection based on the compilation target.
//!
//! * Native targets use the AWS-LC crypto provider.
//! * `wasm32` targets use the WebCrypto provider (browser SubtleCrypto). It is
//!   async-only, which is why the MLS API is compiled in `mls_build_async` mode
//!   on wasm32 (see `data-plane/.cargo/config.toml`).

#[cfg(not(target_arch = "wasm32"))]
pub use mls_rs_crypto_awslc::AwsLcCryptoProvider as CryptoProviderImpl;

#[cfg(target_arch = "wasm32")]
pub use mls_rs_crypto_webcrypto::WebCryptoProvider as CryptoProviderImpl;

/// Construct the default crypto provider for the active target.
// `AwsLcCryptoProvider` (native) is a regular struct that requires `default()`,
// whereas `WebCryptoProvider` (wasm) is a unit struct for which clippy would
// otherwise prefer the bare value. Keep the uniform `default()` call for both.
#[allow(clippy::default_constructed_unit_structs)]
pub fn default_crypto_provider() -> CryptoProviderImpl {
    CryptoProviderImpl::default()
}

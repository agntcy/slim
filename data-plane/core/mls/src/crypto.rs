// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Crypto provider selection based on compilation target and features.
//! 
//! - `native` feature (default): Uses AWS-LC crypto provider for native targets
//! - `wasm` feature: Uses WebCrypto provider for browser/WASM targets

#[cfg(feature = "native")]
pub use mls_rs_crypto_awslc::AwsLcCryptoProvider as CryptoProviderImpl;

#[cfg(all(feature = "wasm", not(feature = "native")))]
pub use mls_rs_crypto_webcrypto::WebCryptoProvider as CryptoProviderImpl;

pub fn default_crypto_provider() -> CryptoProviderImpl {
    CryptoProviderImpl::default()
}

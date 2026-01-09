// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::runtime;

/// Initialize the crypto provider
///
/// This must be called before any TLS operations. It's safe to call multiple times.
#[uniffi::export]
pub fn initialize_crypto_provider() {
    // Initialize the crypto provider in the slim_config module
    slim_config::tls::provider::initialize_crypto_provider();

    // Also initialize the global runtime for async operations
    let _ = runtime::get_runtime();
}

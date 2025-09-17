// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Once;

static RUSTLS: Once = Once::new();

pub fn initialize_crypto_provider() {
    RUSTLS.call_once(|| {
        // Set aws-lc as default crypto provider
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .unwrap();
    });
}

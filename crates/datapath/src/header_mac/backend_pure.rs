// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Pure-Rust HMAC-SHA256 backend (`hmac` + `sha2`).
//!
//! This is the browser backend (`aws_lc_rs` is native-only). It is also compiled
//! into native **test** builds so the native coverage run instruments it and can
//! assert byte-for-byte parity with the [`super::backend_awslc`] backend — the
//! property that makes a native↔browser link interoperable.

use hmac::{Hmac, Mac};

/// Per-link HMAC key material (the raw secret; `hmac::Hmac` is constructed per call).
pub type MacKey = Vec<u8>;

pub fn new(secret: &[u8]) -> MacKey {
    secret.to_vec()
}

pub fn sign(key: &MacKey, data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

pub fn verify(key: &MacKey, data: &[u8], tag: &[u8]) -> bool {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.verify_slice(tag).is_ok()
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Native HMAC-SHA256 backend using `aws_lc_rs`.
//!
//! Selected on every non-wasm target. Computes an RFC 2104 HMAC-SHA256 that is
//! byte-for-byte identical to the pure-Rust [`super::backend_pure`] backend, so
//! signatures interoperate across a native↔browser link (pinned by the
//! `pure_hmac_backend_matches_production_backend` parity test).

use aws_lc_rs::hmac;

/// Per-link HMAC key material.
pub type MacKey = hmac::Key;

pub fn new(secret: &[u8]) -> MacKey {
    hmac::Key::new(hmac::HMAC_SHA256, secret)
}

pub fn sign(key: &MacKey, data: &[u8]) -> Vec<u8> {
    Vec::from(hmac::sign(key, data).as_ref())
}

pub fn verify(key: &MacKey, data: &[u8], tag: &[u8]) -> bool {
    hmac::verify(key, data, tag).is_ok()
}

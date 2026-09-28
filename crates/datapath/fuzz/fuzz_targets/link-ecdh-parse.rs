// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Peer key material parsing for inter-node link key exchange
//! (`crates/datapath/src/link_ecdh.rs` and its two backends,
//! `link_ecdh/backend_awslc.rs` and `link_ecdh/backend_pure.rs`).
//!
//! During link negotiation (`crates/datapath/src/negotiation.rs`) a node
//! accepts, from an unauthenticated peer, an X25519 public key and
//! optionally ML-KEM-768 encapsulation material (a public key from the
//! initiator, a ciphertext back from the responder) before any header MAC
//! exists to validate the sender. Parsing/accepting that peer-controlled
//! material must reject malformed input rather than panic.
//!
//! There are two independent backends for this: `backend_awslc` (native
//! production, via `aws_lc_rs`) and `backend_pure` (the wasm/browser
//! production backend, also compiled into native test and, via the
//! `fuzzing` feature, native fuzz builds). A native↔browser link only works
//! if both sides agree on what is a valid key, so this target feeds the
//! *same* fuzzer-supplied bytes to both backends' parsing paths and asserts
//! they reach the same accept/reject decision -- a divergence between them
//! is a bug in its own right, independent of any panic.
//!
//! Invariants:
//! - Parsing an X25519 peer public key, an ML-KEM-768 peer public key
//!   (encapsulation), and an ML-KEM-768 ciphertext (decapsulation) never
//!   panics in either backend, for any bytes.
//! - `backend_awslc` and `backend_pure` accept or reject identical
//!   peer-supplied bytes identically, for all three of the above.

#![no_main]

use libfuzzer_sys::fuzz_target;
use slim_datapath::link_ecdh::fuzzing_backend_awslc as awslc;
use slim_datapath::link_ecdh::fuzzing_backend_pure as pure;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }

    // The fuzzer controls the split points directly, so it can freely
    // explore short/long/boundary-length peer material for each of the
    // three checks below.
    let x_len = data[0] as usize;
    let pk_len = data[1] as usize;
    let rest = &data[2..];

    let x_len = x_len.min(rest.len());
    let (x_bytes, rest) = rest.split_at(x_len);
    let pk_len = pk_len.min(rest.len());
    let (pk_bytes, ct_bytes) = rest.split_at(pk_len);

    // --- X25519 peer public key parsing ---
    // Ephemeral DH private keys are single-use by API design in both
    // backends (consumed by `derive`), so generate a fresh local keypair per
    // backend per call, and feed the *same* peer bytes to both.
    if let (Ok((sk_a, _)), Ok((sk_b, _))) = (awslc::generate(), pure::generate()) {
        let awslc_ok = awslc::derive(sk_a, x_bytes, "fuzz-link").is_ok();
        let pure_ok = pure::derive(sk_b, x_bytes, "fuzz-link").is_ok();
        assert_eq!(
            awslc_ok,
            pure_ok,
            "backend_awslc and backend_pure disagree on a {}-byte X25519 peer public key",
            x_bytes.len()
        );
    }

    // --- ML-KEM-768 peer encapsulation key parsing ---
    // `encapsulate_mlkem768` only borrows the peer bytes, so both backends
    // can be called directly against the same slice with no per-call setup.
    let awslc_encap_ok = awslc::encapsulate_mlkem768(pk_bytes).is_ok();
    let pure_encap_ok = pure::encapsulate_mlkem768(pk_bytes).is_ok();
    assert_eq!(
        awslc_encap_ok,
        pure_encap_ok,
        "backend_awslc and backend_pure disagree on a {}-byte ML-KEM-768 peer public key",
        pk_bytes.len()
    );

    // --- ML-KEM-768 peer ciphertext parsing (decapsulation) ---
    // Decapsulation keys are consumed per call (matches real usage: one
    // negotiation, one decapsulation), so generate a fresh one per backend
    // and feed the *same* peer-supplied ciphertext bytes to both.
    if let (Ok((dk_a, _)), Ok((dk_b, _))) = (awslc::generate_mlkem768(), pure::generate_mlkem768())
    {
        let awslc_decap_ok = awslc::decapsulate_mlkem768(dk_a, ct_bytes).is_ok();
        let pure_decap_ok = pure::decapsulate_mlkem768(dk_b, ct_bytes).is_ok();
        assert_eq!(
            awslc_decap_ok,
            pure_decap_ok,
            "backend_awslc and backend_pure disagree on a {}-byte ML-KEM-768 ciphertext",
            ct_bytes.len()
        );
    }
});

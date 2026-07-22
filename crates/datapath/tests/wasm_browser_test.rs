// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser-runtime unit tests for wasm-only data-plane code.
//!
//! These validate behavior that only exists (or only differs) on the
//! `wasm32-unknown-unknown` target and therefore cannot run in the native test
//! suite:
//!   * the `tokio_with_wasm` task spawner (`runtime::spawn`), which drops the
//!     `Send` bound the native spawner requires;
//!   * the pure-Rust crypto backends running in the *actual* wasm runtime with
//!     the `wasm_js` `getrandom` backend (the native suite exercises the same
//!     code but with the OS `getrandom` backend).
//!
//! They do NOT contribute to the native `cargo llvm-cov` coverage report — that
//! build never compiles `cfg(target_arch = "wasm32")` code. Their purpose is
//! functional assurance that the browser build works.
//!
//! Run with `task test:wasm` (which sets the wasm-bindgen-test runner), or:
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     cargo test -p agntcy-slim-datapath --target wasm32-unknown-unknown \
//!     --test wasm_browser_test
//!
//! The `wasm-bindgen-test-runner` binary must match the locked `wasm-bindgen`
//! version, otherwise it aborts before running any test.
#![cfg(target_arch = "wasm32")]

use slim_datapath::api::proto::dataplane::v1::SlimHeader;
use slim_datapath::header_mac::{HeaderMacError, HeaderMacSession};
use slim_datapath::link_ecdh::{
    decapsulate_mlkem768, derive_header_mac_from_ecdh, derive_header_mac_hybrid,
    encapsulate_mlkem768, generate_mlkem768, generate_x25519_ephemeral,
    ML_KEM768_CIPHERTEXT_LEN, ML_KEM768_PUBLIC_KEY_LEN,
};
use slim_datapath::messages::utils::DEFAULT_TTL;
use slim_datapath::runtime;

use wasm_bindgen_test::wasm_bindgen_test;

const LINK_ID: &str = "browser-link-id";

fn empty_header() -> SlimHeader {
    SlimHeader {
        source: None,
        destination: None,
        identity: "i".into(),
        fanout: 0,
        version: String::new(),
        recv_from: None,
        forward_to: None,
        incoming_conn: None,
        error: None,
        header_mac: None,
        ttl: DEFAULT_TTL,
        e2e_header_sig: None,
    }
}

// The browser spawner drives a future to completion and its JoinHandle resolves
// to the future's output.
#[wasm_bindgen_test]
async fn runtime_spawn_runs_future() {
    let handle = runtime::spawn(async { 21 * 2 });
    let value = handle.await.expect("spawned task should not fail");
    assert_eq!(value, 42);
}

// A `!Send` future (captures an `Rc`) is accepted by the wasm spawner — the
// reason the browser variant drops the `Send` bound the native one requires.
#[wasm_bindgen_test]
async fn runtime_spawn_accepts_non_send_future() {
    let (tx, rx) = tokio::sync::oneshot::channel::<u32>();
    let not_send = std::rc::Rc::new(7u32);
    runtime::spawn(async move {
        let _ = tx.send(*not_send);
    });
    assert_eq!(rx.await.expect("sender dropped"), 7);
}

// HMAC-SHA256 header integrity works end-to-end in the wasm runtime (pure-Rust
// `hmac`/`sha2` backend).
#[wasm_bindgen_test]
fn header_mac_sign_verify_round_trip() {
    let key = b"01234567890123456789012345678901";
    let mac = HeaderMacSession::new(key).expect("valid key");
    let mut hdr = empty_header();
    mac.sign_slim_header(&mut hdr, LINK_ID).expect("sign");
    assert!(hdr.header_mac.is_some(), "tag attached");
    mac.verify_slim_header(&hdr, LINK_ID).expect("verify");

    hdr.fanout = 99;
    assert!(
        matches!(
            mac.verify_slim_header(&hdr, LINK_ID),
            Err(HeaderMacError::VerificationFailed)
        ),
        "tampered header must fail"
    );
}

// X25519 keypair generation uses the `wasm_js` getrandom backend, and two peers
// derive a matching link MAC key so a header signed by one verifies with the
// other. This is the path that can only be validated in a real wasm runtime.
#[wasm_bindgen_test]
fn ecdh_hkdf_agrees_between_peers() {
    let (init_sk, init_pk) = generate_x25519_ephemeral().expect("init keypair");
    let (resp_sk, resp_pk) = generate_x25519_ephemeral().expect("resp keypair");
    assert_ne!(init_pk, resp_pk, "getrandom must yield distinct keys");

    let initiator = derive_header_mac_from_ecdh(init_sk, &resp_pk, LINK_ID).expect("init derive");
    let responder = derive_header_mac_from_ecdh(resp_sk, &init_pk, LINK_ID).expect("resp derive");

    let mut hdr = empty_header();
    initiator.sign_slim_header(&mut hdr, LINK_ID).expect("sign");
    responder
        .verify_slim_header(&hdr, LINK_ID)
        .expect("cross-peer verify");
}

// A malformed peer public key (wrong length) is rejected by key agreement.
#[wasm_bindgen_test]
fn ecdh_rejects_bad_peer_key() {
    let (sk, _pk) = generate_x25519_ephemeral().expect("keypair");
    let err = derive_header_mac_from_ecdh(sk, &[0u8; 8], LINK_ID).unwrap_err();
    assert!(matches!(err, HeaderMacError::KeyAgreement));
}

#[wasm_bindgen_test]
fn hybrid_link_mac_agrees_between_initiator_and_responder() {
    let link_id = "browser-hybrid-link-id";

    let (init_x_sk, init_x_pk) = generate_x25519_ephemeral().expect("init x25519");
    let (resp_x_sk, resp_x_pk) = generate_x25519_ephemeral().expect("resp x25519");
    let (init_ml_sk, init_ml_pk) = generate_mlkem768().expect("init ml-kem");
    assert_eq!(init_ml_pk.len(), ML_KEM768_PUBLIC_KEY_LEN);

    let (ct, ml_shared) = encapsulate_mlkem768(&init_ml_pk).expect("encapsulate");
    assert_eq!(ct.len(), ML_KEM768_CIPHERTEXT_LEN);

    let ml_shared_resp = decapsulate_mlkem768(init_ml_sk, &ct).expect("decapsulate");
    assert_eq!(ml_shared, ml_shared_resp);

    let initiator = derive_header_mac_hybrid(init_x_sk, &resp_x_pk, &ml_shared, link_id)
        .expect("init hybrid derive");
    let responder = derive_header_mac_hybrid(resp_x_sk, &init_x_pk, &ml_shared, link_id)
        .expect("resp hybrid derive");

    let mut hdr = empty_header();
    initiator.sign_slim_header(&mut hdr, link_id).expect("sign");
    responder.verify_slim_header(&hdr, link_id).expect("verify");
}

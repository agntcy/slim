// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! X25519 ECDH + HKDF-SHA256 for per-inter-node-link HMAC keys.
//!
//! Native uses `aws_lc_rs`; the browser build uses the pure-Rust
//! `x25519-dalek` + `hkdf` + `sha2` crates, seeded from the workspace
//! `getrandom` (0.3, `wasm_js` backend). Both implement the same X25519 DH and
//! HKDF-SHA256 expansion, so a native↔browser link derives the same key.

use crate::header_mac::{HeaderMacError, HeaderMacSession};

/// Wire encoding length for an X25519 public key in this protocol.
pub const X25519_PUBLIC_KEY_LEN: usize = 32;

const HKDF_INFO: &[u8] = b"SLIM-DP-inter-node-hmac-v1";

// ECDH + HKDF backend selection. Native uses `aws_lc_rs`; the browser build uses
// the pure-Rust `x25519-dalek` + `hkdf` + `sha2` crates. Both implement the same
// X25519 DH and HKDF-SHA256 expansion, so a native↔browser link derives the same
// key. Each backend exposes the same surface (`EphemeralKey`, `generate`,
// `derive`), so the public functions below are one-line forwarders with no
// `#[cfg]`.
#[cfg(not(target_arch = "wasm32"))]
#[path = "link_ecdh/backend_awslc.rs"]
mod backend;
#[cfg(target_arch = "wasm32")]
#[path = "link_ecdh/backend_pure.rs"]
mod backend;

// The pure backend is additionally compiled into native test builds (under a
// distinct name) so the parity tests can exercise it. On wasm the production
// backend already *is* the pure one.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "link_ecdh/backend_pure.rs"]
mod backend_pure;

/// Opaque ephemeral private key, backend-specific. Held between
/// [`generate_x25519_ephemeral`] and [`derive_header_mac_from_ecdh`].
pub type EphemeralKey = backend::EphemeralKey;

/// Generate an ephemeral X25519 keypair for link negotiation.
pub fn generate_x25519_ephemeral() -> Result<(EphemeralKey, Vec<u8>), HeaderMacError> {
    backend::generate()
}

/// Derive a [`HeaderMacSession`] from our ephemeral secret and the peer's public key.
pub fn derive_header_mac_from_ecdh(
    my_private: EphemeralKey,
    peer_public: &[u8],
    link_id: &str,
) -> Result<HeaderMacSession, HeaderMacError> {
    backend::derive(my_private, peer_public, link_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::utils::DEFAULT_TTL;

    // The pure backend under its native-test name, or the production backend on
    // wasm (where it already is the pure one). Used by the parity tests below.
    #[cfg(not(target_arch = "wasm32"))]
    use super::backend_pure;
    #[cfg(target_arch = "wasm32")]
    use super::backend as backend_pure;

    fn empty_header() -> crate::api::proto::dataplane::v1::SlimHeader {
        crate::api::proto::dataplane::v1::SlimHeader {
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

    #[test]
    fn ecdh_hkdf_matches_between_initiator_and_responder() {
        let link_id = uuid::Uuid::new_v4().to_string();
        let (init_sk, init_pk) = generate_x25519_ephemeral().unwrap();
        let (resp_sk, resp_pk) = generate_x25519_ephemeral().unwrap();
        let a = derive_header_mac_from_ecdh(init_sk, resp_pk.as_slice(), &link_id).unwrap();
        let b = derive_header_mac_from_ecdh(resp_sk, init_pk.as_slice(), &link_id).unwrap();
        let mut h = empty_header();
        a.sign_slim_header(&mut h, &link_id).unwrap();
        b.verify_slim_header(&h, &link_id).unwrap();
    }

    // Drives the browser ECDH+HKDF backend on the native coverage run: two peers
    // negotiate over the pure-Rust path and derive a matching link MAC key, so a
    // header signed by one verifies with the other. The public key wire length
    // also matches the protocol constant.
    #[test]
    fn pure_ecdh_backend_peers_agree() {
        let link_id = uuid::Uuid::new_v4().to_string();
        let (init_sk, init_pk) = backend_pure::generate().unwrap();
        let (resp_sk, resp_pk) = backend_pure::generate().unwrap();
        assert_eq!(init_pk.len(), X25519_PUBLIC_KEY_LEN);
        assert_eq!(resp_pk.len(), X25519_PUBLIC_KEY_LEN);
        assert_ne!(init_pk, resp_pk, "distinct keypairs");

        let initiator = backend_pure::derive(init_sk, &resp_pk, &link_id).unwrap();
        let responder = backend_pure::derive(resp_sk, &init_pk, &link_id).unwrap();

        let mut h = empty_header();
        initiator.sign_slim_header(&mut h, &link_id).unwrap();
        responder.verify_slim_header(&h, &link_id).unwrap();
    }

    // A different link_id (HKDF salt) yields a different key, so a header signed
    // for one link must not verify on another even with the same peers.
    #[test]
    fn pure_ecdh_backend_key_is_bound_to_link_id() {
        let (init_sk, init_pk) = backend_pure::generate().unwrap();
        let (resp_sk, resp_pk) = backend_pure::generate().unwrap();

        let a = backend_pure::derive(init_sk, &resp_pk, "link-a").unwrap();
        let b = backend_pure::derive(resp_sk, &init_pk, "link-b").unwrap();

        let mut h = empty_header();
        a.sign_slim_header(&mut h, "link-a").unwrap();
        assert!(
            b.verify_slim_header(&h, "link-b").is_err(),
            "keys derived under different link_ids must not interoperate"
        );
    }

    // A malformed peer public key (wrong length) is rejected by key agreement.
    #[test]
    fn pure_ecdh_backend_rejects_bad_peer_key() {
        let (sk, _pk) = backend_pure::generate().unwrap();
        let err = backend_pure::derive(sk, &[0u8; 8], "link").unwrap_err();
        assert!(matches!(err, HeaderMacError::KeyAgreement));
    }
}

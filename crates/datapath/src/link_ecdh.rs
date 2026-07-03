// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! X25519 ECDH + HKDF-SHA256 for per-inter-node-link HMAC keys.
//!
//! Native uses `aws_lc_rs`; the browser build uses the pure-Rust
//! `x25519-dalek` + `hkdf` + `sha2` crates, seeded from the workspace
//! `getrandom` (0.3, `wasm_js` backend). Both implement the same X25519 DH and
//! HKDF-SHA256 expansion, so a native↔browser link derives the same key.

#[cfg(not(target_arch = "wasm32"))]
use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
#[cfg(not(target_arch = "wasm32"))]
use aws_lc_rs::hkdf;
#[cfg(not(target_arch = "wasm32"))]
use aws_lc_rs::rand::SystemRandom;

use crate::header_mac::{HeaderMacError, HeaderMacSession};

/// Wire encoding length for an X25519 public key in this protocol.
pub const X25519_PUBLIC_KEY_LEN: usize = 32;

const HKDF_INFO: &[u8] = b"SLIM-DP-inter-node-hmac-v1";

/// Opaque ephemeral private key, backend-specific. Held between
/// [`generate_x25519_ephemeral`] and [`derive_header_mac_from_ecdh`].
#[cfg(not(target_arch = "wasm32"))]
pub type EphemeralKey = EphemeralPrivateKey;
#[cfg(target_arch = "wasm32")]
pub type EphemeralKey = x25519_dalek::StaticSecret;

/// Generate an ephemeral X25519 keypair for link negotiation.
#[cfg(not(target_arch = "wasm32"))]
pub fn generate_x25519_ephemeral() -> Result<(EphemeralKey, Vec<u8>), HeaderMacError> {
    let rng = SystemRandom::new();
    EphemeralPrivateKey::generate(&agreement::X25519, &rng)
        .map_err(|_| {
            HeaderMacError::KeyGenerationFailed("private key generation failed".to_string())
        })
        .and_then(|sk| {
            sk.compute_public_key()
                .map_err(|_| {
                    HeaderMacError::KeyGenerationFailed("public key generation failed".to_string())
                })
                .map(|pk| (sk, pk.as_ref().to_vec()))
        })
}

#[cfg(target_arch = "wasm32")]
pub fn generate_x25519_ephemeral() -> Result<(EphemeralKey, Vec<u8>), HeaderMacError> {
    pure_ecdh::generate()
}

/// Pure-Rust X25519 ECDH + HKDF-SHA256 backend (`x25519-dalek` + `hkdf`,
/// seeded from `getrandom`).
///
/// This is the browser backend (`aws_lc_rs` is native-only). It is also compiled
/// into native **test** builds so the native coverage run instruments it and can
/// assert that two peers derive the same link MAC key — the property that lets a
/// native↔browser link interoperate.
#[cfg(any(target_arch = "wasm32", test))]
mod pure_ecdh {
    use x25519_dalek::{PublicKey, StaticSecret};

    use super::{HKDF_INFO, HeaderMacError, HeaderMacSession, X25519_PUBLIC_KEY_LEN};

    pub(super) fn generate() -> Result<(StaticSecret, Vec<u8>), HeaderMacError> {
        let mut secret = [0u8; X25519_PUBLIC_KEY_LEN];
        getrandom::fill(&mut secret)
            .map_err(|e| HeaderMacError::KeyGenerationFailed(e.to_string()))?;
        let sk = StaticSecret::from(secret);
        let pk = PublicKey::from(&sk);
        Ok((sk, pk.as_bytes().to_vec()))
    }

    pub(super) fn derive(
        my_private: StaticSecret,
        peer_public: &[u8],
        link_id: &str,
    ) -> Result<HeaderMacSession, HeaderMacError> {
        let peer_bytes: [u8; X25519_PUBLIC_KEY_LEN] = peer_public
            .try_into()
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        let peer = PublicKey::from(peer_bytes);
        let shared = my_private.diffie_hellman(&peer);

        let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(link_id.as_bytes()), shared.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(HKDF_INFO, &mut okm)
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        HeaderMacSession::new(&okm)
    }
}

/// Derive a [`HeaderMacSession`] from our ephemeral secret and the peer's public key.
#[cfg(not(target_arch = "wasm32"))]
pub fn derive_header_mac_from_ecdh(
    my_private: EphemeralKey,
    peer_public: &[u8],
    link_id: &str,
) -> Result<HeaderMacSession, HeaderMacError> {
    if peer_public.len() != X25519_PUBLIC_KEY_LEN {
        return Err(HeaderMacError::KeyAgreement);
    }
    let peer = UnparsedPublicKey::new(&agreement::X25519, peer_public);
    let mut okm = [0u8; 32];
    agree_ephemeral(my_private, peer, HeaderMacError::KeyAgreement, |secret| {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, link_id.as_bytes());
        let prk = salt.extract(secret);
        let out = prk
            .expand(&[HKDF_INFO], hkdf::HKDF_SHA256)
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        out.fill(&mut okm)
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        Ok(())
    })?;
    HeaderMacSession::new(&okm)
}

#[cfg(target_arch = "wasm32")]
pub fn derive_header_mac_from_ecdh(
    my_private: EphemeralKey,
    peer_public: &[u8],
    link_id: &str,
) -> Result<HeaderMacSession, HeaderMacError> {
    pure_ecdh::derive(my_private, peer_public, link_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::utils::DEFAULT_TTL;

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

    // Drives the browser ECDH+HKDF backend (`pure_ecdh`) on the native coverage
    // run: two peers negotiate over the pure-Rust path and derive a matching
    // link MAC key, so a header signed by one verifies with the other. The
    // public key wire length also matches the protocol constant.
    #[test]
    fn pure_ecdh_backend_peers_agree() {
        let link_id = uuid::Uuid::new_v4().to_string();
        let (init_sk, init_pk) = pure_ecdh::generate().unwrap();
        let (resp_sk, resp_pk) = pure_ecdh::generate().unwrap();
        assert_eq!(init_pk.len(), X25519_PUBLIC_KEY_LEN);
        assert_eq!(resp_pk.len(), X25519_PUBLIC_KEY_LEN);
        assert_ne!(init_pk, resp_pk, "distinct keypairs");

        let initiator = pure_ecdh::derive(init_sk, &resp_pk, &link_id).unwrap();
        let responder = pure_ecdh::derive(resp_sk, &init_pk, &link_id).unwrap();

        let mut h = empty_header();
        initiator.sign_slim_header(&mut h, &link_id).unwrap();
        responder.verify_slim_header(&h, &link_id).unwrap();
    }

    // A different link_id (HKDF salt) yields a different key, so a header signed
    // for one link must not verify on another even with the same peers.
    #[test]
    fn pure_ecdh_backend_key_is_bound_to_link_id() {
        let (init_sk, init_pk) = pure_ecdh::generate().unwrap();
        let (resp_sk, resp_pk) = pure_ecdh::generate().unwrap();

        let a = pure_ecdh::derive(init_sk, &resp_pk, "link-a").unwrap();
        let b = pure_ecdh::derive(resp_sk, &init_pk, "link-b").unwrap();

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
        let (sk, _pk) = pure_ecdh::generate().unwrap();
        let err = pure_ecdh::derive(sk, &[0u8; 8], "link").unwrap_err();
        assert!(matches!(err, HeaderMacError::KeyAgreement));
    }
}

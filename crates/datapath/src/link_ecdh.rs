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
    use x25519_dalek::{PublicKey, StaticSecret};

    let mut secret = [0u8; X25519_PUBLIC_KEY_LEN];
    getrandom::fill(&mut secret)
        .map_err(|e| HeaderMacError::KeyGenerationFailed(e.to_string()))?;
    let sk = StaticSecret::from(secret);
    let pk = PublicKey::from(&sk);
    Ok((sk, pk.as_bytes().to_vec()))
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
    use x25519_dalek::PublicKey;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::utils::DEFAULT_TTL;

    #[test]
    fn ecdh_hkdf_matches_between_initiator_and_responder() {
        let link_id = uuid::Uuid::new_v4().to_string();
        let (init_sk, init_pk) = generate_x25519_ephemeral().unwrap();
        let (resp_sk, resp_pk) = generate_x25519_ephemeral().unwrap();
        let a = derive_header_mac_from_ecdh(init_sk, resp_pk.as_slice(), &link_id).unwrap();
        let b = derive_header_mac_from_ecdh(resp_sk, init_pk.as_slice(), &link_id).unwrap();
        let lid = link_id.clone();
        let mut h = crate::api::proto::dataplane::v1::SlimHeader {
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
        };
        a.sign_slim_header(&mut h, &lid).unwrap();
        b.verify_slim_header(&h, &lid).unwrap();
    }
}

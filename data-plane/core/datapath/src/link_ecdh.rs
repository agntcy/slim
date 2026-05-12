// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! X25519 ECDH + HKDF-SHA256 for per-inter-node-link HMAC keys (`aws_lc_rs` only).

use std::sync::Arc;

use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
use aws_lc_rs::error::Unspecified;
use aws_lc_rs::hkdf;
use aws_lc_rs::rand::SystemRandom;

use crate::header_mac::{HeaderMacError, HeaderMacSession};

/// Wire encoding length for an X25519 public key in this protocol.
pub const X25519_PUBLIC_KEY_LEN: usize = 32;

const HKDF_INFO: &[u8] = b"SLIM-DP-inter-node-hmac-v1";

/// Generate an ephemeral X25519 keypair for link negotiation.
pub fn generate_x25519_ephemeral() -> Result<(EphemeralPrivateKey, Vec<u8>), Unspecified> {
    let rng = SystemRandom::new();
    let sk = EphemeralPrivateKey::generate(&agreement::X25519, &rng)?;
    let pk = sk.compute_public_key()?;
    Ok((sk, pk.as_ref().to_vec()))
}

/// Derive a [`HeaderMacSession`] from our ephemeral secret and the peer's public key.
pub fn derive_header_mac_from_ecdh(
    my_private: EphemeralPrivateKey,
    peer_public: &[u8],
    link_id: &str,
) -> Result<Arc<HeaderMacSession>, HeaderMacError> {
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
    Ok(Arc::new(HeaderMacSession::new(&okm)?))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            recv_from: None,
            forward_to: None,
            incoming_conn: None,
            error: None,
            header_mac: None,
        };
        a.sign_slim_header(&mut h, &lid).unwrap();
        b.verify_slim_header(&h, &lid).unwrap();
    }
}

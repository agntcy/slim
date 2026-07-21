// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Pure-Rust X25519 ECDH + HKDF-SHA256 backend (`x25519-dalek` + `hkdf` +
//! `sha2`), seeded from the workspace `getrandom` (0.3, `wasm_js` backend).
//!
//! This is the browser backend (`aws_lc_rs` is native-only). It is also compiled
//! into native **test** builds so the native coverage run instruments it and can
//! assert that two peers derive the same link MAC key — the property that lets a
//! native↔browser link interoperate.

use x25519_dalek::{PublicKey, StaticSecret};

use super::X25519_PUBLIC_KEY_LEN;
use crate::{
    header_mac::{HeaderMacError, HeaderMacSession},
    link_ecdh::HkdfInfo,
};

/// Opaque ephemeral private key, held between [`generate`] and [`derive`].
pub type EphemeralKey = StaticSecret;

pub fn generate() -> Result<(EphemeralKey, Vec<u8>), HeaderMacError> {
    let mut secret = [0u8; X25519_PUBLIC_KEY_LEN];
    getrandom::fill(&mut secret).map_err(|e| HeaderMacError::KeyGenerationFailed(e.to_string()))?;
    let sk = StaticSecret::from(secret);
    let pk = PublicKey::from(&sk);
    Ok((sk, pk.as_bytes().to_vec()))
}

pub fn derive(
    my_private: EphemeralKey,
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
    hk.expand(HkdfInfo::Classical.as_bytes(), &mut okm)
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    HeaderMacSession::new(&okm)
}

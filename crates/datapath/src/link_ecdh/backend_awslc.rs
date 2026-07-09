// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Native X25519 ECDH + HKDF-SHA256 backend using `aws_lc_rs`.
//!
//! Selected on every non-wasm target. Performs the same X25519 DH and
//! HKDF-SHA256 expansion as the pure-Rust [`super::backend_pure`] backend, so a
//! native↔browser link derives the same per-link MAC key.

use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
use aws_lc_rs::hkdf;
use aws_lc_rs::rand::SystemRandom;

use super::{HKDF_INFO, X25519_PUBLIC_KEY_LEN};
use crate::header_mac::{HeaderMacError, HeaderMacSession};

/// Opaque ephemeral private key, held between [`generate`] and [`derive`].
pub type EphemeralKey = EphemeralPrivateKey;

pub fn generate() -> Result<(EphemeralKey, Vec<u8>), HeaderMacError> {
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

pub fn derive(
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

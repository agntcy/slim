// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Native X25519 ECDH + HKDF-SHA256 backend using `aws_lc_rs`.
//!
//! Selected on every non-wasm target. Performs the same X25519 DH and
//! HKDF-SHA256 expansion as the pure-Rust [`super::backend_pure`] backend, so a
//! native↔browser link derives the same per-link MAC key.

use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
use aws_lc_rs::kem::{Ciphertext, DecapsulationKey, EncapsulationKey};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::{hkdf, kem};

use super::X25519_PUBLIC_KEY_LEN;
use crate::header_mac::{HeaderMacError, HeaderMacSession};
use crate::link_ecdh::HkdfInfo;

/// Opaque ephemeral private key, held between [`generate`] and [`derive`].
pub type EphemeralKey = EphemeralPrivateKey;

/// Opaque ML-KEM-768 decapsulation key, held between keygen and decapsulation.
pub type MlKem768SecretKey = DecapsulationKey;

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
        let info = HkdfInfo::Classical.as_bytes();
        let info_arr = [info];
        let out = prk
            .expand(&info_arr, hkdf::HKDF_SHA256)
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        out.fill(&mut okm)
            .map_err(|_| HeaderMacError::KeyAgreement)?;
        Ok(())
    })?;
    HeaderMacSession::new(&okm)
}

pub fn generate_mlkem768() -> Result<(DecapsulationKey, Vec<u8>), HeaderMacError> {
    let dk = DecapsulationKey::generate(&kem::ML_KEM_768)
        .map_err(|_| HeaderMacError::KeyGenerationFailed("ml-kem-768 keygen".into()))?;
    let ek = dk
        .encapsulation_key()
        .map_err(|_| HeaderMacError::KeyGenerationFailed("ml-kem-768 encapsulation key".into()))?;
    let pk = ek
        .key_bytes()
        .map_err(|_| HeaderMacError::KeyGenerationFailed("ml-kem-768 pk bytes".into()))?;
    Ok((dk, pk.as_ref().to_vec()))
}

pub fn encapsulate_mlkem768(pk: &[u8]) -> Result<(Vec<u8>, [u8; 32]), HeaderMacError> {
    let ek =
        EncapsulationKey::new(&kem::ML_KEM_768, pk).map_err(|_| HeaderMacError::KeyAgreement)?;
    let (ct, ss) = ek.encapsulate().map_err(|_| HeaderMacError::KeyAgreement)?;
    let mut shared = [0u8; 32];
    shared.copy_from_slice(ss.as_ref());
    Ok((ct.as_ref().to_vec(), shared))
}

pub fn decapsulate_mlkem768(dk: DecapsulationKey, ct: &[u8]) -> Result<[u8; 32], HeaderMacError> {
    let ss = dk
        .decapsulate(Ciphertext::from(ct))
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    let mut shared = [0u8; 32];
    shared.copy_from_slice(ss.as_ref());
    Ok(shared)
}

pub fn derive_hybrid(
    my_private: EphemeralKey,
    peer_public: &[u8],
    mlkem_shared: &[u8; 32],
    link_id: &str,
) -> Result<HeaderMacSession, HeaderMacError> {
    if peer_public.len() != X25519_PUBLIC_KEY_LEN {
        return Err(HeaderMacError::KeyAgreement);
    }
    let peer = UnparsedPublicKey::new(&agreement::X25519, peer_public);
    let mut okm = [0u8; 32];
    agree_ephemeral(
        my_private,
        peer,
        HeaderMacError::KeyAgreement,
        |x25519_shared| {
            let mut ikm = [0u8; 64];
            ikm[..32].copy_from_slice(x25519_shared);
            ikm[32..].copy_from_slice(mlkem_shared);
            let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, link_id.as_bytes());
            let prk = salt.extract(&ikm);
            let info = HkdfInfo::PostQuantum.as_bytes();
            let info_arr = [info];
            let out = prk
                .expand(&info_arr, hkdf::HKDF_SHA256)
                .map_err(|_| HeaderMacError::KeyAgreement)?;
            out.fill(&mut okm)
                .map_err(|_| HeaderMacError::KeyAgreement)?;
            Ok(())
        },
    )?;
    HeaderMacSession::new(&okm)
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Pure-Rust X25519 ECDH + HKDF-SHA256 backend (`x25519-dalek` + `hkdf` +
//! `sha2`), seeded from the workspace `getrandom` (0.3, `wasm_js` backend).
//!
//! ML-KEM-768 hybrid link keys use the `ml-kem` crate (FIPS 203). This is the
//! browser backend (`aws_lc_rs` is native-only). It is also compiled into native
//! **test** builds so the native coverage run instruments it and can assert that
//! two peers derive the same link MAC key — the property that lets a
//! native↔browser link interoperate.

use core::convert::Infallible;

use ml_kem::kem::{Decapsulate, Encapsulate, Generate, KeyExport, TryKeyInit};
use ml_kem::ml_kem_768::{DecapsulationKey, EncapsulationKey};
use rand_core::{TryCryptoRng, TryRng};
use x25519_dalek::{PublicKey, StaticSecret};

use super::{ML_KEM768_CIPHERTEXT_LEN, ML_KEM768_PUBLIC_KEY_LEN, X25519_PUBLIC_KEY_LEN};
use crate::{
    header_mac::{HeaderMacError, HeaderMacSession},
    link_ecdh::HkdfInfo,
};

/// Seeds ML-KEM via the workspace `getrandom` (0.3, `wasm_js` on wasm32).
///
/// The ml_kem API requires `TryCryptoRng<Error = Infallible>` (via the `CryptoRng`
/// blanket impl), so we cannot surface a getrandom failure through the trait.
/// Instead we store it here and callers check `self.error` after the ml_kem call.
#[derive(Default)]
struct GetrandomRng {
    error: Option<getrandom::Error>,
}

// `TryRng` requires `Result<_, Infallible>`, so these methods always return `Ok`.
// Callers must check [`GetrandomRng::error`] after any `ml_kem` call that used this RNG.
impl TryRng for GetrandomRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut buf = [0u8; 4];
        self.try_fill_bytes(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut buf = [0u8; 8];
        self.try_fill_bytes(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    /// Always returns `Ok(())`; on `getrandom` failure sets [`Self::error`] instead.
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        if let Err(e) = getrandom::fill(dest) {
            self.error = Some(e);
        }
        Ok(())
    }
}

impl TryCryptoRng for GetrandomRng {}

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
    hk.expand(HkdfInfo::Classical.to_bytes(), &mut okm)
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    HeaderMacSession::new(&okm)
}

pub fn generate_mlkem768() -> Result<(DecapsulationKey, Vec<u8>), HeaderMacError> {
    let mut rng = GetrandomRng::default();
    let dk = DecapsulationKey::generate_from_rng(&mut rng);
    if let Some(e) = rng.error {
        return Err(HeaderMacError::KeyGenerationFailed(e.to_string()));
    }
    let pk = dk.encapsulation_key().to_bytes();
    if pk.len() != ML_KEM768_PUBLIC_KEY_LEN {
        return Err(HeaderMacError::KeyGenerationFailed(
            "ml-kem-768 pk length mismatch".into(),
        ));
    }
    Ok((dk, pk.to_vec()))
}

pub fn encapsulate_mlkem768(pk: &[u8]) -> Result<(Vec<u8>, [u8; 32]), HeaderMacError> {
    if pk.len() != ML_KEM768_PUBLIC_KEY_LEN {
        return Err(HeaderMacError::KeyAgreement);
    }
    let ek = EncapsulationKey::new_from_slice(pk).map_err(|_| HeaderMacError::KeyAgreement)?;
    let mut rng = GetrandomRng::default();
    let (ct, shared) = ek.encapsulate_with_rng(&mut rng);
    if let Some(e) = rng.error {
        return Err(HeaderMacError::KeyGenerationFailed(e.to_string()));
    }
    if ct.len() != ML_KEM768_CIPHERTEXT_LEN {
        return Err(HeaderMacError::KeyAgreement);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(shared.as_slice());
    Ok((ct.to_vec(), out))
}

pub fn decapsulate_mlkem768(dk: DecapsulationKey, ct: &[u8]) -> Result<[u8; 32], HeaderMacError> {
    if ct.len() != ML_KEM768_CIPHERTEXT_LEN {
        return Err(HeaderMacError::KeyAgreement);
    }
    let shared = dk
        .decapsulate_slice(ct)
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(shared.as_slice());
    Ok(out)
}

pub fn derive_hybrid(
    my_private: EphemeralKey,
    peer_public: &[u8],
    mlkem_shared: &[u8; 32],
    link_id: &str,
) -> Result<HeaderMacSession, HeaderMacError> {
    let peer_bytes: [u8; X25519_PUBLIC_KEY_LEN] = peer_public
        .try_into()
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    let peer = PublicKey::from(peer_bytes);
    let x25519_shared = my_private.diffie_hellman(&peer);

    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(x25519_shared.as_bytes());
    ikm[32..].copy_from_slice(mlkem_shared);

    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(link_id.as_bytes()), &ikm);
    let mut okm = [0u8; 32];
    hk.expand(HkdfInfo::PostQuantum.to_bytes(), &mut okm)
        .map_err(|_| HeaderMacError::KeyAgreement)?;
    HeaderMacSession::new(&okm)
}

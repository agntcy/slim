// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Value-level authenticated encryption for the at-rest store.
//!
//! The database engine is plain (unencrypted) SQLite; confidentiality and
//! integrity of the sensitive blobs (MLS group snapshots/epoch secrets and
//! session records) are provided here with **AES-256-GCM via `aws-lc-rs`** —
//! the same crypto stack the MLS layer already uses, so no OpenSSL/SQLCipher is
//! linked. Keys (`session:<id>`, group ids) are stored in plaintext; they are
//! not secret.
//!
//! Each ciphertext is `nonce (12 bytes) || AES-256-GCM(ciphertext||tag)`. Nonces
//! are generated randomly per encryption by `aws-lc-rs`.

use std::sync::Arc;

use aws_lc_rs::aead::{AES_256_GCM, Aad, NONCE_LEN, Nonce, RandomizedNonceKey};
use aws_lc_rs::hkdf::{HKDF_SHA256, KeyType, Salt};

use crate::errors::PersistenceError;
use crate::group_storage::MlsEncryptionKey;

// Domain-separated, fixed HKDF salt/info so a passphrase/identity always derives
// the same key across restarts.
const HKDF_SALT: &[u8] = b"slim-persistence-hkdf-salt-v1";
const HKDF_INFO: &[u8] = b"slim-persistence-aes-256-gcm-value-key-v1";

struct Aes256KeyLen;
impl KeyType for Aes256KeyLen {
    fn len(&self) -> usize {
        32
    }
}

/// AES-256-GCM cipher for individual stored values.
pub(crate) struct ValueCipher {
    key: RandomizedNonceKey,
}

impl ValueCipher {
    /// Derive a cipher from the configured key, or from the endpoint identity
    /// when no explicit key is given.
    pub(crate) fn derive(
        key: Option<MlsEncryptionKey>,
        identity: &str,
    ) -> Result<Arc<Self>, PersistenceError> {
        let key_bytes: [u8; 32] = match key {
            Some(MlsEncryptionKey::RawKey(k)) => k,
            Some(MlsEncryptionKey::Passphrase(p)) => derive_key(p.as_bytes())?,
            None => derive_key(format!("slim-mls-v1:{identity}").as_bytes())?,
        };

        let key = RandomizedNonceKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|_| PersistenceError::Storage("failed to initialize AEAD key".into()))?;

        Ok(Arc::new(Self { key }))
    }

    /// Encrypt `plaintext` into `nonce || ciphertext||tag`.
    pub(crate) fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, PersistenceError> {
        let mut in_out = plaintext.to_vec();
        let nonce = self
            .key
            .seal_in_place_append_tag(Aad::empty(), &mut in_out)
            .map_err(|_| PersistenceError::Storage("encryption failed".into()))?;

        let nonce_bytes: &[u8; NONCE_LEN] = nonce.as_ref();
        let mut out = Vec::with_capacity(NONCE_LEN + in_out.len());
        out.extend_from_slice(nonce_bytes);
        out.extend_from_slice(&in_out);
        Ok(out)
    }

    /// Decrypt a `nonce || ciphertext||tag` blob. Fails on a wrong key or
    /// tampered/corrupt data (AEAD authentication).
    pub(crate) fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>, PersistenceError> {
        if blob.len() < NONCE_LEN {
            return Err(PersistenceError::Storage("ciphertext too short".into()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| PersistenceError::Storage("invalid nonce".into()))?;

        let mut in_out = ciphertext.to_vec();
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| {
                PersistenceError::Storage("decryption failed (wrong key or corrupt data)".into())
            })?;
        Ok(plaintext.to_vec())
    }
}

fn derive_key(ikm: &[u8]) -> Result<[u8; 32], PersistenceError> {
    let salt = Salt::new(HKDF_SHA256, HKDF_SALT);
    let prk = salt.extract(ikm);
    let okm = prk
        .expand(&[HKDF_INFO], Aes256KeyLen)
        .map_err(|_| PersistenceError::Storage("key derivation failed".into()))?;
    let mut key = [0u8; 32];
    okm.fill(&mut key)
        .map_err(|_| PersistenceError::Storage("key derivation failed".into()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper_detection() {
        let cipher = ValueCipher::derive(None, "alice").unwrap();
        let ct = cipher.encrypt(b"secret payload").unwrap();
        assert_ne!(&ct[NONCE_LEN..], b"secret payload");
        assert_eq!(cipher.decrypt(&ct).unwrap(), b"secret payload");

        // Flipping a byte fails authentication.
        let mut tampered = ct.clone();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(cipher.decrypt(&tampered).is_err());
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let a =
            ValueCipher::derive(Some(MlsEncryptionKey::Passphrase("aaa".into())), "id").unwrap();
        let b =
            ValueCipher::derive(Some(MlsEncryptionKey::Passphrase("bbb".into())), "id").unwrap();
        let ct = a.encrypt(b"data").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }

    #[test]
    fn derivation_is_deterministic() {
        let a = ValueCipher::derive(None, "alice").unwrap();
        let b = ValueCipher::derive(None, "alice").unwrap();
        // A value encrypted under one must decrypt under the other (same key).
        let ct = a.encrypt(b"x").unwrap();
        assert_eq!(b.decrypt(&ct).unwrap(), b"x");
    }
}

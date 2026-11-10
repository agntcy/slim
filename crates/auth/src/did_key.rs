// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `did:key` identifiers for Ed25519 keys: `did:key:z` followed by the
//! base58btc encoding of the Ed25519 public key multicodec prefix
//! (`0xed 0x01`) and the 32 key bytes.

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine;

use crate::errors::AuthError;

const ED25519_PUB_MULTICODEC: [u8; 2] = [0xed, 0x01];

/// The `did:key` of an Ed25519 public key.
pub fn ed25519_did_key(public_key: &[u8]) -> String {
    let mut bytes = ED25519_PUB_MULTICODEC.to_vec();
    bytes.extend_from_slice(public_key);
    format!("did:key:z{}", bs58::encode(bytes).into_string())
}

/// The Ed25519 public key encoded by a `did:key:z...` identifier.
pub fn ed25519_public_key_from_did_key(did: &str) -> Result<[u8; 32], AuthError> {
    let unsupported = || AuthError::InvalidDidKey(did.to_string());

    let multibase_value = did.strip_prefix("did:key:").ok_or_else(unsupported)?;
    // 'z' is the multibase prefix for base58btc; did:key only ever uses it.
    let base58_value = multibase_value.strip_prefix('z').ok_or_else(unsupported)?;
    let decoded = bs58::decode(base58_value)
        .into_vec()
        .map_err(|_| unsupported())?;

    match decoded.as_slice() {
        [0xed, 0x01, key @ ..] if key.len() == 32 => {
            let mut pubkey = [0u8; 32];
            pubkey.copy_from_slice(key);
            Ok(pubkey)
        }
        _ => Err(unsupported()),
    }
}

/// The `did:key` of the Ed25519 key in a PKCS#8 PEM document
/// (`-----BEGIN PRIVATE KEY-----`).
pub fn ed25519_did_key_from_pkcs8_pem(pem: &str) -> Result<String, AuthError> {
    Ok(ed25519_did_key(&ed25519_public_key_from_pkcs8_pem(pem)?))
}

/// The public key of the Ed25519 key in a PKCS#8 PEM document.
pub fn ed25519_public_key_from_pkcs8_pem(pem: &str) -> Result<Vec<u8>, AuthError> {
    let key_pair = ed25519_key_pair_from_pkcs8_pem(pem)?;
    Ok(key_pair.public_key().as_ref().to_vec())
}

/// The Ed25519 key pair in a PKCS#8 PEM document, for signing.
pub fn ed25519_key_pair_from_pkcs8_pem(pem: &str) -> Result<Ed25519KeyPair, AuthError> {
    let der = pem_contents(pem, "PRIVATE KEY")?;
    Ed25519KeyPair::from_pkcs8(&der).map_err(|e| AuthError::InvalidEd25519Key(e.to_string()))
}

/// MLS signature keys `(secret, public)` for the Ed25519 key in a PKCS#8 PEM
/// document, in the encoding the Curve25519 MLS ciphersuite uses (AWS-LC:
/// the secret key is the 32-byte seed followed by the 32-byte public key).
pub fn ed25519_mls_signature_keys_from_pkcs8_pem(
    pem: &str,
) -> Result<(Vec<u8>, Vec<u8>), AuthError> {
    use aws_lc_rs::encoding::AsBigEndian;

    let key_pair = ed25519_key_pair_from_pkcs8_pem(pem)?;
    let public = key_pair.public_key().as_ref().to_vec();
    let seed = key_pair
        .seed()
        .map_err(|e| AuthError::InvalidEd25519Key(e.to_string()))?;
    let seed = seed
        .as_be_bytes()
        .map_err(|e| AuthError::InvalidEd25519Key(e.to_string()))?;
    let mut secret = seed.as_ref().to_vec();
    secret.extend_from_slice(&public);
    Ok((secret, public))
}

/// The DER bytes of the first `label` block in `pem`.
fn pem_contents(pem: &str, label: &str) -> Result<Vec<u8>, AuthError> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let missing = || AuthError::InvalidEd25519Key(format!("no {begin} block"));
    let start = pem.find(&begin).ok_or_else(missing)? + begin.len();
    let stop = start + pem[start..].find(&end).ok_or_else(missing)?;
    let base64: String = pem[start..stop].split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| AuthError::InvalidEd25519Key(e.to_string()))
}

/// A fresh Ed25519 identity for tests: its PKCS#8 PEM, its `did:key`, and its
/// raw public key bytes.
#[cfg(all(test, feature = "insecure-self-certified-did"))]
pub(crate) fn fresh_identity() -> (String, String, Vec<u8>) {
    use aws_lc_rs::rand::SystemRandom;
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let public = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .unwrap()
        .public_key()
        .as_ref()
        .to_vec();
    let pem = crate::utils::bytes_to_pem(
        pkcs8.as_ref(),
        "-----BEGIN PRIVATE KEY-----\n",
        "\n-----END PRIVATE KEY-----\n",
    );
    (pem, ed25519_did_key(&public), public)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::bytes_to_pem;

    /// RFC 8032 section 7.1, test 1.
    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// PKCS#8 v1 (seed only, as `openssl genpkey -algorithm ed25519` writes).
    fn pkcs8_pem(seed: &[u8]) -> String {
        let mut der = hex("302e020100300506032b657004220420");
        der.extend_from_slice(seed);
        bytes_to_pem(
            &der,
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----\n",
        )
    }

    #[test]
    fn did_key_of_a_public_key() {
        let did = ed25519_did_key(&hex(PUBLIC));
        let decoded = bs58::decode(did.strip_prefix("did:key:z").unwrap())
            .into_vec()
            .unwrap();
        assert_eq!(&decoded[..2], &[0xed, 0x01]);
        assert_eq!(&decoded[2..], hex(PUBLIC).as_slice());
    }

    #[test]
    fn decoding_a_did_key_recovers_the_public_key() {
        let did = ed25519_did_key(&hex(PUBLIC));
        assert_eq!(
            ed25519_public_key_from_did_key(&did).unwrap(),
            hex(PUBLIC).as_slice()
        );
    }

    #[test]
    fn rejects_a_did_key_without_the_did_key_prefix() {
        assert!(matches!(
            ed25519_public_key_from_did_key("z6MkhaXg"),
            Err(AuthError::InvalidDidKey(_))
        ));
    }

    #[test]
    fn rejects_a_did_key_without_the_multibase_prefix() {
        assert!(matches!(
            ed25519_public_key_from_did_key("did:key:6MkhaXg"),
            Err(AuthError::InvalidDidKey(_))
        ));
    }

    #[test]
    fn rejects_a_did_key_with_invalid_base58() {
        assert!(matches!(
            ed25519_public_key_from_did_key("did:key:z0OIl"),
            Err(AuthError::InvalidDidKey(_))
        ));
    }

    #[test]
    fn rejects_a_did_key_with_the_wrong_multicodec_prefix() {
        let mut bytes = vec![0x00, 0x00];
        bytes.extend_from_slice(&hex(PUBLIC));
        let did = format!("did:key:z{}", bs58::encode(bytes).into_string());
        assert!(matches!(
            ed25519_public_key_from_did_key(&did),
            Err(AuthError::InvalidDidKey(_))
        ));
    }

    #[test]
    fn rejects_a_did_key_with_the_wrong_key_length() {
        let mut bytes = vec![0xed, 0x01];
        bytes.extend_from_slice(&hex(PUBLIC)[..16]);
        let did = format!("did:key:z{}", bs58::encode(bytes).into_string());
        assert!(matches!(
            ed25519_public_key_from_did_key(&did),
            Err(AuthError::InvalidDidKey(_))
        ));
    }

    #[test]
    fn did_key_from_a_pkcs8_pem_matches_its_public_key() {
        assert_eq!(
            ed25519_did_key_from_pkcs8_pem(&pkcs8_pem(&hex(SEED))).unwrap(),
            ed25519_did_key(&hex(PUBLIC))
        );
    }

    #[test]
    fn rejects_a_pem_without_a_private_key_block() {
        let pem = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
        assert!(matches!(
            ed25519_did_key_from_pkcs8_pem(pem),
            Err(AuthError::InvalidEd25519Key(_))
        ));
    }

    #[test]
    fn rejects_a_key_that_is_not_ed25519() {
        // A PKCS#8 header with garbage key material.
        let pem = bytes_to_pem(
            &[0x30, 0x03, 0x02, 0x01, 0x00],
            "-----BEGIN PRIVATE KEY-----\n",
            "\n-----END PRIVATE KEY-----\n",
        );
        assert!(matches!(
            ed25519_did_key_from_pkcs8_pem(&pem),
            Err(AuthError::InvalidEd25519Key(_))
        ));
    }

    #[test]
    fn mls_signature_keys_are_seed_then_public_and_sign() {
        let (secret, public) =
            ed25519_mls_signature_keys_from_pkcs8_pem(&pkcs8_pem(&hex(SEED))).unwrap();
        assert_eq!(public, hex(PUBLIC));
        assert_eq!(secret, [hex(SEED), hex(PUBLIC)].concat());
        let sig = crate::utils::sign_header_aad(b"aad", &secret, &public).unwrap();
        crate::utils::verify_header_aad(b"aad", &sig, &public).unwrap();
    }
}

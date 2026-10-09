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

/// The `did:key` of the Ed25519 key in a PKCS#8 PEM document
/// (`-----BEGIN PRIVATE KEY-----`).
pub fn ed25519_did_key_from_pkcs8_pem(pem: &str) -> Result<String, AuthError> {
    let der = pem_contents(pem, "PRIVATE KEY")?;
    let key_pair = Ed25519KeyPair::from_pkcs8(&der)
        .map_err(|e| AuthError::InvalidEd25519Key(e.to_string()))?;
    Ok(ed25519_did_key(key_pair.public_key().as_ref()))
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
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The keys of a `did:key` identity, as raw bytes or base64 text.
//!
//! [`Service::create_app_with_did_key`](crate::Service) takes the app's private key and the
//! public keys it trusts as [`KeyBytes`], and converts them to what the identity machinery
//! uses: a PKCS#8 PEM document for the private key, a JWKS for the trust anchor.
//!
//! The identity is Ed25519: that is the key type `did:key` is built on in `slim-auth`
//! (the tokens are signed with `EdDSA`), so the functions take no key type.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use slim_auth::did_key::{ed25519_did_key, ed25519_public_key_from_pkcs8_pem};

use crate::errors::ServiceError;

/// The bytes of a PKCS#8 v1 document for an Ed25519 key, up to the 32 bytes of its seed.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

const ED25519_KEY_LEN: usize = 32;

/// Key material: the bytes themselves, or their base64 text (standard or URL-safe
/// alphabet, with or without padding).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyBytes {
    Raw(Vec<u8>),
    Base64(String),
}

impl KeyBytes {
    /// The bytes of the key.
    pub fn decode(&self) -> Result<Vec<u8>, ServiceError> {
        match self {
            KeyBytes::Raw(bytes) => Ok(bytes.clone()),
            KeyBytes::Base64(text) => {
                let text = text.trim();
                [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
                    .iter()
                    .find_map(|alphabet| alphabet.decode(text).ok())
                    .ok_or_else(|| invalid("a key is not valid base64"))
            }
        }
    }
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::InvalidConfig(format!("invalid did:key identity: {message}"))
}

/// The private key as a PKCS#8 PEM document. `key` is the 32-byte Ed25519 seed (RFC 8032),
/// or a PKCS#8 document (DER), such as `openssl genpkey -algorithm ed25519 -outform DER`
/// writes.
pub(crate) fn private_key_pem(key: &KeyBytes) -> Result<String, ServiceError> {
    let bytes = key.decode()?;
    let der = match bytes.len() {
        ED25519_KEY_LEN => [PKCS8_ED25519_PREFIX.as_slice(), &bytes].concat(),
        n if n == PKCS8_ED25519_PREFIX.len() + ED25519_KEY_LEN
            && bytes.starts_with(&PKCS8_ED25519_PREFIX) =>
        {
            bytes
        }
        n => {
            return Err(invalid(&format!(
                "the private key has {n} bytes: expected a 32-byte Ed25519 seed or a \
                 48-byte PKCS#8 document"
            )));
        }
    };
    Ok(slim_auth::utils::bytes_to_pem(
        &der,
        "-----BEGIN PRIVATE KEY-----\n",
        "\n-----END PRIVATE KEY-----\n",
    ))
}

/// The `did:key` of the private key.
pub fn did_key_of_private_key(key: &KeyBytes) -> Result<String, ServiceError> {
    let public_key = ed25519_public_key_from_pkcs8_pem(&private_key_pem(key)?)?;
    Ok(ed25519_did_key(&public_key))
}

/// The JWKS that holds the public keys in `trusted` and the public key of `own_pem`: the
/// app must trust its own key, since MLS verifies the app's own token against it.
pub(crate) fn trusted_keys_jwks(
    trusted: &[KeyBytes],
    own_pem: &str,
) -> Result<String, ServiceError> {
    let mut public_keys: Vec<Vec<u8>> = vec![ed25519_public_key_from_pkcs8_pem(own_pem)?];
    for key in trusted {
        let bytes = key.decode()?;
        if bytes.len() != ED25519_KEY_LEN {
            return Err(invalid(&format!(
                "a trusted key has {} bytes: expected a 32-byte Ed25519 public key",
                bytes.len()
            )));
        }
        if !public_keys.contains(&bytes) {
            public_keys.push(bytes);
        }
    }

    let keys: Vec<String> = public_keys
        .iter()
        .map(|key| {
            format!(
                r#"{{"kty":"OKP","crv":"Ed25519","alg":"EdDSA","use":"sig","x":"{}"}}"#,
                URL_SAFE_NO_PAD.encode(key)
            )
        })
        .collect();
    Ok(format!(r#"{{"keys":[{}]}}"#, keys.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8032 section 7.1, test 1.
    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    // RFC 8032 section 7.1, test 2.
    const PUBLIC_2: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn base64_in_any_alphabet_decodes() {
        let bytes = hex(PUBLIC);
        for text in [
            STANDARD.encode(&bytes),
            STANDARD_NO_PAD.encode(&bytes),
            URL_SAFE.encode(&bytes),
            URL_SAFE_NO_PAD.encode(&bytes),
            format!(" {} \n", STANDARD.encode(&bytes)),
        ] {
            assert_eq!(KeyBytes::Base64(text).decode().unwrap(), bytes);
        }
        assert!(KeyBytes::Base64("not base64!".into()).decode().is_err());
    }

    #[test]
    fn a_seed_and_a_pkcs8_document_give_the_same_key() {
        let seed = KeyBytes::Raw(hex(SEED));
        let pkcs8 = KeyBytes::Raw([PKCS8_ED25519_PREFIX.as_slice(), &hex(SEED)].concat());
        assert_eq!(
            private_key_pem(&seed).unwrap(),
            private_key_pem(&pkcs8).unwrap()
        );
        assert_eq!(
            did_key_of_private_key(&seed).unwrap(),
            ed25519_did_key(&hex(PUBLIC))
        );
        // Base64 of the seed is the same key too.
        assert_eq!(
            did_key_of_private_key(&KeyBytes::Base64(STANDARD.encode(hex(SEED)))).unwrap(),
            ed25519_did_key(&hex(PUBLIC))
        );
    }

    #[test]
    fn a_private_key_of_another_size_is_rejected() {
        for len in [0, 31, 33, 64] {
            let err = private_key_pem(&KeyBytes::Raw(vec![1; len])).unwrap_err();
            assert!(
                err.to_string().contains("invalid did:key identity"),
                "{err}"
            );
        }
        // 48 bytes, but not an Ed25519 PKCS#8 header.
        assert!(private_key_pem(&KeyBytes::Raw(vec![7; 48])).is_err());
    }

    #[test]
    fn the_trust_anchor_holds_the_own_key_and_the_trusted_ones_once() {
        let own = private_key_pem(&KeyBytes::Raw(hex(SEED))).unwrap();
        // The own key is listed too: it is not duplicated.
        let jwks = trusted_keys_jwks(
            &[
                KeyBytes::Raw(hex(PUBLIC_2)),
                KeyBytes::Base64(STANDARD.encode(hex(PUBLIC))),
            ],
            &own,
        )
        .unwrap();
        assert_eq!(jwks.matches("\"kty\":\"OKP\"").count(), 2);
        assert!(jwks.contains(&URL_SAFE_NO_PAD.encode(hex(PUBLIC))));
        assert!(jwks.contains(&URL_SAFE_NO_PAD.encode(hex(PUBLIC_2))));
        // With nothing else trusted, the app trusts itself.
        let only_own = trusted_keys_jwks(&[], &own).unwrap();
        assert_eq!(only_own.matches("\"kty\":\"OKP\"").count(), 1);
    }

    #[test]
    fn a_trusted_key_of_another_size_is_rejected() {
        let own = private_key_pem(&KeyBytes::Raw(hex(SEED))).unwrap();
        assert!(trusted_keys_jwks(&[KeyBytes::Raw(vec![1; 31])], &own).is_err());
    }
}

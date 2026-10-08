// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Signed grants: proof that a channel's owner authorized a specific
//! participant change.
//!
//! Verification is pluggable (`GrantVerifier`) so a deployment can swap in
//! its own grant format and key scheme -- e.g. one that resolves an agent's
//! binding certificate back to its human owner -- without this crate
//! depending on that format. [`DidKeyEd25519Verifier`] is the default: it
//! understands an owner identifier that is a `did:key` encoding an Ed25519
//! public key (self-certifying -- no registry lookup needed) and a grant
//! that is this module's own `SignedGrant` JSON envelope.

use std::fmt;

use aws_lc_rs::signature::{self, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// The parsed, authenticated contents of a grant: what the owner actually
/// authorized. Verifying a grant proves it was signed by the channel's
/// owner -- it does not, on its own, prove it applies to the request at
/// hand. Callers must still check `channel`/`invitee`/`not_after` against
/// the request they received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub channel: String,
    pub invitee: String,
    pub role: String,
    /// Unix seconds after which the grant is no longer valid.
    pub not_after: u64,
    /// Opaque replay-protection token. Not enforced here -- a nonce store
    /// tracking which values have already been consumed is separate work.
    pub nonce: String,
}

/// Why a grant failed to verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// The owner identifier isn't in a format this verifier understands.
    UnsupportedOwnerFormat(String),
    /// The grant bytes aren't in a format this verifier understands.
    MalformedGrant(String),
    /// The grant parsed, but its signature doesn't check out against the
    /// owner's key.
    InvalidSignature,
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::UnsupportedOwnerFormat(owner) => {
                write!(f, "unsupported owner identifier format: {owner}")
            }
            GrantError::MalformedGrant(reason) => write!(f, "malformed grant: {reason}"),
            GrantError::InvalidSignature => write!(f, "invalid grant signature"),
        }
    }
}

impl std::error::Error for GrantError {}

/// Authenticates a grant and parses what it authorizes.
///
/// A deployment supplies its own implementation to `ChannelManagerServer`
/// (see `ChannelManagerServer::new`) to use a different grant format or key
/// scheme than the default. Implementations only authenticate and parse --
/// checking the parsed `Grant` against the live request (channel, invitee,
/// expiry) is the caller's job, so every verifier gets that enforcement
/// uniformly rather than having to implement it itself.
pub trait GrantVerifier: Send + Sync {
    /// Verifies `grant` was signed by `owner`, returning its parsed
    /// contents on success.
    fn verify(&self, owner: &str, grant: &[u8]) -> Result<Grant, GrantError>;
}

/// The wire format [`DidKeyEd25519Verifier`] expects in a request's `grant`
/// field: the grant's fields alongside a detached signature over their
/// canonical encoding (see `canonical_bytes`).
#[derive(Debug, Serialize, Deserialize)]
struct SignedGrant {
    channel: String,
    invitee: String,
    role: String,
    not_after: u64,
    nonce: String,
    /// Base64-encoded (standard, padded) Ed25519 signature.
    signature: String,
}

/// Byte representation a grant's fields are signed over. NUL-separated:
/// none of the fields are expected to contain NUL bytes (they're protocol
/// names, a role label, a timestamp and a nonce), so this needs no escaping
/// and no field-order ambiguity, unlike signing re-serialized JSON would.
fn canonical_bytes(
    channel: &str,
    invitee: &str,
    role: &str,
    not_after: u64,
    nonce: &str,
) -> Vec<u8> {
    [channel, invitee, role, &not_after.to_string(), nonce]
        .join("\0")
        .into_bytes()
}

/// Decodes a `did:key` identifier that encodes an Ed25519 public key.
///
/// Format: `did:key:z` followed by the base58btc (bitcoin alphabet)
/// encoding of the multicodec-prefixed key -- the 2-byte prefix `0xed 0x01`
/// (Ed25519 public key) followed by the 32 raw key bytes.
fn decode_ed25519_did_key(owner: &str) -> Result<[u8; 32], GrantError> {
    let unsupported = || GrantError::UnsupportedOwnerFormat(owner.to_string());

    let multibase_value = owner.strip_prefix("did:key:").ok_or_else(unsupported)?;
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

/// Default [`GrantVerifier`]: owner is a `did:key`-encoded Ed25519 public
/// key, grant is a [`SignedGrant`] JSON envelope signed with that key.
#[derive(Debug, Default, Clone, Copy)]
pub struct DidKeyEd25519Verifier;

impl GrantVerifier for DidKeyEd25519Verifier {
    fn verify(&self, owner: &str, grant: &[u8]) -> Result<Grant, GrantError> {
        let pubkey_bytes = decode_ed25519_did_key(owner)?;

        let signed: SignedGrant =
            serde_json::from_slice(grant).map_err(|e| GrantError::MalformedGrant(e.to_string()))?;
        let signature_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &signed.signature,
        )
        .map_err(|e| GrantError::MalformedGrant(format!("signature: {e}")))?;

        let payload = canonical_bytes(
            &signed.channel,
            &signed.invitee,
            &signed.role,
            signed.not_after,
            &signed.nonce,
        );

        UnparsedPublicKey::new(&signature::ED25519, &pubkey_bytes[..])
            .verify(&payload, &signature_bytes)
            .map_err(|_| GrantError::InvalidSignature)?;

        Ok(Grant {
            channel: signed.channel,
            invitee: signed.invitee,
            role: signed.role,
            not_after: signed.not_after,
            nonce: signed.nonce,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

    /// Generates a fresh Ed25519 keypair and its did:key identifier.
    fn new_owner() -> (Ed25519KeyPair, String) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();

        let mut multicodec = vec![0xed, 0x01];
        multicodec.extend_from_slice(key_pair.public_key().as_ref());
        let did_key = format!("did:key:z{}", bs58::encode(multicodec).into_string());

        (key_pair, did_key)
    }

    fn sign_grant(
        key_pair: &Ed25519KeyPair,
        channel: &str,
        invitee: &str,
        role: &str,
        not_after: u64,
        nonce: &str,
    ) -> Vec<u8> {
        let payload = canonical_bytes(channel, invitee, role, not_after, nonce);
        let signature = key_pair.sign(&payload);
        let signed = SignedGrant {
            channel: channel.to_string(),
            invitee: invitee.to_string(),
            role: role.to_string(),
            not_after,
            nonce: nonce.to_string(),
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signature.as_ref(),
            ),
        };
        serde_json::to_vec(&signed).unwrap()
    }

    #[test]
    fn verifies_a_correctly_signed_grant() {
        let (key_pair, owner) = new_owner();
        let grant = sign_grant(
            &key_pair,
            "org/ns/ch1",
            "org/ns/agent1",
            "member",
            9_999_999_999,
            "nonce-1",
        );

        let parsed = DidKeyEd25519Verifier.verify(&owner, &grant).unwrap();
        assert_eq!(
            parsed,
            Grant {
                channel: "org/ns/ch1".to_string(),
                invitee: "org/ns/agent1".to_string(),
                role: "member".to_string(),
                not_after: 9_999_999_999,
                nonce: "nonce-1".to_string(),
            }
        );
    }

    #[test]
    fn rejects_a_grant_signed_by_a_different_key() {
        let (_signer_key, _signer_owner) = new_owner();
        let (signer_key, _) = new_owner();
        let (_, claimed_owner) = new_owner();
        let grant = sign_grant(
            &signer_key,
            "org/ns/ch1",
            "org/ns/agent1",
            "member",
            9_999_999_999,
            "n",
        );

        assert_eq!(
            DidKeyEd25519Verifier.verify(&claimed_owner, &grant),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_tampered_field() {
        let (key_pair, owner) = new_owner();
        let grant = sign_grant(
            &key_pair,
            "org/ns/ch1",
            "org/ns/agent1",
            "member",
            9_999_999_999,
            "n",
        );
        let mut tampered: SignedGrant = serde_json::from_slice(&grant).unwrap();
        tampered.invitee = "org/ns/someone-else".to_string();
        let tampered_bytes = serde_json::to_vec(&tampered).unwrap();

        assert_eq!(
            DidKeyEd25519Verifier.verify(&owner, &tampered_bytes),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_an_owner_that_is_not_a_did_key() {
        let (key_pair, _) = new_owner();
        let grant = sign_grant(&key_pair, "org/ns/ch1", "org/ns/agent1", "member", 1, "n");

        assert_eq!(
            DidKeyEd25519Verifier.verify("not-a-did-key", &grant),
            Err(GrantError::UnsupportedOwnerFormat(
                "not-a-did-key".to_string()
            ))
        );
    }

    #[test]
    fn rejects_malformed_grant_bytes() {
        let (_, owner) = new_owner();
        let err = DidKeyEd25519Verifier
            .verify(&owner, b"not json")
            .unwrap_err();
        assert!(matches!(err, GrantError::MalformedGrant(_)));
    }

    #[test]
    fn rejects_a_did_key_with_the_wrong_multicodec_prefix() {
        let (key_pair, _) = new_owner();
        // A 34-byte payload that doesn't start with the Ed25519 prefix (0xed, 0x01).
        let bogus = bs58::encode(
            [0x00, 0x01]
                .iter()
                .chain([0u8; 32].iter())
                .copied()
                .collect::<Vec<u8>>(),
        )
        .into_string();
        let owner = format!("did:key:z{bogus}");
        let grant = sign_grant(&key_pair, "org/ns/ch1", "org/ns/agent1", "member", 1, "n");

        assert!(matches!(
            DidKeyEd25519Verifier.verify(&owner, &grant),
            Err(GrantError::UnsupportedOwnerFormat(_))
        ));
    }
}

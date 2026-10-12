// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Signed grants: proof that a channel's owner authorized a specific
//! participant change. Also the owner's signed create and delete requests,
//! which make whoever signs a create the channel's owner and let only that
//! owner delete it.
//!
//! Verification is pluggable (`GrantVerifier`) so a deployment can swap in
//! its own grant format and key scheme -- e.g. one that resolves an agent's
//! binding certificate back to its human owner -- without this crate
//! depending on that format. [`DidKeyEd25519Verifier`] is the default: it
//! understands an owner identifier that is a `did:key` encoding an Ed25519
//! public key (self-certifying -- no registry lookup needed) and this
//! module's own JSON envelopes (`SignedGrant`, `SignedCreate`,
//! `SignedDelete`).

use std::fmt;

use aws_lc_rs::signature::{self, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// Which change a grant authorizes. Part of what's signed, so a grant to
/// add someone can't be spent removing them, or the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantAction {
    Add,
    Delete,
}

impl GrantAction {
    fn as_str(self) -> &'static str {
        match self {
            GrantAction::Add => "add",
            GrantAction::Delete => "delete",
        }
    }
}

/// The parsed, authenticated contents of a grant: what the owner actually
/// authorized. Verifying a grant proves it was signed by the channel's
/// owner -- it does not, on its own, prove it applies to the request at
/// hand. Callers must still check `channel`/`invitee`/`action`/`not_after`
/// against the request they received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub channel: String,
    pub invitee: String,
    pub action: GrantAction,
    pub role: String,
    /// Unix seconds after which the grant is no longer valid.
    pub not_after: u64,
    /// Opaque replay-protection token. Not enforced here -- a nonce store
    /// tracking which values have already been consumed is separate work.
    pub nonce: String,
}

/// The authenticated contents of a signed create request. Its signer,
/// `owner`, becomes the channel's owner. As with [`Grant`], callers must
/// check every field against the request it came with, so a signature for
/// one set of settings can't create a channel with another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCreation {
    /// The channel manager the request is meant for, so it can't be
    /// replayed against another one.
    pub audience: String,
    pub channel: String,
    pub owner: String,
    pub mls_enabled: bool,
    pub ttl_seconds: Option<u64>,
    pub owner_callback_name: Option<String>,
    /// Unix seconds after which the request is no longer valid.
    pub not_after: u64,
    pub nonce: String,
}

/// The authenticated contents of a signed delete request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDeletion {
    /// The channel manager the request is meant for.
    pub audience: String,
    pub channel: String,
    /// Unix seconds after which the request is no longer valid.
    pub not_after: u64,
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
            GrantError::MalformedGrant(reason) => write!(f, "malformed: {reason}"),
            GrantError::InvalidSignature => {
                write!(f, "the signature does not verify against the signer's key")
            }
        }
    }
}

impl std::error::Error for GrantError {}

/// Authenticates a grant and parses what it authorizes.
///
/// A deployment supplies its own implementation to `ChannelManagerServer`
/// (see `ChannelManagerServer::with_grant_verifier`) to use a different
/// grant format or key scheme than the default. Implementations only
/// authenticate and parse -- checking the parsed `Grant` against the live
/// request (channel, invitee, action, expiry) is the caller's job, so every
/// verifier gets that enforcement uniformly rather than having to implement
/// it itself.
pub trait GrantVerifier: Send + Sync {
    /// Verifies `grant` was signed by `owner`, returning its parsed
    /// contents on success.
    fn verify(&self, owner: &str, grant: &[u8]) -> Result<Grant, GrantError>;

    /// Verifies a signed create request, returning what it authorizes,
    /// including who signed it. The default refuses, so a verifier written
    /// before signed creation existed keeps compiling, and signed creates
    /// fail with it.
    fn verify_create(&self, proof: &[u8]) -> Result<ChannelCreation, GrantError> {
        let _ = proof;
        Err(GrantError::MalformedGrant(
            "this grant verifier does not support signed channel creation".to_string(),
        ))
    }

    /// Verifies a delete request was signed by `owner`. The default refuses,
    /// like [`Self::verify_create`].
    fn verify_delete(&self, owner: &str, proof: &[u8]) -> Result<ChannelDeletion, GrantError> {
        let _ = (owner, proof);
        Err(GrantError::MalformedGrant(
            "this grant verifier does not support signed channel deletion".to_string(),
        ))
    }
}

/// The wire format [`DidKeyEd25519Verifier`] expects in a request's `grant`
/// field: the grant's fields alongside a detached signature over their
/// canonical encoding (see `canonical_bytes`). `action` is `"add"` or
/// `"delete"`.
#[derive(Debug, Serialize, Deserialize)]
struct SignedGrant {
    channel: String,
    invitee: String,
    action: GrantAction,
    role: String,
    not_after: u64,
    nonce: String,
    /// Base64-encoded (standard, padded) Ed25519 signature.
    signature: String,
}

/// Domain-separation tag: the first element of every grant's signed bytes,
/// so a signature made for something else with the same key can never
/// verify as a grant. Bump the version if the layout below changes.
const GRANT_DOMAIN: &str = "SLIM-CHANNEL-GRANT/1";

/// Refuses a field containing a NUL byte. The signed bytes join fields with
/// NUL, so one inside a field could shift the boundaries between them.
fn reject_nul(fields: &[&str]) -> Result<(), GrantError> {
    if fields.iter().any(|field| field.contains('\0')) {
        return Err(GrantError::MalformedGrant(
            "fields must not contain NUL bytes".to_string(),
        ));
    }
    Ok(())
}

/// Byte representation a grant is signed over: [`GRANT_DOMAIN`] then the
/// grant's fields, in this order, NUL-separated. Fields containing NUL are
/// refused (see `reject_nul`), so this needs no escaping and has no
/// field-order ambiguity, unlike signing re-serialized JSON would.
fn canonical_bytes(grant: &Grant) -> Vec<u8> {
    [
        GRANT_DOMAIN,
        grant.channel.as_str(),
        grant.invitee.as_str(),
        grant.action.as_str(),
        grant.role.as_str(),
        &grant.not_after.to_string(),
        grant.nonce.as_str(),
    ]
    .join("\0")
    .into_bytes()
}

/// The wire format [`DidKeyEd25519Verifier`] expects in a create request's
/// `owner_proof`: the request's settings and its signer, signed with the
/// signer's key (see `create_canonical_bytes`). Optional settings are
/// omitted (or null) when unset.
#[derive(Debug, Serialize, Deserialize)]
struct SignedCreate {
    audience: String,
    channel: String,
    owner: String,
    mls_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_callback_name: Option<String>,
    not_after: u64,
    nonce: String,
    /// Base64-encoded (standard, padded) Ed25519 signature.
    signature: String,
}

/// Domain-separation tag of a signed create request, distinct from
/// [`GRANT_DOMAIN`] and [`DELETE_DOMAIN`] so no signed statement can pass
/// for another kind.
const CREATE_DOMAIN: &str = "SLIM-CHANNEL-CREATE/1";

/// Bytes a create request is signed over: [`CREATE_DOMAIN`], then its
/// fields in this order, NUL-separated. `mls_enabled` is `true` or `false`;
/// an unset `ttl_seconds` or `owner_callback_name` is the empty string.
fn create_canonical_bytes(creation: &ChannelCreation) -> Vec<u8> {
    [
        CREATE_DOMAIN,
        creation.audience.as_str(),
        creation.channel.as_str(),
        creation.owner.as_str(),
        if creation.mls_enabled {
            "true"
        } else {
            "false"
        },
        &creation
            .ttl_seconds
            .map(|ttl| ttl.to_string())
            .unwrap_or_default(),
        creation.owner_callback_name.as_deref().unwrap_or_default(),
        &creation.not_after.to_string(),
        creation.nonce.as_str(),
    ]
    .join("\0")
    .into_bytes()
}

/// The wire format [`DidKeyEd25519Verifier`] expects in a delete request's
/// `owner_proof`, signed with the owner's key (see `delete_canonical_bytes`).
#[derive(Debug, Serialize, Deserialize)]
struct SignedDelete {
    audience: String,
    channel: String,
    not_after: u64,
    nonce: String,
    /// Base64-encoded (standard, padded) Ed25519 signature.
    signature: String,
}

/// Domain-separation tag of a signed delete request.
const DELETE_DOMAIN: &str = "SLIM-CHANNEL-DELETE/1";

/// Bytes a delete request is signed over: [`DELETE_DOMAIN`], `audience`,
/// `channel`, `not_after` and `nonce`, NUL-separated.
fn delete_canonical_bytes(deletion: &ChannelDeletion) -> Vec<u8> {
    [
        DELETE_DOMAIN,
        deletion.audience.as_str(),
        deletion.channel.as_str(),
        &deletion.not_after.to_string(),
        deletion.nonce.as_str(),
    ]
    .join("\0")
    .into_bytes()
}

/// Checks `signature` (standard, padded base64) is `signer`'s Ed25519
/// signature over `message`.
fn check_signature(signer: &[u8; 32], message: &[u8], signature: &str) -> Result<(), GrantError> {
    let signature = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature)
        .map_err(|e| GrantError::MalformedGrant(format!("signature: {e}")))?;
    UnparsedPublicKey::new(&signature::ED25519, &signer[..])
        .verify(message, &signature)
        .map_err(|_| GrantError::InvalidSignature)
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

/// An Ed25519 key that signs grants and create and delete requests in the
/// formats [`DidKeyEd25519Verifier`] checks: a channel owner's key, or an
/// operator's. Kept beside the verifier so the two can't drift apart.
pub struct SigningKey {
    key_pair: Ed25519KeyPair,
    did_key: String,
}

impl SigningKey {
    /// Loads an Ed25519 private key from a PKCS#8 PEM document, as
    /// `openssl genpkey -algorithm ed25519` writes.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self, slim_auth::errors::AuthError> {
        let key_pair = slim_auth::did_key::ed25519_key_pair_from_pkcs8_pem(pem)?;
        let did_key = slim_auth::did_key::ed25519_did_key(key_pair.public_key().as_ref());
        Ok(SigningKey { key_pair, did_key })
    }

    /// The key's `did:key`: the owner its create requests name, and how
    /// `operators` lists it.
    pub fn did_key(&self) -> &str {
        &self.did_key
    }

    fn signature(&self, message: &[u8]) -> String {
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            self.key_pair.sign(message).as_ref(),
        )
    }

    /// `grant`, signed: the bytes a request's `grant` field carries.
    pub fn sign_grant(&self, grant: &Grant) -> Vec<u8> {
        let signed = SignedGrant {
            channel: grant.channel.clone(),
            invitee: grant.invitee.clone(),
            action: grant.action,
            role: grant.role.clone(),
            not_after: grant.not_after,
            nonce: grant.nonce.clone(),
            signature: self.signature(&canonical_bytes(grant)),
        };
        serde_json::to_vec(&signed).expect("a signed grant serializes")
    }

    /// `creation`, signed: the bytes `CreateChannelRequest.owner_proof`
    /// carries. Its `owner` must be this key's did:key, since a signed
    /// create names its signer.
    pub fn sign_create(&self, creation: &ChannelCreation) -> Result<Vec<u8>, GrantError> {
        if creation.owner != self.did_key {
            return Err(GrantError::MalformedGrant(
                "a signed create request's owner must be the signing key's did:key".to_string(),
            ));
        }
        let signed = SignedCreate {
            audience: creation.audience.clone(),
            channel: creation.channel.clone(),
            owner: creation.owner.clone(),
            mls_enabled: creation.mls_enabled,
            ttl_seconds: creation.ttl_seconds,
            owner_callback_name: creation.owner_callback_name.clone(),
            not_after: creation.not_after,
            nonce: creation.nonce.clone(),
            signature: self.signature(&create_canonical_bytes(creation)),
        };
        Ok(serde_json::to_vec(&signed).expect("a signed create request serializes"))
    }

    /// `deletion`, signed: the bytes `DeleteChannelRequest.owner_proof`
    /// carries.
    pub fn sign_delete(&self, deletion: &ChannelDeletion) -> Vec<u8> {
        let signed = SignedDelete {
            audience: deletion.audience.clone(),
            channel: deletion.channel.clone(),
            not_after: deletion.not_after,
            nonce: deletion.nonce.clone(),
            signature: self.signature(&delete_canonical_bytes(deletion)),
        };
        serde_json::to_vec(&signed).expect("a signed delete request serializes")
    }
}

/// Default [`GrantVerifier`]: owner is a `did:key`-encoded Ed25519 public
/// key, and grants, create and delete requests are this module's JSON
/// envelopes signed with that key.
#[derive(Debug, Default, Clone, Copy)]
pub struct DidKeyEd25519Verifier;

impl GrantVerifier for DidKeyEd25519Verifier {
    fn verify(&self, owner: &str, grant: &[u8]) -> Result<Grant, GrantError> {
        let pubkey_bytes = decode_ed25519_did_key(owner)?;

        let signed: SignedGrant =
            serde_json::from_slice(grant).map_err(|e| GrantError::MalformedGrant(e.to_string()))?;
        reject_nul(&[
            &signed.channel,
            &signed.invitee,
            &signed.role,
            &signed.nonce,
        ])?;
        let grant = Grant {
            channel: signed.channel,
            invitee: signed.invitee,
            action: signed.action,
            role: signed.role,
            not_after: signed.not_after,
            nonce: signed.nonce,
        };

        check_signature(&pubkey_bytes, &canonical_bytes(&grant), &signed.signature)?;
        Ok(grant)
    }

    /// The signer is the `owner` the request names: self-certifying, as a
    /// did:key carries its own public key.
    fn verify_create(&self, proof: &[u8]) -> Result<ChannelCreation, GrantError> {
        let signed: SignedCreate =
            serde_json::from_slice(proof).map_err(|e| GrantError::MalformedGrant(e.to_string()))?;
        // An empty callback name would sign the same bytes as none at all.
        if signed.owner_callback_name.as_deref() == Some("") {
            return Err(GrantError::MalformedGrant(
                "owner_callback_name must be omitted rather than empty".to_string(),
            ));
        }
        reject_nul(&[
            &signed.audience,
            &signed.channel,
            &signed.owner,
            signed.owner_callback_name.as_deref().unwrap_or_default(),
            &signed.nonce,
        ])?;
        let pubkey_bytes = decode_ed25519_did_key(&signed.owner)?;
        let creation = ChannelCreation {
            audience: signed.audience,
            channel: signed.channel,
            owner: signed.owner,
            mls_enabled: signed.mls_enabled,
            ttl_seconds: signed.ttl_seconds,
            owner_callback_name: signed.owner_callback_name,
            not_after: signed.not_after,
            nonce: signed.nonce,
        };

        check_signature(
            &pubkey_bytes,
            &create_canonical_bytes(&creation),
            &signed.signature,
        )?;
        Ok(creation)
    }

    fn verify_delete(&self, owner: &str, proof: &[u8]) -> Result<ChannelDeletion, GrantError> {
        let pubkey_bytes = decode_ed25519_did_key(owner)?;

        let signed: SignedDelete =
            serde_json::from_slice(proof).map_err(|e| GrantError::MalformedGrant(e.to_string()))?;
        reject_nul(&[&signed.audience, &signed.channel, &signed.nonce])?;
        let deletion = ChannelDeletion {
            audience: signed.audience,
            channel: signed.channel,
            not_after: signed.not_after,
            nonce: signed.nonce,
        };

        check_signature(
            &pubkey_bytes,
            &delete_canonical_bytes(&deletion),
            &signed.signature,
        )?;
        Ok(deletion)
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

    fn grant() -> Grant {
        Grant {
            channel: "org/ns/ch1".to_string(),
            invitee: "org/ns/agent1".to_string(),
            action: GrantAction::Add,
            role: "member".to_string(),
            not_after: 9_999_999_999,
            nonce: "nonce-1".to_string(),
        }
    }

    fn sign(key_pair: &Ed25519KeyPair, grant: &Grant) -> SignedGrant {
        let signature = key_pair.sign(&canonical_bytes(grant));
        SignedGrant {
            channel: grant.channel.clone(),
            invitee: grant.invitee.clone(),
            action: grant.action,
            role: grant.role.clone(),
            not_after: grant.not_after,
            nonce: grant.nonce.clone(),
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signature.as_ref(),
            ),
        }
    }

    fn to_bytes(signed: &SignedGrant) -> Vec<u8> {
        serde_json::to_vec(signed).unwrap()
    }

    #[test]
    fn verifies_a_correctly_signed_grant() {
        let (key_pair, owner) = new_owner();
        let bytes = to_bytes(&sign(&key_pair, &grant()));

        assert_eq!(DidKeyEd25519Verifier.verify(&owner, &bytes), Ok(grant()));
    }

    #[test]
    fn serializes_the_action_as_a_lowercase_string() {
        let (key_pair, _) = new_owner();
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(&sign(&key_pair, &grant()))).unwrap();

        assert_eq!(json["action"], "add");
    }

    #[test]
    fn rejects_a_grant_signed_by_a_different_key() {
        let (signer_key, _) = new_owner();
        let (_, claimed_owner) = new_owner();
        let bytes = to_bytes(&sign(&signer_key, &grant()));

        assert_eq!(
            DidKeyEd25519Verifier.verify(&claimed_owner, &bytes),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_tampered_invitee() {
        let (key_pair, owner) = new_owner();
        let mut tampered = sign(&key_pair, &grant());
        tampered.invitee = "org/ns/someone-else".to_string();

        assert_eq!(
            DidKeyEd25519Verifier.verify(&owner, &to_bytes(&tampered)),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_an_add_grant_rewritten_as_a_delete() {
        let (key_pair, owner) = new_owner();
        let mut tampered = sign(&key_pair, &grant());
        tampered.action = GrantAction::Delete;

        assert_eq!(
            DidKeyEd25519Verifier.verify(&owner, &to_bytes(&tampered)),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_signature_made_without_the_domain_prefix() {
        let (key_pair, owner) = new_owner();
        let mut signed = sign(&key_pair, &grant());
        let unprefixed = &canonical_bytes(&grant())[GRANT_DOMAIN.len() + 1..];
        signed.signature = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            key_pair.sign(unprefixed).as_ref(),
        );

        assert_eq!(
            DidKeyEd25519Verifier.verify(&owner, &to_bytes(&signed)),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_an_unknown_action() {
        let (key_pair, owner) = new_owner();
        let mut json: serde_json::Value =
            serde_json::from_slice(&to_bytes(&sign(&key_pair, &grant()))).unwrap();
        json["action"] = "promote".into();

        let err = DidKeyEd25519Verifier
            .verify(&owner, json.to_string().as_bytes())
            .unwrap_err();
        assert!(matches!(err, GrantError::MalformedGrant(_)));
    }

    #[test]
    fn rejects_an_owner_that_is_not_a_did_key() {
        let (key_pair, _) = new_owner();
        let bytes = to_bytes(&sign(&key_pair, &grant()));

        assert_eq!(
            DidKeyEd25519Verifier.verify("not-a-did-key", &bytes),
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

    // ── signed create and delete requests ─────────────────────────────

    fn creation(owner: &str) -> ChannelCreation {
        ChannelCreation {
            audience: "org/ns/channel-manager".to_string(),
            channel: "org/ns/ch1".to_string(),
            owner: owner.to_string(),
            mls_enabled: true,
            ttl_seconds: Some(3600),
            owner_callback_name: Some("org/ns/owner-endpoint".to_string()),
            not_after: 9_999_999_999,
            nonce: "nonce-c".to_string(),
        }
    }

    fn signed_create(key_pair: &Ed25519KeyPair, creation: &ChannelCreation) -> serde_json::Value {
        let signature = key_pair.sign(&create_canonical_bytes(creation));
        serde_json::to_value(SignedCreate {
            audience: creation.audience.clone(),
            channel: creation.channel.clone(),
            owner: creation.owner.clone(),
            mls_enabled: creation.mls_enabled,
            ttl_seconds: creation.ttl_seconds,
            owner_callback_name: creation.owner_callback_name.clone(),
            not_after: creation.not_after,
            nonce: creation.nonce.clone(),
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signature.as_ref(),
            ),
        })
        .unwrap()
    }

    fn deletion() -> ChannelDeletion {
        ChannelDeletion {
            audience: "org/ns/channel-manager".to_string(),
            channel: "org/ns/ch1".to_string(),
            not_after: 9_999_999_999,
            nonce: "nonce-d".to_string(),
        }
    }

    fn signed_delete(key_pair: &Ed25519KeyPair, deletion: &ChannelDeletion) -> serde_json::Value {
        let signature = key_pair.sign(&delete_canonical_bytes(deletion));
        serde_json::to_value(SignedDelete {
            audience: deletion.audience.clone(),
            channel: deletion.channel.clone(),
            not_after: deletion.not_after,
            nonce: deletion.nonce.clone(),
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                signature.as_ref(),
            ),
        })
        .unwrap()
    }

    #[test]
    fn verifies_a_signed_create_and_names_its_signer() {
        let (key_pair, owner) = new_owner();
        let proof = signed_create(&key_pair, &creation(&owner)).to_string();

        assert_eq!(
            DidKeyEd25519Verifier.verify_create(proof.as_bytes()),
            Ok(creation(&owner))
        );
    }

    #[test]
    fn verifies_a_signed_create_without_optional_settings() {
        let (key_pair, owner) = new_owner();
        let bare = ChannelCreation {
            ttl_seconds: None,
            owner_callback_name: None,
            ..creation(&owner)
        };
        let proof = signed_create(&key_pair, &bare);
        assert!(proof.get("ttl_seconds").is_none());

        assert_eq!(
            DidKeyEd25519Verifier.verify_create(proof.to_string().as_bytes()),
            Ok(bare)
        );
    }

    #[test]
    fn rejects_a_create_with_a_changed_setting() {
        let (key_pair, owner) = new_owner();
        for (field, value) in [
            ("mls_enabled", serde_json::json!(false)),
            ("ttl_seconds", serde_json::json!(1)),
            ("owner_callback_name", serde_json::json!("org/ns/attacker")),
            ("channel", serde_json::json!("org/ns/other")),
            ("audience", serde_json::json!("org/ns/another-manager")),
        ] {
            let mut proof = signed_create(&key_pair, &creation(&owner));
            proof[field] = value;
            assert_eq!(
                DidKeyEd25519Verifier.verify_create(proof.to_string().as_bytes()),
                Err(GrantError::InvalidSignature),
                "{field}"
            );
        }
    }

    #[test]
    fn rejects_a_create_naming_an_owner_other_than_its_signer() {
        let (signer, _) = new_owner();
        let (_, claimed_owner) = new_owner();
        let proof = signed_create(&signer, &creation(&claimed_owner)).to_string();

        assert_eq!(
            DidKeyEd25519Verifier.verify_create(proof.as_bytes()),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_create_with_an_empty_callback_name() {
        let (key_pair, owner) = new_owner();
        let empty = ChannelCreation {
            owner_callback_name: Some(String::new()),
            ..creation(&owner)
        };
        let proof = signed_create(&key_pair, &empty).to_string();

        assert!(matches!(
            DidKeyEd25519Verifier.verify_create(proof.as_bytes()),
            Err(GrantError::MalformedGrant(_))
        ));
    }

    #[test]
    fn verifies_a_signed_delete() {
        let (key_pair, owner) = new_owner();
        let proof = signed_delete(&key_pair, &deletion()).to_string();

        assert_eq!(
            DidKeyEd25519Verifier.verify_delete(&owner, proof.as_bytes()),
            Ok(deletion())
        );
    }

    #[test]
    fn rejects_a_delete_signed_by_someone_other_than_the_owner() {
        let (signer, _) = new_owner();
        let (_, owner) = new_owner();
        let proof = signed_delete(&signer, &deletion()).to_string();

        assert_eq!(
            DidKeyEd25519Verifier.verify_delete(&owner, proof.as_bytes()),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_delete_retargeted_at_another_channel_or_manager() {
        let (key_pair, owner) = new_owner();
        for (field, value) in [
            ("channel", "org/ns/other"),
            ("audience", "org/ns/another-manager"),
        ] {
            let mut proof = signed_delete(&key_pair, &deletion());
            proof[field] = value.into();
            assert_eq!(
                DidKeyEd25519Verifier.verify_delete(&owner, proof.to_string().as_bytes()),
                Err(GrantError::InvalidSignature),
                "{field}"
            );
        }
    }

    #[test]
    fn a_delete_signature_does_not_pass_as_a_create() {
        // Same key, same channel, same nonce: only the domain tells them apart.
        let (key_pair, owner) = new_owner();
        let delete_signature = key_pair.sign(&delete_canonical_bytes(&deletion()));
        let mut proof = signed_create(&key_pair, &creation(&owner));
        proof["signature"] = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            delete_signature.as_ref(),
        )
        .into();

        assert_eq!(
            DidKeyEd25519Verifier.verify_create(proof.to_string().as_bytes()),
            Err(GrantError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_fields_containing_nul() {
        let (key_pair, owner) = new_owner();
        let nul_invitee = Grant {
            invitee: "org/ns/agent1\0member".to_string(),
            ..grant()
        };
        let bytes = to_bytes(&sign(&key_pair, &nul_invitee));
        assert!(matches!(
            DidKeyEd25519Verifier.verify(&owner, &bytes),
            Err(GrantError::MalformedGrant(_))
        ));

        let nul_channel = ChannelCreation {
            channel: "org/ns/ch1\0x".to_string(),
            ..creation(&owner)
        };
        let proof = signed_create(&key_pair, &nul_channel).to_string();
        assert!(matches!(
            DidKeyEd25519Verifier.verify_create(proof.as_bytes()),
            Err(GrantError::MalformedGrant(_))
        ));

        let nul_nonce = ChannelDeletion {
            nonce: "n\0".to_string(),
            ..deletion()
        };
        let proof = signed_delete(&key_pair, &nul_nonce).to_string();
        assert!(matches!(
            DidKeyEd25519Verifier.verify_delete(&owner, proof.as_bytes()),
            Err(GrantError::MalformedGrant(_))
        ));
    }

    #[test]
    fn a_verifier_without_signed_requests_refuses_them() {
        struct GrantsOnly;
        impl GrantVerifier for GrantsOnly {
            fn verify(&self, _: &str, _: &[u8]) -> Result<Grant, GrantError> {
                unreachable!()
            }
        }

        assert!(matches!(
            GrantsOnly.verify_create(b"{}"),
            Err(GrantError::MalformedGrant(_))
        ));
        assert!(matches!(
            GrantsOnly.verify_delete("did:key:z", b"{}"),
            Err(GrantError::MalformedGrant(_))
        ));
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
        let bytes = to_bytes(&sign(&key_pair, &grant()));

        assert!(matches!(
            DidKeyEd25519Verifier.verify(&owner, &bytes),
            Err(GrantError::UnsupportedOwnerFormat(_))
        ));
    }

    // ── SigningKey ────────────────────────────────────────────────────

    /// A fresh key as a PKCS#8 PEM document.
    fn new_pem() -> String {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let body =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pkcs8.as_ref());
        format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n")
    }

    #[test]
    fn a_signing_key_loads_a_pem_and_knows_its_did_key() {
        let pem = new_pem();
        let key = SigningKey::from_pkcs8_pem(&pem).unwrap();
        assert_eq!(
            key.did_key(),
            slim_auth::did_key::ed25519_did_key_from_pkcs8_pem(&pem).unwrap()
        );
        assert!(SigningKey::from_pkcs8_pem("not a key").is_err());
    }

    #[test]
    fn what_a_signing_key_signs_the_default_verifier_accepts() {
        let key = SigningKey::from_pkcs8_pem(&new_pem()).unwrap();
        let owner = key.did_key().to_string();

        let grant_bytes = key.sign_grant(&grant());
        assert_eq!(
            DidKeyEd25519Verifier.verify(&owner, &grant_bytes),
            Ok(grant())
        );

        let create_bytes = key.sign_create(&creation(&owner)).unwrap();
        assert_eq!(
            DidKeyEd25519Verifier.verify_create(&create_bytes),
            Ok(creation(&owner))
        );

        let delete_bytes = key.sign_delete(&deletion());
        assert_eq!(
            DidKeyEd25519Verifier.verify_delete(&owner, &delete_bytes),
            Ok(deletion())
        );
    }

    #[test]
    fn a_signing_key_refuses_a_create_naming_another_owner() {
        let key = SigningKey::from_pkcs8_pem(&new_pem()).unwrap();
        let (_, someone_else) = new_owner();
        assert!(matches!(
            key.sign_create(&creation(&someone_else)),
            Err(GrantError::MalformedGrant(_))
        ));
    }
}

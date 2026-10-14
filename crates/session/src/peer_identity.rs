// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The verified identity of the peer that set up a session.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::Value;
use slim_auth::identity_claims::IdentityClaims;
use slim_datapath::api::{NameId, ProtoName};

/// Message metadata key carrying the verified identity (`sub`) of a data
/// message's sender. Sessions remove it from every message they receive and
/// set it only after an MLS decryption that identifies the sender.
pub const PEER_SUBJECT_KEY: &str = "slim-peer-subject";

/// Who set up a session, as their identity token said, verified together
/// with the signature over the message that carried it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    subject: String,
    issuer: Option<String>,
}

impl PeerIdentity {
    pub fn new(subject: impl Into<String>, issuer: Option<String>) -> Self {
        PeerIdentity {
            subject: subject.into(),
            issuer,
        }
    }

    /// The token's subject (`sub`): e.g. a `did:key`, a SPIFFE ID, or a
    /// shared-secret id.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The token's issuer (`iss`), when it has one.
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }

    /// From a verified token's claims.
    pub(crate) fn from_claims(claims: &Value) -> Option<Self> {
        let subject = IdentityClaims::from_json(claims).ok()?.subject;
        let issuer = claims
            .get("iss")
            .and_then(Value::as_str)
            .map(str::to_string);
        Some(PeerIdentity { subject, issuer })
    }
}

/// The readable part of a name: its three encoded components.
type Readable = (u64, u64, u64);

fn readable(name: &ProtoName) -> Option<Readable> {
    name.name
        .as_ref()
        .map(|n| (n.component_0, n.component_1, n.component_2))
}

/// What a session knows about its peers' identities: the identity each
/// invite is pinned to, by readable name, and the verified identity each
/// member joined with, by full name.
#[derive(Debug, Default)]
pub(crate) struct PeerIdentities {
    pins: Mutex<HashMap<Readable, String>>,
    members: Mutex<HashMap<(Readable, u128), PeerIdentity>>,
}

// Each lock guards one map insert or lookup, so a panic can't leave either
// map half-updated: sharing this across an unwind is safe, and keeps
// `SessionController` `UnwindSafe`.
impl std::panic::RefUnwindSafe for PeerIdentities {}

impl PeerIdentities {
    /// Accept `name` only from a peer verified as `subject`.
    pub(crate) fn pin(&self, name: &ProtoName, subject: String) {
        if let Some(key) = readable(name) {
            self.pins.lock().insert(key, subject);
        }
    }

    /// The identity `name` is pinned to, if any.
    pub(crate) fn pinned(&self, name: &ProtoName) -> Option<String> {
        readable(name).and_then(|key| self.pins.lock().get(&key).cloned())
    }

    /// Remember that the member `name` joined as `identity`.
    pub(crate) fn record(&self, name: &ProtoName, identity: PeerIdentity) {
        if let Some(key) = readable(name) {
            self.members.lock().insert((key, name.id()), identity);
        }
    }

    /// The identity the member `name` joined as. A name without an id matches
    /// any member with that readable name.
    pub(crate) fn of(&self, name: &ProtoName) -> Option<PeerIdentity> {
        let key = readable(name)?;
        let members = self.members.lock();
        if name.id() != NameId::NULL_COMPONENT {
            return members.get(&(key, name.id())).cloned();
        }
        members
            .iter()
            .find(|((readable, _), _)| *readable == key)
            .map(|(_, identity)| identity.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_subject_and_issuer_from_claims() {
        let claims = serde_json::json!({
            "sub": "did:key:z6Mkagent", "iss": "did:key:z6Mkagent", "pubkey": "AAAA",
        });
        assert_eq!(
            PeerIdentity::from_claims(&claims),
            Some(PeerIdentity::new(
                "did:key:z6Mkagent",
                Some("did:key:z6Mkagent".to_string())
            ))
        );
    }

    #[test]
    fn issuer_is_optional() {
        let claims = serde_json::json!({ "sub": "org/ns/agent", "pubkey": "AAAA" });
        let identity = PeerIdentity::from_claims(&claims).unwrap();
        assert_eq!(identity.subject(), "org/ns/agent");
        assert_eq!(identity.issuer(), None);
    }
}

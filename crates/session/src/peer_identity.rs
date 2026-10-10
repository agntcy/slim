// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The verified identity of the peer that set up a session.

use serde_json::Value;
use slim_auth::identity_claims::IdentityClaims;

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

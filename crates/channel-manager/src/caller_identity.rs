// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The verified identity of whoever is calling a channel-manager RPC.

use slim_auth::metadata::MetadataMap;

/// The verified caller of an RPC, sourced from whichever auth backend (JWT,
/// OIDC, SPIRE) the server has configured.
///
/// `None` everywhere this type appears means no usable identity was found —
/// either no auth middleware ran (the server has no `auth` configured) or it
/// ran but the verified claims carried no `sub`. That is distinct from "the
/// caller is anonymous": today it just means no identity to attach to a
/// channel or a grant, not that the call should be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    /// The verified subject: a SPIFFE ID, an OIDC/JWT `sub` claim, or
    /// whichever identifier the configured auth backend verifies.
    pub subject: String,
}

impl CallerIdentity {
    /// Extracts the caller's identity from a request's extensions.
    ///
    /// The configured auth middleware (`slim_auth::jwt_middleware`, wired in
    /// for JWT/OIDC/SPIRE alike) already placed a `MetadataMap` of verified
    /// claims there before this request reached the handler — this just
    /// reads the `sub` claim back out. Must be called before
    /// `Request::into_inner`, which drops the extensions.
    pub fn from_request<T>(request: &tonic::Request<T>) -> Option<Self> {
        let claims = request.extensions().get::<MetadataMap>()?;
        let subject = claims.get("sub")?.as_str()?.to_string();
        Some(CallerIdentity { subject })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_request_reads_the_sub_claim() {
        let mut claims = MetadataMap::new();
        claims.insert("sub", "spiffe://example.org/ns/default/sa/agent");
        let mut request = tonic::Request::new(());
        request.extensions_mut().insert(claims);

        assert_eq!(
            CallerIdentity::from_request(&request),
            Some(CallerIdentity {
                subject: "spiffe://example.org/ns/default/sa/agent".to_string(),
            })
        );
    }

    #[test]
    fn from_request_is_none_without_auth_middleware() {
        let request = tonic::Request::new(());
        assert_eq!(CallerIdentity::from_request(&request), None);
    }

    #[test]
    fn from_request_is_none_without_a_sub_claim() {
        let mut claims = MetadataMap::new();
        claims.insert("groups", vec!["admins"]);
        let mut request = tonic::Request::new(());
        request.extensions_mut().insert(claims);

        assert_eq!(CallerIdentity::from_request(&request), None);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The verified identity of whoever is calling a channel-manager RPC.

use slim_auth::metadata::MetadataMap;
use tonic::transport::server::TcpConnectInfo;
use tonic_tls::rustls::SslConnectInfo;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::{FromDer, X509Certificate};

/// Connect info `slim_config`'s gRPC server attaches to a request that
/// arrived over TLS on a TCP listener.
type TlsConnectInfo = SslConnectInfo<TcpConnectInfo>;

/// The verified caller of an RPC: the `sub` of the token the server's auth
/// middleware (JWT, OIDC, SPIRE) verified or, failing that, the SPIFFE ID of
/// the client certificate verified during the mTLS handshake.
///
/// `None` everywhere this type appears means no usable identity was found —
/// neither verified claims with a `sub` nor a verified client certificate
/// carrying a SPIFFE ID. That is distinct from "the caller is anonymous":
/// today it just means no identity to attach to a channel or a grant, not
/// that the call should be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    /// The verified subject: a SPIFFE ID, an OIDC/JWT `sub` claim, or
    /// whichever identifier the configured auth backend verifies.
    pub subject: String,
}

impl CallerIdentity {
    /// Extracts the caller's identity from a request's extensions. Must be
    /// called before `Request::into_inner`, which drops the extensions.
    ///
    /// Token claims win over the client certificate: a token names the
    /// caller, while a certificate may identify only the workload carrying
    /// the call.
    pub fn from_request<T>(request: &tonic::Request<T>) -> Option<Self> {
        Self::from_claims(request).or_else(|| Self::from_client_certificate(request))
    }

    /// The configured auth middleware (`slim_auth::jwt_middleware`, wired in
    /// for JWT/OIDC/SPIRE alike) places a `MetadataMap` of verified claims
    /// in the extensions; this reads its `sub` claim.
    fn from_claims<T>(request: &tonic::Request<T>) -> Option<Self> {
        let claims = request.extensions().get::<MetadataMap>()?;
        let subject = claims.get("sub")?.as_str()?.to_string();
        Some(CallerIdentity { subject })
    }

    /// The server only accepts client certificates that chain to its
    /// configured client CA, so any certificate present here was verified.
    fn from_client_certificate<T>(request: &tonic::Request<T>) -> Option<Self> {
        let certs = request.extensions().get::<TlsConnectInfo>()?.peer_certs()?;
        // The client's own certificate comes first in the chain.
        let subject = spiffe_id(certs.first()?.as_ref())?;
        Some(CallerIdentity { subject })
    }
}

/// The SPIFFE ID of a DER-encoded certificate: its URI SAN, when it has
/// exactly one and it is a `spiffe://` URI -- an X.509-SVID carries exactly
/// one URI SAN, so a certificate with several isn't one.
fn spiffe_id(der: &[u8]) -> Option<String> {
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let san = cert.subject_alternative_name().ok()??;
    let mut uris = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) => Some(*uri),
            _ => None,
        });
    let uri = uris.next()?;
    if uris.next().is_some() {
        return None;
    }
    uri.starts_with("spiffe://").then(|| uri.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType};

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

    /// DER of a throwaway self-signed certificate with these SANs.
    fn certificate_with(subject_alt_names: Vec<SanType>) -> Vec<u8> {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.subject_alt_names = subject_alt_names;
        let key = KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().der().to_vec()
    }

    fn uri(uri: &str) -> SanType {
        SanType::URI(uri.try_into().unwrap())
    }

    #[test]
    fn spiffe_id_reads_an_svids_uri_san() {
        assert_eq!(
            spiffe_id(&certificate_with(vec![uri("spiffe://example.org/owner")])),
            Some("spiffe://example.org/owner".to_string())
        );
    }

    #[test]
    fn spiffe_id_is_none_without_a_uri_san() {
        let dns = SanType::DnsName("localhost".try_into().unwrap());
        assert_eq!(spiffe_id(&certificate_with(vec![dns])), None);
    }

    #[test]
    fn spiffe_id_is_none_with_several_uri_sans() {
        let two = vec![uri("spiffe://example.org/a"), uri("spiffe://example.org/b")];
        assert_eq!(spiffe_id(&certificate_with(two)), None);
    }

    #[test]
    fn spiffe_id_is_none_for_a_non_spiffe_uri() {
        let https = vec![uri("https://example.org/agent")];
        assert_eq!(spiffe_id(&certificate_with(https)), None);
    }

    #[test]
    fn spiffe_id_is_none_for_bytes_that_are_not_a_certificate() {
        assert_eq!(spiffe_id(b"not a certificate"), None);
    }
}

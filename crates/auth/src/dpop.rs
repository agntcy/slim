// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! DPoP (RFC 9449) proofs and RFC 7638 JWK thumbprints over the app's MLS
//! signature key.
//!
//! Possession is proved once, at token exchange; the IdP returns the key's
//! thumbprint as `cnf.jkt`. MLS leaf-node signatures are the continuing proof
//! afterwards, so there is no per-message DPoP overhead.
//!
//! Ciphersuite comes from the public key length, as in
//! [`crate::utils::sign_header_aad`]: 32 → Ed25519/`EdDSA`, 33 or 65 → P-256/`ES256`.
//!
//! Avoids `jsonwebtoken` and `reqwest` so it compiles for wasm32, where
//! `validate_member` still needs thumbprints.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use sha2::{Digest, Sha256};

use crate::errors::AuthError;

/// The RFC 7638 canonical JWK of an MLS public key, plus its JOSE `alg`.
///
/// Required members only, lexicographically ordered, no whitespace. Serves as
/// both the thumbprint hash input and a proof's `jwk` header member.
fn canonical_jwk(public_key: &[u8]) -> Result<(String, &'static str), AuthError> {
    match public_key.len() {
        32 => Ok((
            format!(
                r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#,
                B64URL.encode(public_key)
            ),
            "EdDSA",
        )),
        // Decompress first: one key must not thumbprint differently depending on
        // which SEC1 encoding the provider handed back.
        33 | 65 => {
            let verifying_key = crate::utils::p256_verifying_key(public_key)?;
            let point = verifying_key.to_encoded_point(false);
            let x = point.x().ok_or(AuthError::DpopUnsupportedKeyType)?;
            let y = point.y().ok_or(AuthError::DpopUnsupportedKeyType)?;
            Ok((
                format!(
                    r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
                    B64URL.encode(x),
                    B64URL.encode(y)
                ),
                "ES256",
            ))
        }
        _ => Err(AuthError::DpopUnsupportedKeyType),
    }
}

/// RFC 7638 JWK thumbprint of an MLS public key — the value an IdP puts in
/// `cnf.jkt`, and what a peer recomputes to check the binding.
pub fn jwk_thumbprint(public_key: &[u8]) -> Result<String, AuthError> {
    let (jwk, _) = canonical_jwk(public_key)?;
    Ok(B64URL.encode(Sha256::digest(jwk.as_bytes())))
}

/// Read `cnf.jkt` from an access token, or `None` if it is unbound.
///
/// Unverified by design: only ever compared against a key the caller already
/// holds. `OidcVerifier` validates peer credentials against JWKS separately.
pub fn token_confirmation(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(&B64URL.decode(payload).ok()?).ok()?;
    claims.get("cnf")?.get("jkt")?.as_str().map(str::to_owned)
}

/// Build a DPoP proof JWT binding an HTTP request to the MLS signing key.
///
/// `htm` is the HTTP method (e.g. `POST`) and `htu` the target URI; per RFC 9449
/// §4.2 the query and fragment are stripped from `htu` before signing.
///
/// `nonce` answers a `use_dpop_nonce` challenge (RFC 9449 §8); pass `None` on
/// the first attempt.
pub fn build_proof(
    private_key: &[u8],
    public_key: &[u8],
    htm: &str,
    htu: &str,
    nonce: Option<&str>,
) -> Result<String, AuthError> {
    let (jwk, alg) = canonical_jwk(public_key)?;

    let htu = htu.split(['?', '#']).next().unwrap_or(htu);

    let iat = web_time::SystemTime::now()
        .duration_since(web_time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut jti_bytes = [0u8; 16];
    rand::Rng::fill(&mut rand::rng(), &mut jti_bytes);

    let header = format!(r#"{{"typ":"dpop+jwt","alg":"{alg}","jwk":{jwk}}}"#);
    // `htu` comes from a discovery document, so let serde escape it.
    let mut payload = serde_json::json!({
        "htm": htm,
        "htu": htu,
        "iat": iat,
        "jti": B64URL.encode(jti_bytes),
    });
    if let (Some(nonce), Some(obj)) = (nonce, payload.as_object_mut()) {
        obj.insert("nonce".to_string(), serde_json::Value::String(nonce.into()));
    }
    let payload = payload.to_string();

    let signing_input = format!(
        "{}.{}",
        B64URL.encode(header.as_bytes()),
        B64URL.encode(payload.as_bytes())
    );
    let signature = sign_jws(signing_input.as_bytes(), private_key, public_key)?;

    Ok(format!("{signing_input}.{}", B64URL.encode(signature)))
}

/// Sign JWS signing input. JOSE wants fixed-width `R || S` for `ES256`, not the
/// DER encoding [`crate::utils::sign_header_aad`] produces.
fn sign_jws(
    signing_input: &[u8],
    private_key: &[u8],
    public_key: &[u8],
) -> Result<Vec<u8>, AuthError> {
    match public_key.len() {
        32 => {
            use ed25519_dalek::Signer;

            let key = crate::utils::ed25519_signing_key(private_key, public_key)?;
            Ok(key.sign(signing_input).to_bytes().to_vec())
        }
        33 | 65 => {
            use p256::ecdsa::Signature;
            use p256::ecdsa::signature::Signer as _;

            let key = crate::utils::p256_signing_key(private_key, public_key)?;
            let signature: Signature = key.sign(signing_input);
            Ok(signature.to_bytes().to_vec())
        }
        _ => Err(AuthError::DpopUnsupportedKeyType),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// RFC 7638 §3.1 worked example. It uses an RSA key, which MLS never
    /// produces, so exercise the hash-and-encode step directly against the
    /// canonical JSON the RFC prints to pin down that half of the algorithm.
    #[test]
    fn rfc7638_thumbprint_vector() {
        let canonical = r#"{"e":"AQAB","kty":"RSA","n":"0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw"}"#;
        assert_eq!(
            B64URL.encode(Sha256::digest(canonical.as_bytes())),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }

    /// A key handed back compressed and uncompressed is one key, so it must
    /// yield one thumbprint — otherwise `cnf.jkt` would fail to match whenever
    /// the token side and the MLS side disagreed on point encoding.
    #[test]
    fn p256_thumbprint_is_encoding_independent() {
        let (_, compressed, uncompressed) = test_p256_keys();
        assert_eq!(
            jwk_thumbprint(&compressed).unwrap(),
            jwk_thumbprint(&uncompressed).unwrap()
        );
    }

    #[test]
    fn distinct_keys_get_distinct_thumbprints() {
        let (_, a, _) = test_p256_keys();
        let (_, b, _) = test_p256_keys();
        assert_ne!(jwk_thumbprint(&a).unwrap(), jwk_thumbprint(&b).unwrap());
    }

    #[test]
    fn unsupported_key_length_rejected() {
        assert!(matches!(
            jwk_thumbprint(&[0u8; 20]),
            Err(AuthError::DpopUnsupportedKeyType)
        ));
    }

    /// The proof must verify under the very key whose thumbprint it advertises —
    /// that identity is the whole point of the binding.
    #[test]
    fn p256_proof_verifies_against_advertised_key() {
        use p256::ecdsa::Signature;
        use p256::ecdsa::signature::Verifier;

        let (secret, public, _) = test_p256_keys();
        let proof = build_proof(&secret, &public, "POST", "https://idp/token?x=1#f", None).unwrap();

        let parts: Vec<&str> = proof.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header: Value = serde_json::from_slice(&B64URL.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["typ"], "dpop+jwt");
        assert_eq!(header["alg"], "ES256");
        // The advertised jwk is the presented key.
        assert_eq!(
            B64URL.encode(Sha256::digest(
                serde_json::to_string(&header["jwk"]).unwrap().as_bytes()
            )),
            jwk_thumbprint(&public).unwrap()
        );

        let payload: Value = serde_json::from_slice(&B64URL.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(payload["htm"], "POST");
        // Query and fragment stripped per RFC 9449 §4.2.
        assert_eq!(payload["htu"], "https://idp/token");
        assert!(payload["iat"].as_u64().unwrap() > 0);
        assert!(!payload["jti"].as_str().unwrap().is_empty());

        let signature = Signature::from_slice(&B64URL.decode(parts[2]).unwrap()).unwrap();
        crate::utils::p256_verifying_key(&public)
            .unwrap()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn ed25519_proof_verifies_against_advertised_key() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let (secret, public) = test_ed25519_keys();
        let proof = build_proof(&secret, &public, "POST", "https://idp/token", None).unwrap();

        let parts: Vec<&str> = proof.split('.').collect();
        let header: Value = serde_json::from_slice(&B64URL.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["jwk"]["kty"], "OKP");

        let signature = Signature::from_slice(&B64URL.decode(parts[2]).unwrap()).unwrap();
        VerifyingKey::from_bytes(public[..].try_into().unwrap())
            .unwrap()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    /// Two proofs from one key must differ, or the IdP's `jti` replay cache
    /// rejects the second request.
    #[test]
    fn proofs_are_unique_per_call() {
        let (secret, public, _) = test_p256_keys();
        let a = build_proof(&secret, &public, "POST", "https://idp/token", None).unwrap();
        let b = build_proof(&secret, &public, "POST", "https://idp/token", None).unwrap();
        assert_ne!(a, b);
    }

    fn test_p256_keys() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        use p256::SecretKey;
        use p256::elliptic_curve::rand_core::OsRng;

        let secret_key = SecretKey::random(&mut OsRng);
        let verifying_key = p256::ecdsa::SigningKey::from(&secret_key);
        let verifying_key = verifying_key.verifying_key();
        (
            secret_key.to_bytes().to_vec(),
            verifying_key.to_encoded_point(true).as_bytes().to_vec(),
            verifying_key.to_encoded_point(false).as_bytes().to_vec(),
        )
    }

    fn test_ed25519_keys() -> (Vec<u8>, Vec<u8>) {
        use ed25519_dalek::SigningKey;
        use rand::Rng;

        let mut seed = [0u8; 32];
        rand::rng().fill(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let public = signing_key.verifying_key().to_bytes().to_vec();
        (seed.to_vec(), public)
    }
}

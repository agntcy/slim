// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The keys of a `did:key` identity, as a JWK and a JWKS.
//!
//! [`Service::create_app_with_did_key`](crate::Service) takes the app's private key as a
//! [`PrivateJwk`] and the keys it trusts as a [`JwkSet`], and converts them to what the
//! identity machinery uses: a PKCS#8 PEM document for the private key, and the JWKS of the
//! trust anchor.
//!
//! A JWK names its key type (`kty` and `crv`), so the functions take no key type argument.
//! Only `OKP` with `Ed25519` (RFC 8037) is supported: it is the key type `did:key` is built
//! on in `slim-auth` (the tokens are signed with `EdDSA`).

use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve};
// The types of the trust anchor, from `jsonwebtoken`, so that callers need no dependency on it.
pub use jsonwebtoken::jwk::{Jwk, JwkSet};
use serde::Deserialize;
use slim_auth::did_key::{ed25519_did_key, ed25519_public_key_from_pkcs8_pem};

use crate::errors::ServiceError;

/// The bytes of a PKCS#8 v1 document for an Ed25519 key, up to the 32 bytes of its seed.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

const ED25519_KEY_LEN: usize = 32;

fn invalid(message: impl fmt::Display) -> ServiceError {
    ServiceError::InvalidConfig(format!("invalid did:key identity: {message}"))
}

/// The private key of an identity, as a JWK (RFC 7517, RFC 8037). It is not a [`Jwk`]:
/// that type only holds public keys, and has no member for `d`. The members are `kty`
/// (`OKP`), `crv` (`Ed25519`) and `d`, the base64url-encoded 32-byte seed. `x`, the
/// base64url-encoded public key, is optional; if present, it must belong to `d`.
///
/// ```json
/// {"kty":"OKP","crv":"Ed25519","d":"nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A",
///  "x":"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"}
/// ```
#[derive(Clone, Deserialize)]
pub struct PrivateJwk {
    kty: String,
    crv: String,
    d: String,
    #[serde(default)]
    x: Option<String>,
}

impl PrivateJwk {
    /// Parse a JWK document.
    pub fn from_json(json: &str) -> Result<Self, ServiceError> {
        serde_json::from_str(json).map_err(|e| {
            invalid(format!(
                "the private key is not a JWK with kty, crv and d: {e}"
            ))
        })
    }

    /// The 32-byte seed, after checking the key type.
    fn seed(&self) -> Result<Vec<u8>, ServiceError> {
        if self.kty != "OKP" || self.crv != "Ed25519" {
            return Err(invalid(format!(
                "the private key is a {} {} key: only OKP Ed25519 is supported",
                self.kty, self.crv
            )));
        }
        let seed = decode_member("d", &self.d)?;
        if seed.len() != ED25519_KEY_LEN {
            return Err(invalid(format!(
                "the member d of the private key has {} bytes: expected 32",
                seed.len()
            )));
        }
        Ok(seed)
    }
}

// The private key stays out of the logs.
impl fmt::Debug for PrivateJwk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateJwk")
            .field("kty", &self.kty)
            .field("crv", &self.crv)
            .field("d", &"<redacted>")
            .field("x", &self.x)
            .finish()
    }
}

impl FromStr for PrivateJwk {
    type Err = ServiceError;

    fn from_str(json: &str) -> Result<Self, Self::Err> {
        Self::from_json(json)
    }
}

/// A base64url member of a JWK, with or without padding.
fn decode_member(name: &str, value: &str) -> Result<Vec<u8>, ServiceError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|e| invalid(format!("the member {name} of a key is not base64url: {e}")))
}

/// The private key as a PKCS#8 PEM document.
pub(crate) fn private_key_pem(key: &PrivateJwk) -> Result<String, ServiceError> {
    let seed = key.seed()?;
    let pem = slim_auth::utils::bytes_to_pem(
        &[PKCS8_ED25519_PREFIX.as_slice(), &seed].concat(),
        "-----BEGIN PRIVATE KEY-----\n",
        "\n-----END PRIVATE KEY-----\n",
    );
    // `x`, when given, has to be the public key of `d`.
    if let Some(x) = &key.x
        && decode_member("x", x)? != ed25519_public_key_from_pkcs8_pem(&pem)?
    {
        return Err(invalid(
            "the member x of the private key is not the public key of d",
        ));
    }
    Ok(pem)
}

/// The `did:key` of the private key.
pub fn did_key_of_private_key(key: &PrivateJwk) -> Result<String, ServiceError> {
    let public_key = ed25519_public_key_from_pkcs8_pem(&private_key_pem(key)?)?;
    Ok(ed25519_did_key(&public_key))
}

/// The public key of an `OKP` `Ed25519` JWK.
fn okp_public_key(jwk: &Jwk) -> Result<Vec<u8>, ServiceError> {
    match &jwk.algorithm {
        AlgorithmParameters::OctetKeyPair(okp) if okp.curve == EllipticCurve::Ed25519 => {
            let key = decode_member("x", &okp.x)?;
            if key.len() != ED25519_KEY_LEN {
                return Err(invalid(format!(
                    "a trusted key has {} bytes: expected 32",
                    key.len()
                )));
            }
            Ok(key)
        }
        _ => Err(invalid(
            "a trusted key is not an OKP Ed25519 JWK: only that key type is supported",
        )),
    }
}

/// The JWKS of the trust anchor: the keys in `trusted`, and `own_public_key` if they do not
/// list it. The app has to trust its own key, since MLS verifies the app's own token
/// against the trust anchor.
pub(crate) fn trusted_keys_jwks(
    trusted: &JwkSet,
    own_public_key: &[u8],
) -> Result<String, ServiceError> {
    let mut keys = trusted.keys.clone();
    let mut lists_own_key = false;
    for jwk in &keys {
        lists_own_key |= okp_public_key(jwk)? == own_public_key;
    }
    if !lists_own_key {
        let own: Jwk = serde_json::from_value(serde_json::json!({
            "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig",
            "x": URL_SAFE_NO_PAD.encode(own_public_key),
        }))
        .map_err(|e| invalid(format!("cannot build the JWK of the app's own key: {e}")))?;
        keys.push(own);
    }
    serde_json::to_string(&JwkSet { keys })
        .map_err(|e| invalid(format!("cannot serialize the trust anchor: {e}")))
}

/// The public key of the private key, for the trust anchor.
pub(crate) fn public_key_of(key: &PrivateJwk) -> Result<Vec<u8>, ServiceError> {
    Ok(ed25519_public_key_from_pkcs8_pem(&private_key_pem(key)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8032 section 7.1, test 1, as in RFC 8037 appendix A.
    const SEED: &str = "nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A";
    const PUBLIC: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
    // RFC 8032 section 7.1, test 2.
    const PUBLIC_2: &str = "PUAXw-hDiVqStwqnTRt-vJyYLM8uxJaMwM1V8Sr0Zgw";

    fn private_jwk(d: &str, x: Option<&str>) -> PrivateJwk {
        let x = x.map(|x| format!(r#","x":"{x}""#)).unwrap_or_default();
        PrivateJwk::from_json(&format!(r#"{{"kty":"OKP","crv":"Ed25519","d":"{d}"{x}}}"#)).unwrap()
    }

    fn jwks(public_keys: &[&str]) -> JwkSet {
        let keys: Vec<String> = public_keys
            .iter()
            .map(|x| {
                format!(r#"{{"kty":"OKP","crv":"Ed25519","alg":"EdDSA","use":"sig","x":"{x}"}}"#)
            })
            .collect();
        serde_json::from_str(&format!(r#"{{"keys":[{}]}}"#, keys.join(","))).unwrap()
    }

    #[test]
    fn the_did_key_is_that_of_the_public_key() {
        let public = URL_SAFE_NO_PAD.decode(PUBLIC).unwrap();
        // With and without `x`: the same key.
        for key in [private_jwk(SEED, None), private_jwk(SEED, Some(PUBLIC))] {
            assert_eq!(
                did_key_of_private_key(&key).unwrap(),
                ed25519_did_key(&public)
            );
            assert_eq!(public_key_of(&key).unwrap(), public);
        }
        // base64url with padding is accepted too.
        assert!(private_key_pem(&private_jwk(&format!("{SEED}="), None)).is_ok());
    }

    #[test]
    fn a_private_key_that_is_not_an_ed25519_jwk_is_rejected() {
        let wrong_type = PrivateJwk::from_json(r#"{"kty":"EC","crv":"P-256","d":"AAAA"}"#).unwrap();
        let err = private_key_pem(&wrong_type).unwrap_err();
        assert!(err.to_string().contains("only OKP Ed25519"), "{err}");

        // A seed of another size.
        let err = private_key_pem(&private_jwk("AAAA", None)).unwrap_err();
        assert!(err.to_string().contains("expected 32"), "{err}");
        // Not base64url.
        assert!(private_key_pem(&private_jwk("not base64!", None)).is_err());
        // `x` of another key.
        let err = private_key_pem(&private_jwk(SEED, Some(PUBLIC_2))).unwrap_err();
        assert!(err.to_string().contains("not the public key of d"), "{err}");
        // Not a JWK at all.
        assert!(PrivateJwk::from_json("not a jwk").is_err());
        assert!(PrivateJwk::from_json(r#"{"kty":"OKP","crv":"Ed25519"}"#).is_err());
    }

    #[test]
    fn the_private_key_stays_out_of_debug_output() {
        let text = format!("{:?}", private_jwk(SEED, Some(PUBLIC)));
        assert!(!text.contains(SEED), "{text}");
    }

    #[test]
    fn the_trust_anchor_holds_the_own_key_and_the_trusted_ones_once() {
        let own = URL_SAFE_NO_PAD.decode(PUBLIC).unwrap();
        let count = |jwks: &str| jwks.matches("\"kty\":\"OKP\"").count();

        // The own key is added when the trusted keys do not list it...
        let added = trusted_keys_jwks(&jwks(&[PUBLIC_2]), &own).unwrap();
        assert_eq!(count(&added), 2);
        assert!(added.contains(PUBLIC) && added.contains(PUBLIC_2));
        // ...and not twice when they do.
        let listed = trusted_keys_jwks(&jwks(&[PUBLIC_2, PUBLIC]), &own).unwrap();
        assert_eq!(count(&listed), 2);
        // An app that trusts nobody else trusts itself.
        assert_eq!(count(&trusted_keys_jwks(&jwks(&[]), &own).unwrap()), 1);
    }

    #[test]
    fn a_trusted_key_of_another_type_is_rejected() {
        let rsa: JwkSet = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","n":"sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw","e":"AQAB"}]}"#,
        )
        .unwrap();
        let err = trusted_keys_jwks(&rsa, &[0; 32]).unwrap_err();
        assert!(err.to_string().contains("not an OKP Ed25519 JWK"), "{err}");
    }
}

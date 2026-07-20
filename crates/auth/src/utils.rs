// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! General utility helpers for the authentication crate.

use base64::Engine;
#[cfg(not(target_arch = "wasm32"))]
use mls_rs_core::crypto::CipherSuiteProvider;
#[cfg(not(target_arch = "wasm32"))]
use mls_rs_core::crypto::CryptoProvider;
#[cfg(not(target_arch = "wasm32"))]
use mls_rs_crypto_awslc::AwsLcCryptoProvider;

// Must stay in lock-step with the MLS layer's ciphersuite
// (`agntcy-slim-mls`): the `curve25519` feature is propagated from that crate so
// the keys an identity ships are always valid for the ciphersuite MLS uses. The
// default (P-256) matches MLS's default; enabling `curve25519` switches both.
#[cfg(all(not(target_arch = "wasm32"), feature = "curve25519"))]
const CIPHERSUITE: mls_rs_core::crypto::CipherSuite =
    mls_rs_core::crypto::CipherSuite::CURVE25519_AES128;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "curve25519")))]
const CIPHERSUITE: mls_rs_core::crypto::CipherSuite = mls_rs_core::crypto::CipherSuite::P256_AES128;

/// Generate an MLS signature key pair valid for the ciphersuite MLS uses.
///
/// Returns provider-compatible `(secret_key_bytes, public_key_bytes)`: AWS-LC
/// bytes for the active `CIPHERSUITE` on native targets (P-256 by default, or
/// Curve25519 with the `curve25519` feature), and PKCS#8 DER plus an
/// uncompressed SEC1 public key for browser WebCrypto.
pub fn generate_mls_signature_keys() -> Result<(Vec<u8>, Vec<u8>), crate::errors::AuthError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let crypto_provider = AwsLcCryptoProvider::default();
        let cipher_suite_provider = crypto_provider
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or(crate::errors::AuthError::MlsKeyGenerationFailed)?;

        let (secret_key, public_key) = cipher_suite_provider
            .signature_key_generate()
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;

        Ok((
            secret_key.as_bytes().to_vec(),
            public_key.as_bytes().to_vec(),
        ))
    }

    #[cfg(target_arch = "wasm32")]
    {
        // AWS-LC cannot target the browser, so generate the P-256 key with the
        // pure-Rust `p256` crate. MLS uses the WebCrypto provider on wasm,
        // whose ECDSA key import requires a PKCS#8 private key (not the raw
        // scalar accepted by the native AWS-LC provider). The public key stays
        // an uncompressed SEC1 point. A random 32-byte value is a valid scalar
        // with overwhelming probability; retry the vanishingly rare
        // rejections.
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        use p256::pkcs8::EncodePrivateKey;
        for _ in 0..8 {
            let mut scalar = [0_u8; 32];
            getrandom::fill(&mut scalar)
                .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
            if let Ok(secret_key) = p256::SecretKey::from_bytes((&scalar).into()) {
                let private = secret_key
                    .to_pkcs8_der()
                    .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?
                    .as_bytes()
                    .to_vec();
                let public = secret_key
                    .public_key()
                    .to_encoded_point(false)
                    .as_bytes()
                    .to_vec();
                return Ok((private, public));
            }
        }
        Err(crate::errors::AuthError::MlsKeyGenerationFailed)
    }
}

/// Sign the header AAD bytes using the MLS signature key pair.
///
/// Supports Ed25519 (Curve25519 MLS ciphersuite) and ECDSA P-256 (default MLS
/// ciphersuite), selected from the public key encoding length.
pub fn sign_header_aad(
    aad_bytes: &[u8],
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<Vec<u8>, crate::errors::AuthError> {
    match public_key_bytes.len() {
        32 => sign_header_aad_ed25519(aad_bytes, private_key_bytes, public_key_bytes),
        33 | 65 => sign_header_aad_p256(aad_bytes, private_key_bytes, public_key_bytes),
        _ => Err(crate::errors::AuthError::MlsKeyGenerationFailed),
    }
}

/// Verify the header AAD signature using an MLS signature public key.
pub fn verify_header_aad(
    aad_bytes: &[u8],
    signature_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), crate::errors::AuthError> {
    match public_key_bytes.len() {
        32 => verify_header_aad_ed25519(aad_bytes, signature_bytes, public_key_bytes),
        33 | 65 => verify_header_aad_p256(aad_bytes, signature_bytes, public_key_bytes),
        _ => Err(crate::errors::AuthError::TokenInvalid),
    }
}

fn sign_header_aad_ed25519(
    aad_bytes: &[u8],
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<Vec<u8>, crate::errors::AuthError> {
    use ed25519_dalek::Signer;

    let signing_key = ed25519_signing_key(private_key_bytes, public_key_bytes)?;
    Ok(signing_key.sign(aad_bytes).to_bytes().to_vec())
}

fn sign_header_aad_p256(
    aad_bytes: &[u8],
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<Vec<u8>, crate::errors::AuthError> {
    use p256::ecdsa::Signature;
    use p256::ecdsa::signature::Signer as _;

    let signing_key = p256_signing_key(private_key_bytes, public_key_bytes)?;
    let signature: Signature = signing_key.sign(aad_bytes);
    Ok(signature.to_der().as_bytes().to_vec())
}

fn verify_header_aad_ed25519(
    aad_bytes: &[u8],
    signature_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), crate::errors::AuthError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let verifying_key = VerifyingKey::from_bytes(
        public_key_bytes
            .try_into()
            .map_err(|_| crate::errors::AuthError::TokenInvalid)?,
    )
    .map_err(|_| crate::errors::AuthError::TokenInvalid)?;
    let signature = Signature::from_bytes(
        signature_bytes
            .try_into()
            .map_err(|_| crate::errors::AuthError::TokenInvalid)?,
    );
    verifying_key
        .verify(aad_bytes, &signature)
        .map_err(|_| crate::errors::AuthError::TokenInvalid)
}

fn verify_header_aad_p256(
    aad_bytes: &[u8],
    signature_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), crate::errors::AuthError> {
    use p256::ecdsa::Signature;
    use p256::ecdsa::signature::Verifier;

    let verifying_key = p256_verifying_key(public_key_bytes)?;
    let signature =
        Signature::from_der(signature_bytes).map_err(|_| crate::errors::AuthError::TokenInvalid)?;
    verifying_key
        .verify(aad_bytes, &signature)
        .map_err(|_| crate::errors::AuthError::TokenInvalid)
}

fn ed25519_signing_key(
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<ed25519_dalek::SigningKey, crate::errors::AuthError> {
    use ed25519_dalek::SigningKey;

    if private_key_bytes.len() >= 64 {
        let keypair: [u8; 64] = private_key_bytes[..64]
            .try_into()
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
        SigningKey::from_keypair_bytes(&keypair)
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)
    } else {
        let seed: [u8; 32] = private_key_bytes
            .try_into()
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
        let signing_key = SigningKey::from_bytes(&seed);
        let expected_public = signing_key.verifying_key().to_bytes();
        let provided_public: [u8; 32] = public_key_bytes
            .try_into()
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
        if expected_public != provided_public {
            return Err(crate::errors::AuthError::MlsKeyGenerationFailed);
        }
        Ok(signing_key)
    }
}

fn p256_signing_key(
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<p256::ecdsa::SigningKey, crate::errors::AuthError> {
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::DecodePrivateKey;

    // A P-256 private scalar is a 32-byte big-endian integer, but the AWS-LC
    // provider returns it with leading zero bytes stripped, so a scalar whose
    // top byte(s) are zero comes back as 31 (or fewer) bytes — roughly 1 key in
    // 256. Left-pad any raw scalar of at most 32 bytes back to the fixed width
    // before importing; only genuinely longer encodings are treated as PKCS#8
    // DER. (Before this, a 31-byte scalar fell through to the DER branch and
    // failed with `MlsKeyGenerationFailed`, breaking header signing ~1/256 of
    // the time.)
    if private_key_bytes.len() <= 32 {
        let mut scalar = [0u8; 32];
        scalar[32 - private_key_bytes.len()..].copy_from_slice(private_key_bytes);
        let secret_key = p256::SecretKey::from_bytes((&scalar).into())
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
        let signing_key = SigningKey::from(&secret_key);
        let verifying_key = p256_verifying_key(public_key_bytes)?;
        if *signing_key.verifying_key() != verifying_key {
            return Err(crate::errors::AuthError::MlsKeyGenerationFailed);
        }
        Ok(signing_key)
    } else {
        SigningKey::from_pkcs8_der(private_key_bytes)
            .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)
    }
}

fn p256_verifying_key(
    public_key_bytes: &[u8],
) -> Result<p256::ecdsa::VerifyingKey, crate::errors::AuthError> {
    use p256::EncodedPoint;
    use p256::ecdsa::VerifyingKey;
    use p256::elliptic_curve::sec1::FromEncodedPoint;

    let point = EncodedPoint::from_bytes(public_key_bytes)
        .map_err(|_| crate::errors::AuthError::MlsKeyGenerationFailed)?;
    let public_key = p256::PublicKey::from_encoded_point(&point)
        .into_option()
        .ok_or(crate::errors::AuthError::MlsKeyGenerationFailed)?;
    Ok(VerifyingKey::from(&public_key))
}

/// Convert arbitrary bytes into a PEM-formatted string with the provided header/footer.
/// The body is wrapped at 64 character lines per RFC 7468 guidance.
/// Header/footer should include the BEGIN/END lines with trailing/leading newlines as desired.
pub fn bytes_to_pem(key_bytes: &[u8], header: &str, footer: &str) -> String {
    // Use base64 with standard encoding (not URL safe)
    let encoded = base64::engine::general_purpose::STANDARD.encode(key_bytes);

    // Insert newlines every 64 characters as per PEM format
    let mut pem_body = String::new();
    for i in 0..(encoded.len().div_ceil(64)) {
        let start = i * 64;
        let end = std::cmp::min(start + 64, encoded.len());
        if start < encoded.len() {
            pem_body.push_str(&encoded[start..end]);
            if end < encoded.len() {
                pem_body.push('\n');
            }
        }
    }

    format!("{}{}{}", header, pem_body, footer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_ed25519_keys() -> (Vec<u8>, Vec<u8>) {
        use ed25519_dalek::SigningKey;
        use rand::Rng;

        let mut seed_bytes = [0u8; 32];
        rand::rng().fill(&mut seed_bytes);
        let signing_key = SigningKey::from_bytes(&seed_bytes);
        let public = signing_key.verifying_key().to_bytes();
        (seed_bytes.to_vec(), public.to_vec())
    }

    fn generate_test_p256_keys() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        use p256::SecretKey;
        use p256::elliptic_curve::rand_core::OsRng;

        let secret_key = SecretKey::random(&mut OsRng);
        let secret_bytes = secret_key.to_bytes().to_vec();
        let signing_key = p256::ecdsa::SigningKey::from(&secret_key);
        let verifying_key = signing_key.verifying_key();
        let public_compressed = verifying_key.to_encoded_point(true).as_bytes().to_vec();
        let public_uncompressed = verifying_key.to_encoded_point(false).as_bytes().to_vec();
        (secret_bytes, public_compressed, public_uncompressed)
    }

    #[test]
    fn test_sign_header_aad_with_generated_keys() {
        let (secret, public) = generate_test_ed25519_keys();
        let msg = b"hello";
        let sig = sign_header_aad(msg, &secret, &public).unwrap();
        verify_header_aad(msg, &sig, &public).unwrap();
    }

    #[test]
    fn test_sign_header_aad_with_concatenated_ed25519_private_key() {
        let (seed, public) = generate_test_ed25519_keys();
        let mut concatenated = seed;
        concatenated.extend_from_slice(&public);
        let msg = b"hello";
        let sig = sign_header_aad(msg, &concatenated, &public).unwrap();
        verify_header_aad(msg, &sig, &public).unwrap();
    }

    #[test]
    fn test_sign_verify_p256_compressed() {
        let (secret, public_comp, _) = generate_test_p256_keys();
        let msg = b"hello p256 compressed";
        let sig = sign_header_aad(msg, &secret, &public_comp).unwrap();
        verify_header_aad(msg, &sig, &public_comp).unwrap();
    }

    #[test]
    fn test_sign_verify_p256_uncompressed() {
        let (secret, _, public_uncomp) = generate_test_p256_keys();
        let msg = b"hello p256 uncompressed";
        let sig = sign_header_aad(msg, &secret, &public_uncomp).unwrap();
        verify_header_aad(msg, &sig, &public_uncomp).unwrap();
    }

    #[test]
    fn test_sign_verify_p256_pkcs8() {
        use p256::SecretKey;
        use p256::elliptic_curve::rand_core::OsRng;
        use p256::pkcs8::EncodePrivateKey;

        let secret_key = SecretKey::random(&mut OsRng);
        let pkcs8_der = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();

        let signing_key = p256::ecdsa::SigningKey::from(&secret_key);
        let verifying_key = signing_key.verifying_key();
        let public_comp = verifying_key.to_encoded_point(true).as_bytes().to_vec();

        let msg = b"hello p256 pkcs8";
        let sig = sign_header_aad(msg, &pkcs8_der, &public_comp).unwrap();
        verify_header_aad(msg, &sig, &public_comp).unwrap();
    }

    #[test]
    fn test_invalid_key_lengths() {
        let msg = b"invalid keys";
        // Invalid public key length
        let res_sign = sign_header_aad(msg, &[0; 32], &[0; 10]);
        assert!(matches!(
            res_sign,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        let res_verify = verify_header_aad(msg, &[0; 64], &[0; 10]);
        assert!(matches!(
            res_verify,
            Err(crate::errors::AuthError::TokenInvalid)
        ));
    }

    #[test]
    fn test_ed25519_key_errors() {
        let (secret, public) = generate_test_ed25519_keys();
        let msg = b"ed25519 error tests";

        // Mismatched public key
        let mut wrong_public = public.clone();
        wrong_public[0] ^= 0xFF;
        let res_sign = sign_header_aad(msg, &secret, &wrong_public);
        assert!(matches!(
            res_sign,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Invalid private key length (neither 32 nor >=64)
        let res_sign_len = sign_header_aad(msg, &[0; 16], &public);
        assert!(matches!(
            res_sign_len,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Invalid signature verification
        let sig = sign_header_aad(msg, &secret, &public).unwrap();
        let mut bad_sig = sig.clone();
        bad_sig[0] ^= 0xFF;
        let res_verify = verify_header_aad(msg, &bad_sig, &public);
        assert!(matches!(
            res_verify,
            Err(crate::errors::AuthError::TokenInvalid)
        ));

        // Signature too short
        let res_verify_short = verify_header_aad(msg, &sig[..10], &public);
        assert!(matches!(
            res_verify_short,
            Err(crate::errors::AuthError::TokenInvalid)
        ));
    }

    #[test]
    fn test_p256_key_errors() {
        let (secret, public_comp, _) = generate_test_p256_keys();
        let msg = b"p256 error tests";

        // Mismatched public key
        let mut wrong_public = public_comp.clone();
        // Modify to make it a valid but different public key point if possible, or just invalid/mismatched.
        wrong_public[1] ^= 0xFF;
        let res_sign = sign_header_aad(msg, &secret, &wrong_public);
        assert!(matches!(
            res_sign,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Valid point, but mismatched public key
        let (_, other_public, _) = generate_test_p256_keys();
        let res_sign_mismatch = sign_header_aad(msg, &secret, &other_public);
        assert!(matches!(
            res_sign_mismatch,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Invalid private key bytes (32 bytes but all zeros - not a valid scalar)
        let res_sign_zero = sign_header_aad(msg, &[0; 32], &public_comp);
        assert!(matches!(
            res_sign_zero,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Invalid PKCS8 DER private key
        let res_sign_der = sign_header_aad(msg, &[0; 40], &public_comp);
        assert!(matches!(
            res_sign_der,
            Err(crate::errors::AuthError::MlsKeyGenerationFailed)
        ));

        // Invalid signature format for verification
        let sig = sign_header_aad(msg, &secret, &public_comp).unwrap();
        let mut bad_sig = sig.clone();
        if !bad_sig.is_empty() {
            bad_sig[0] ^= 0xFF;
        }
        let res_verify = verify_header_aad(msg, &bad_sig, &public_comp);
        assert!(matches!(
            res_verify,
            Err(crate::errors::AuthError::TokenInvalid)
        ));
    }

    #[test]
    fn test_bytes_to_pem_basic() {
        let data = b"hello world"; // base64: aGVsbG8gd29ybGQ=
        let pem = bytes_to_pem(data, "-----BEGIN TEST-----\n", "\n-----END TEST-----");
        assert!(pem.starts_with("-----BEGIN TEST-----"));
        assert!(pem.ends_with("-----END TEST-----"));
        assert!(pem.contains("aGVsbG8gd29ybGQ="));
    }

    #[test]
    fn test_bytes_to_pem_line_wrapping() {
        // 70 bytes -> base64 longer than 64 chars to test wrapping
        let data = vec![b'A'; 70];
        let pem = bytes_to_pem(&data, "-----BEGIN TEST-----\n", "\n-----END TEST-----");
        // Count lines between header and footer
        let body = pem
            .replace("-----BEGIN TEST-----\n", "")
            .replace("\n-----END TEST-----", "");
        let lines: Vec<&str> = body.split('\n').collect();
        assert!(lines.len() >= 2, "Should have wrapped into multiple lines");
        assert!(lines.iter().all(|l| l.len() <= 64));
    }
}

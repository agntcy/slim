// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/// Centralized valid shared secret for tests (>=32 chars).
/// Use this for simple cases where the actual value does not matter.
pub const TEST_VALID_SECRET: &str = "test-shared-secret-value-0123456789abcdef";

use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::signature::KeyPair; // Import the KeyPair trait for public_key() method
use aws_lc_rs::{rand, rsa, signature};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken_aws_lc::Algorithm;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub async fn setup_test_jwt_resolver(algorithm: Algorithm) -> (String, MockServer, String) {
    // Set up the mock server for JWKS
    let mock_server = MockServer::start().await;

    // Get the algorithm name as a string and prepare the key
    let (test_key, jwks_key_json, alg_str) = match algorithm {
        Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512 => {
            // Create an RSA keypair for testing using AWS-LC
            // Generate a new RSA private key
            let private_key = signature::RsaKeyPair::generate(rsa::KeySize::Rsa2048).unwrap();

            // Get the public key
            let public_key = private_key.public_key();

            // Get PKCS8 DER format for the private key
            let private_key_pkcs8 = private_key.as_der().unwrap();
            // Get public key DER format
            // With aws-lc-rs 1.13: public_key.as_der().unwrap();
            let public_key_der = public_key;

            // Derive key ID from the public key
            let key_id = URL_SAFE_NO_PAD.encode(public_key_der.as_ref());

            // Extract modulus and exponent - we'll have to compute them from the DER format
            // For simplicity, let's just use these values directly
            let modulus = public_key.modulus().big_endian_without_leading_zero();
            let exponent = public_key.exponent().big_endian_without_leading_zero();

            let modulus_encoded = URL_SAFE_NO_PAD.encode(modulus);
            let exponent_encoded = URL_SAFE_NO_PAD.encode(exponent);

            let alg_str = match algorithm {
                Algorithm::RS256 => "RS256",
                Algorithm::RS384 => "RS384",
                Algorithm::RS512 => "RS512",
                Algorithm::PS256 => "PS256",
                Algorithm::PS384 => "PS384",
                Algorithm::PS512 => "PS512",
                _ => unreachable!(),
            };

            let jwks_key = json!({
                "kty": "RSA",
                "alg": alg_str,
                "use": "sig",
                "kid": key_id,
                "n": modulus_encoded,
                "e": exponent_encoded,
            });

            // Convert the private key DER to PEM format
            let private_key_pem = bytes_to_pem(
                private_key_pkcs8.as_ref(),
                "-----BEGIN PRIVATE KEY-----\n",
                "\n-----END PRIVATE KEY-----",
            );

            (private_key_pem, jwks_key, alg_str.to_string())
        }
        Algorithm::ES256 | Algorithm::ES384 => {
            // Choose the right signing algorithm for the algorithm
            let (signing_alg, curve_name) = match algorithm {
                Algorithm::ES256 => (&signature::ECDSA_P256_SHA256_ASN1_SIGNING, "P-256"),
                Algorithm::ES384 => (&signature::ECDSA_P384_SHA384_ASN1_SIGNING, "P-384"),
                _ => unreachable!(),
            };

            // Create EC keypair
            let rng = rand::SystemRandom::new();
            let pkcs8_bytes = signature::EcdsaKeyPair::generate_pkcs8(signing_alg, &rng).unwrap();

            // Get private key in PKCS8 format (already in the right format)
            let private_key_der = pkcs8_bytes.as_ref().to_vec();

            // Get the EC public key by creating a key pair from the pkcs8 document
            let key_pair =
                signature::EcdsaKeyPair::from_pkcs8(signing_alg, pkcs8_bytes.as_ref()).unwrap();
            let public_key_bytes = key_pair.public_key().as_ref();
            let key_id = URL_SAFE_NO_PAD.encode(public_key_bytes);

            // For ECDSA public keys, the byte format is:
            // - First byte is 0x04 (uncompressed point format)
            // - Next X bytes are X coordinate (32 bytes for P-256, 48 bytes for P-384)
            // - Next X bytes are Y coordinate (32 bytes for P-256, 48 bytes for P-384)
            let coordinate_size = match algorithm {
                Algorithm::ES256 => 32,
                Algorithm::ES384 => 48,
                _ => unreachable!(),
            };

            // Skip the first byte (0x04) and extract X and Y coordinates
            let x_coordinate = &public_key_bytes[1..(1 + coordinate_size)];
            let y_coordinate = &public_key_bytes[(1 + coordinate_size)..];

            // Convert coordinates to base64url
            let x_encoded = URL_SAFE_NO_PAD.encode(x_coordinate);
            let y_encoded = URL_SAFE_NO_PAD.encode(y_coordinate);

            let alg_str = match algorithm {
                Algorithm::ES256 => "ES256",
                Algorithm::ES384 => "ES384",
                _ => unreachable!(),
            };

            let jwks_key = json!({
                "kty": "EC",
                "alg": alg_str,
                "use": "sig",
                "kid": key_id,
                "crv": curve_name,
                "x": x_encoded,
                "y": y_encoded
            });

            // Convert the private key bytes to PEM format
            let private_key_pem = bytes_to_pem(
                &private_key_der,
                "-----BEGIN PRIVATE KEY-----\n",
                "\n-----END PRIVATE KEY-----",
            );

            (private_key_pem, jwks_key, alg_str.to_string())
        }
        Algorithm::EdDSA => {
            // Generate Ed25519 key
            let rng = rand::SystemRandom::new();
            let pkcs8 = signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
            let keypair = signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();

            // Get the public key bytes
            let public_key_bytes = keypair.public_key().as_ref();
            let key_id = URL_SAFE_NO_PAD.encode(public_key_bytes);

            // Ed25519 public key is already in the correct format - just encode it
            let x_encoded = URL_SAFE_NO_PAD.encode(public_key_bytes);

            let jwks_key = json!({
                "kty": "OKP",
                "alg": "EdDSA",
                "use": "sig",
                "kid": key_id,
                "crv": "Ed25519",
                "x": x_encoded
            });

            // Convert the private key bytes to PEM format
            let private_key_pem = bytes_to_pem(
                pkcs8.as_ref(),
                "-----BEGIN PRIVATE KEY-----\n",
                "\n-----END PRIVATE KEY-----",
            );

            (private_key_pem, jwks_key, "EdDSA".to_string())
        }
        _ => panic!("Unsupported algorithm for this test: {:?}", algorithm),
    };

    // Setup mock for OpenID discovery endpoint
    let jwks_uri = format!("{}/custom/path/to/jwks.json", mock_server.uri());
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": mock_server.uri(),
            "authorization_endpoint": format!("{}/auth", mock_server.uri()),
            "token_endpoint": format!("{}/oauth2/token", mock_server.uri()),
            "jwks_uri": jwks_uri,
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid", "profile", "email"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
            "claims_supported": ["sub", "iss", "aud", "exp", "iat"],
            "grant_types_supported": ["authorization_code", "client_credentials"]
        })))
        .mount(&mock_server)
        .await;

    // Setup mock for JWKS
    Mock::given(method("GET"))
        .and(path("/custom/path/to/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwks_key_json]
        })))
        .mount(&mock_server)
        .await;

    (test_key, mock_server, alg_str.to_string())
}

/// Test claims structure for JWT testing
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TestClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
}

impl TestClaims {
    /// Create new test claims with proper timestamps
    pub fn new(sub: impl Into<String>, iss: impl Into<String>, aud: impl Into<String>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            sub: sub.into(),
            iss: iss.into(),
            aud: aud.into(),
            exp: now + 3600, // Expires in 1 hour
            iat: now,
        }
    }
}

/// Generate a syntactically valid unsigned JWT for tests where signature verification is not needed.
/// The token will have HS256 header and include standard fields (iss,aud,sub,exp,iat,nbf).
/// This is useful for tests that exercise synchronous paths like `try_verify` where
/// we only need to parse claims and detect missing cached keys.
pub fn generate_test_token(issuer: &str, audience: &str, subject: &str) -> String {
    let header_json = r#"{"alg":"HS256","typ":"JWT"}"#;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();
    let claims_json = format!(
        "{{\"iss\":\"{}\",\"aud\":[\"{}\"],\"sub\":\"{}\",\"exp\":{},\"iat\":{},\"nbf\":{}}}",
        issuer,
        audience,
        subject,
        now + 300,
        now,
        now
    );
    format!(
        "{}.{}.sig",
        URL_SAFE_NO_PAD.encode(header_json.as_bytes()),
        URL_SAFE_NO_PAD.encode(claims_json.as_bytes())
    )
}

/// Setup a mock OIDC server for testing both token provider and verifier
pub async fn setup_oidc_mock_server() -> (MockServer, String, String) {
    let mock_server = MockServer::start().await;
    let issuer_url = mock_server.uri();

    // Mock OIDC discovery endpoint
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer_url,
            "authorization_endpoint": format!("{}/auth", issuer_url),
            "token_endpoint": format!("{}/oauth2/token", issuer_url),
            "jwks_uri": format!("{}/oauth2/jwks.json", issuer_url),
            "userinfo_endpoint": format!("{}/userinfo", issuer_url),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid", "profile", "email"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
            "claims_supported": ["sub", "iss", "aud", "exp", "iat"],
            "grant_types_supported": ["authorization_code", "client_credentials"]
        })))
        .mount(&mock_server)
        .await;

    // Mock token endpoint for client credentials
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "test-access-token-12345",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .mount(&mock_server)
        .await;

    // Mock JWKS endpoint (needed for token verification)
    Mock::given(method("GET"))
        .and(path("/oauth2/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [{
                "kty": "RSA",
                "kid": "test-key-1",
                "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbIS_4q3eFD1LTXfhHQjEZzXpk2zb7fF_xxGLmNr8zXczK8TGLLcgPYEEYnNJhJRs5vJ3dNb02f1_Q-sNHd8qXe5s7eXs2E4RbQJvQ",
                "e": "AQAB",
                "alg": "RS256"
            }]
        })))
        .mount(&mock_server)
        .await;

    let access_token = "test-access-token-12345".to_string();

    (mock_server, issuer_url, access_token)
}

/// Convert arbitrary bytes into a PEM-formatted string with the provided header/footer.
/// The body is wrapped at 64 character lines per RFC 7468 guidance.
/// Header/footer should include the BEGIN/END lines with trailing/leading newlines as desired.
fn bytes_to_pem(key_bytes: &[u8], header: &str, footer: &str) -> String {
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

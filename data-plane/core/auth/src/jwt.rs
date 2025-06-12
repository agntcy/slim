// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use jsonwebtoken_aws_lc::{
    Algorithm, DecodingKey, EncodingKey, Header as JwtHeader, TokenData, Validation, decode,
    decode_header, encode, errors::ErrorKind,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::builder::JwtBuilder;
use crate::errors::AuthError;
use crate::resolver::KeyResolver;
use crate::traits::{Signer, Verifier};

/// Standard JWT Claims structure that includes the registered claims
/// as specified in RFC 7519.
#[derive(Debug, Serialize, Deserialize)]
pub struct StandardClaims {
    /// Issuer (who issued the JWT)
    pub iss: String,

    /// Subject (whom the JWT is about)
    pub sub: String,

    /// Audience (who the JWT is intended for)
    pub aud: String,

    /// Expiration time (when the JWT expires)
    pub exp: u64,

    /// Issued at (when the JWT was issued)
    pub iat: u64,

    /// JWT ID (unique identifier for this JWT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,

    /// Not before (when the JWT starts being valid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,

    // Additional custom claims can be added by the user
    #[serde(flatten)]
    pub custom_claims: HashMap<String, serde_json::Value>,
}

/// JWT implementation that uses the jsonwebtoken crate.
pub struct Jwt {
    issuer: String,
    audience: String,
    subject: String,
    token_duration: Duration,
    validation: Validation,
    encoding_key: Option<EncodingKey>,
    decoding_key: Option<DecodingKey>,
    key_resolver: Option<KeyResolver>,
}

impl Jwt {
    /// Creates a new JwtBuilder to build a Jwt instance.
    pub fn builder() -> JwtBuilder<crate::builder::state::Initial> {
        JwtBuilder::new()
    }

    /// Internal constructor used by the builder.
    ///
    /// This should not be called directly. Use the builder pattern instead:
    /// ```
    /// let jwt = Jwt::builder()
    ///     .issuer("my-issuer")
    ///     .audience("my-audience")
    ///     .subject("user-123")
    ///     .private_key("secret-key")
    ///     .build()?;
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        issuer: String,
        audience: String,
        subject: String,
        token_duration: Duration,
        validation: Validation,
        encoding_key: Option<EncodingKey>,
        decoding_key: Option<DecodingKey>,
        key_resolver: Option<KeyResolver>,
    ) -> Self {
        Self {
            issuer,
            audience,
            subject,
            token_duration,
            validation,
            encoding_key,
            decoding_key,
            key_resolver,
        }
    }

    /// Creates a StandardClaims object with the default values.
    pub fn create_standard_claims(
        &self,
        custom_claims: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> StandardClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();

        let expiration = now + self.token_duration.as_secs();

        StandardClaims {
            iss: self.issuer.clone(),
            sub: self.subject.clone(),
            aud: self.audience.clone(),
            exp: expiration,
            iat: now,
            jti: None,
            nbf: Some(now),
            custom_claims: custom_claims.unwrap_or_default(),
        }
    }

    /// Resolve a decoding key for token verification
    async fn resolve_decoding_key(&mut self, token: &str) -> Result<DecodingKey, AuthError> {
        // First check if we already have a decoding key
        if let Some(key) = &self.decoding_key {
            return Ok(key.clone());
        }

        // As we don't have a decoding key, we need to resolve it. The resolver
        // should be set, if it's not we can't proceed.
        let resolver = self.key_resolver.as_mut().ok_or_else(|| {
            AuthError::ConfigError("Key resolver not configured for JWT verification".to_string())
        })?;

        // Parse the token header to get the key ID and algorithm
        let token_header = decode_header(token).map_err(|e| {
            AuthError::TokenInvalid(format!("Failed to decode token header: {}", e))
        })?;

        // Get the algorithm from the validation
        let algorithm = if !self.validation.algorithms.is_empty() {
            self.validation.algorithms[0]
        } else {
            // Default to RS256 if not specified
            Algorithm::RS256
        };

        // Resolve the key
        resolver
            .resolve_key(
                self.decoding_key.clone(),
                &self.issuer,
                algorithm,
                Some(&token_header),
                token,
            )
            .await
    }
}

impl Signer for Jwt {
    fn sign<Claims>(&self, claims: &Claims) -> Result<String, AuthError>
    where
        Claims: Serialize,
    {
        let encoding_key = self.encoding_key.as_ref().ok_or_else(|| {
            AuthError::ConfigError("Private key not configured for signing".to_string())
        })?;

        let header = JwtHeader::new(self.validation.algorithms[0]);

        encode(&header, claims, encoding_key).map_err(|e| AuthError::SigningError(format!("{}", e)))
    }
}

#[async_trait]
impl Verifier for Jwt {
    async fn verify<Claims>(&mut self, token: &str) -> Result<Claims, AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        let decoding_key = self.resolve_decoding_key(token).await?;

        let token_data: TokenData<Claims> = decode(token, &decoding_key, &self.validation)
            .map_err(|e| match e.kind() {
                ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::VerificationError(format!("{}", e)),
            })?;

        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken_aws_lc::Algorithm;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_jwt_sign_and_verify() {
        let jwt = Jwt::builder()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "secret-key")
            .unwrap()
            .build()
            .unwrap();

        let claims = jwt.create_standard_claims(None);
        let token = jwt.sign(&claims).unwrap();

        let mut verifier = jwt;
        let verified_claims: StandardClaims = verifier.verify(&token).await.unwrap();

        assert_eq!(verified_claims.iss, "test-issuer");
        assert_eq!(verified_claims.aud, "test-audience");
        assert_eq!(verified_claims.sub, "test-subject");

        // Try to verify with an invalid token
        let invalid_token = "invalid.token.string";
        let result: Result<StandardClaims, AuthError> = verifier.verify(invalid_token).await;
        assert!(
            result.is_err(),
            "Expected verification to fail for invalid token"
        );
    }

    #[tokio::test]
    async fn test_jwt_sign_and_verify_custom_claims() {
        let jwt = Jwt::builder()
            .issuer("test-issuer")
            .audience("test-audience")
            .subject("test-subject")
            .private_key(Algorithm::HS512, "secret-key")
            .unwrap()
            .build()
            .unwrap();

        let mut custom_claims = HashMap::new();
        custom_claims.insert(
            "role".to_string(),
            serde_json::Value::String("admin".to_string()),
        );
        custom_claims.insert(
            "permissions".to_string(),
            serde_json::Value::Array(vec![
                serde_json::Value::String("read".to_string()),
                serde_json::Value::String("write".to_string()),
            ]),
        );

        let claims = jwt.create_standard_claims(Some(custom_claims));
        let token = jwt.sign(&claims).unwrap();

        let mut verifier = jwt;
        let verified_claims: StandardClaims = verifier.verify(&token).await.unwrap();

        assert_eq!(verified_claims.custom_claims.get("role").unwrap(), "admin");
        assert_eq!(
            verified_claims.custom_claims.get("permissions").unwrap(),
            &serde_json::Value::Array(vec![
                serde_json::Value::String("read".to_string()),
                serde_json::Value::String("write".to_string())
            ])
        );
    } // Enum to represent different types of test keys
    enum TestKey {
        Rsa(openssl::rsa::Rsa<openssl::pkey::Private>),
        Ec(openssl::ec::EcKey<openssl::pkey::Private>),
        Ed25519(openssl::pkey::PKey<openssl::pkey::Private>),
    }

    impl TestKey {
        fn to_encoding_key(&self) -> EncodingKey {
            match self {
                TestKey::Rsa(rsa) => {
                    EncodingKey::from_rsa_der(rsa.private_key_to_der().unwrap().as_ref())
                }
                TestKey::Ec(ec) => {
                    let pkey =
                        openssl::pkey::PKey::<openssl::pkey::Private>::from_ec_key(ec.clone())
                            .unwrap();

                    EncodingKey::from_ec_der(pkey.private_key_to_pkcs8().unwrap().as_ref())
                }
                TestKey::Ed25519(ed) => {
                    EncodingKey::from_ed_der(ed.private_key_to_der().unwrap().as_ref())
                }
            }
        }
    }

    async fn setup_test_jwt_resolver(algorithm: Algorithm) -> (TestKey, MockServer, String) {
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
                // Create an RSA keypair for testing
                let keypair = openssl::rsa::Rsa::generate(2048).unwrap();

                // Get modulus and exponent
                let modulus = keypair.n();
                let exponent = keypair.e();

                // Create key ID and parameters
                let key_id = URL_SAFE_NO_PAD.encode(keypair.public_key_to_der().unwrap());
                let modulus_encoded = URL_SAFE_NO_PAD.encode(modulus.to_vec());
                let exponent_encoded = URL_SAFE_NO_PAD.encode(exponent.to_vec());

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

                (TestKey::Rsa(keypair), jwks_key, alg_str)
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                // Choose the right curve for the algorithm
                let nid = match algorithm {
                    Algorithm::ES256 => openssl::nid::Nid::X9_62_PRIME256V1, // P-256
                    Algorithm::ES384 => openssl::nid::Nid::SECP384R1,        // P-384
                    _ => unreachable!(),
                };

                // Create EC key with appropriate curve
                let ecgroup = openssl::ec::EcGroup::from_curve_name(nid).unwrap();
                let keypair = openssl::ec::EcKey::generate(&ecgroup).unwrap();

                // Get the EC public key in DER format
                let public_key_der = keypair.public_key_to_der().unwrap();
                let key_id = URL_SAFE_NO_PAD.encode(&public_key_der);

                // Extract x and y coordinates for JWKS using the BigNum API
                let ec_point = keypair.public_key();
                let mut bn_ctx = openssl::bn::BigNumContext::new().unwrap();
                let mut x = openssl::bn::BigNum::new().unwrap();
                let mut y = openssl::bn::BigNum::new().unwrap();

                // Get the affine coordinates
                ec_point
                    .affine_coordinates(&ecgroup, &mut x, &mut y, &mut bn_ctx)
                    .unwrap();

                // Convert coordinates to base64url
                let x_encoded = URL_SAFE_NO_PAD.encode(x.to_vec());
                let y_encoded = URL_SAFE_NO_PAD.encode(y.to_vec());

                // These variables are now defined above using the raw public key bytes

                let alg_str = match algorithm {
                    Algorithm::ES256 => "ES256",
                    Algorithm::ES384 => "ES384",
                    _ => unreachable!(),
                };

                let curve_name = match algorithm {
                    Algorithm::ES256 => "P-256",
                    Algorithm::ES384 => "P-384",
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

                (TestKey::Ec(keypair), jwks_key, alg_str)
            }
            Algorithm::EdDSA => {
                // Generate Ed25519 key
                let keypair = openssl::pkey::PKey::generate_ed25519().unwrap();

                // For EdDSA, let's use the public key in DER format
                let public_key_der = keypair.public_key_to_der().unwrap();
                let key_id = URL_SAFE_NO_PAD.encode(&public_key_der);

                // Extract the raw key bytes from DER format - the actual key is after the header
                // For simplicity, we'll use a fixed offset that works for Ed25519
                let x_encoded = URL_SAFE_NO_PAD.encode(&public_key_der[12..]);

                let jwks_key = json!({
                    "kty": "OKP",
                    "alg": "EdDSA",
                    "use": "sig",
                    "kid": key_id,
                    "crv": "Ed25519",
                    "x": x_encoded
                });

                (TestKey::Ed25519(keypair), jwks_key, "EdDSA")
            }
            _ => panic!("Unsupported algorithm for this test: {:?}", algorithm),
        };

        // Setup mock for OpenID discovery endpoint
        let jwks_uri = format!("{}/custom/path/to/jwks.json", mock_server.uri());
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": mock_server.uri(),
                "jwks_uri": jwks_uri
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

    async fn test_jwt_resolve_with_algorithm(algorithm: Algorithm) {
        let (test_key, mock_server, alg_str) = setup_test_jwt_resolver(algorithm).await;

        println!("Testing JWT key resolution with algorithm: {}", alg_str);

        // Build the JWT with auto key resolution
        let mut jwt = Jwt::builder()
            .issuer(mock_server.uri())
            .audience("test-audience")
            .subject("test-subject")
            .with_required_info()
            .unwrap()
            .auto_resolve_keys(true)
            .build()
            .unwrap();

        // Setup the JWT for signing (normally this would be a different instance)
        jwt.encoding_key = Some(test_key.to_encoding_key());
        jwt.validation = Validation::new(algorithm);
        jwt.validation.set_audience(&[&jwt.audience]);
        jwt.validation.set_issuer(&[&jwt.issuer]);

        // Sign and verify the token
        let token = jwt.sign(&jwt.create_standard_claims(None)).unwrap();
        let claims: StandardClaims = jwt.verify(&token).await.unwrap();

        // Validate the claims
        assert_eq!(claims.iss, jwt.issuer);
        assert_eq!(claims.aud, jwt.audience);
        assert_eq!(claims.sub, jwt.subject);
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs256() {
        test_jwt_resolve_with_algorithm(Algorithm::RS256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs384() {
        test_jwt_resolve_with_algorithm(Algorithm::RS384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_rs512() {
        test_jwt_resolve_with_algorithm(Algorithm::RS512).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps256() {
        test_jwt_resolve_with_algorithm(Algorithm::PS256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps384() {
        test_jwt_resolve_with_algorithm(Algorithm::PS384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_ps512() {
        test_jwt_resolve_with_algorithm(Algorithm::PS512).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_es256() {
        test_jwt_resolve_with_algorithm(Algorithm::ES256).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_es384() {
        test_jwt_resolve_with_algorithm(Algorithm::ES384).await;
    }

    #[tokio::test]
    async fn test_jwt_resolve_decoding_key_eddsa() {
        test_jwt_resolve_with_algorithm(Algorithm::EdDSA).await;
    }
}

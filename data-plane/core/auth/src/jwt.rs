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
    use jsonwebtoken_aws_lc::Algorithm;

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
    }
}

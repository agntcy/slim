// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_config::auth::identity::{
    IdentityProviderConfig as CoreIdentityProviderConfig,
    IdentityVerifierConfig as CoreIdentityVerifierConfig,
};
use slim_config::auth::jwt::Config as JwtAuthConfig;
use slim_config::auth::jwt::{Claims as JwtClaims, JwtKey};
use slim_config::auth::static_jwt::Config as StaticJwtConfig;
use std::time::Duration;

#[cfg_attr(target_family = "windows", allow(unused_imports))]
use crate::common_config::SpireConfig;
use crate::errors::SlimError;

/// Static JWT (Bearer token) authentication configuration
/// The token is loaded from a file and automatically reloaded when changed
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct StaticJwtAuth {
    /// Path to file containing the JWT token
    pub token_file: String,
    /// Duration for caching the token before re-reading from file (default: 3600 seconds)
    pub duration: Duration,
}

impl From<StaticJwtAuth> for StaticJwtConfig {
    fn from(config: StaticJwtAuth) -> Self {
        StaticJwtConfig::with_file(&config.token_file).with_duration(config.duration)
    }
}

impl From<StaticJwtConfig> for StaticJwtAuth {
    fn from(config: StaticJwtConfig) -> Self {
        StaticJwtAuth {
            token_file: config.source().file.clone(),
            duration: config.duration(),
        }
    }
}

/// JWT signing/verification algorithm
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum JwtAlgorithm {
    HS256,
    HS384,
    HS512,
    ES256,
    ES384,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    EdDSA,
}

impl From<JwtAlgorithm> for slim_auth::jwt::Algorithm {
    fn from(algo: JwtAlgorithm) -> Self {
        match algo {
            JwtAlgorithm::HS256 => slim_auth::jwt::Algorithm::HS256,
            JwtAlgorithm::HS384 => slim_auth::jwt::Algorithm::HS384,
            JwtAlgorithm::HS512 => slim_auth::jwt::Algorithm::HS512,
            JwtAlgorithm::ES256 => slim_auth::jwt::Algorithm::ES256,
            JwtAlgorithm::ES384 => slim_auth::jwt::Algorithm::ES384,
            JwtAlgorithm::RS256 => slim_auth::jwt::Algorithm::RS256,
            JwtAlgorithm::RS384 => slim_auth::jwt::Algorithm::RS384,
            JwtAlgorithm::RS512 => slim_auth::jwt::Algorithm::RS512,
            JwtAlgorithm::PS256 => slim_auth::jwt::Algorithm::PS256,
            JwtAlgorithm::PS384 => slim_auth::jwt::Algorithm::PS384,
            JwtAlgorithm::PS512 => slim_auth::jwt::Algorithm::PS512,
            JwtAlgorithm::EdDSA => slim_auth::jwt::Algorithm::EdDSA,
        }
    }
}

impl From<slim_auth::jwt::Algorithm> for JwtAlgorithm {
    fn from(algo: slim_auth::jwt::Algorithm) -> Self {
        match algo {
            slim_auth::jwt::Algorithm::HS256 => JwtAlgorithm::HS256,
            slim_auth::jwt::Algorithm::HS384 => JwtAlgorithm::HS384,
            slim_auth::jwt::Algorithm::HS512 => JwtAlgorithm::HS512,
            slim_auth::jwt::Algorithm::ES256 => JwtAlgorithm::ES256,
            slim_auth::jwt::Algorithm::ES384 => JwtAlgorithm::ES384,
            slim_auth::jwt::Algorithm::RS256 => JwtAlgorithm::RS256,
            slim_auth::jwt::Algorithm::RS384 => JwtAlgorithm::RS384,
            slim_auth::jwt::Algorithm::RS512 => JwtAlgorithm::RS512,
            slim_auth::jwt::Algorithm::PS256 => JwtAlgorithm::PS256,
            slim_auth::jwt::Algorithm::PS384 => JwtAlgorithm::PS384,
            slim_auth::jwt::Algorithm::PS512 => JwtAlgorithm::PS512,
            slim_auth::jwt::Algorithm::EdDSA => JwtAlgorithm::EdDSA,
        }
    }
}

/// JWT key format
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum JwtKeyFormat {
    Pem,
    Jwk,
    Jwks,
}

impl From<JwtKeyFormat> for slim_auth::jwt::KeyFormat {
    fn from(format: JwtKeyFormat) -> Self {
        match format {
            JwtKeyFormat::Pem => slim_auth::jwt::KeyFormat::Pem,
            JwtKeyFormat::Jwk => slim_auth::jwt::KeyFormat::Jwk,
            JwtKeyFormat::Jwks => slim_auth::jwt::KeyFormat::Jwks,
        }
    }
}

impl From<slim_auth::jwt::KeyFormat> for JwtKeyFormat {
    fn from(format: slim_auth::jwt::KeyFormat) -> Self {
        match format {
            slim_auth::jwt::KeyFormat::Pem => JwtKeyFormat::Pem,
            slim_auth::jwt::KeyFormat::Jwk => JwtKeyFormat::Jwk,
            slim_auth::jwt::KeyFormat::Jwks => JwtKeyFormat::Jwks,
        }
    }
}

/// JWT key data source
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum JwtKeyData {
    /// String with encoded key(s)
    Data { value: String },
    /// File path to the key(s)
    File { path: String },
}

impl From<JwtKeyData> for slim_auth::jwt::KeyData {
    fn from(data: JwtKeyData) -> Self {
        match data {
            JwtKeyData::Data { value } => slim_auth::jwt::KeyData::Data(value),
            JwtKeyData::File { path } => slim_auth::jwt::KeyData::File(path),
        }
    }
}

impl From<slim_auth::jwt::KeyData> for JwtKeyData {
    fn from(data: slim_auth::jwt::KeyData) -> Self {
        match data {
            slim_auth::jwt::KeyData::Data(value) => JwtKeyData::Data { value },
            slim_auth::jwt::KeyData::File(path) => JwtKeyData::File { path },
        }
    }
}

/// JWT key configuration
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct JwtKeyConfig {
    /// Algorithm used for signing/verifying the JWT
    pub algorithm: JwtAlgorithm,
    /// Key format - PEM, JWK or JWKS
    pub format: JwtKeyFormat,
    /// Encoded key or file path
    pub key: JwtKeyData,
}

impl From<JwtKeyConfig> for slim_auth::jwt::Key {
    fn from(config: JwtKeyConfig) -> Self {
        slim_auth::jwt::Key {
            algorithm: config.algorithm.into(),
            format: config.format.into(),
            key: config.key.into(),
        }
    }
}

impl From<slim_auth::jwt::Key> for JwtKeyConfig {
    fn from(key: slim_auth::jwt::Key) -> Self {
        JwtKeyConfig {
            algorithm: key.algorithm.into(),
            format: key.format.into(),
            key: key.key.into(),
        }
    }
}

/// JWT key type (encoding, decoding, or autoresolve)
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum JwtKeyType {
    /// Encoding key for signing JWTs (client-side)
    Encoding { key: JwtKeyConfig },
    /// Decoding key for verifying JWTs (server-side)
    Decoding { key: JwtKeyConfig },
    /// Automatically resolve keys based on claims
    Autoresolve,
}

impl From<JwtKeyType> for JwtKey {
    fn from(key_type: JwtKeyType) -> Self {
        match key_type {
            JwtKeyType::Encoding { key } => JwtKey::Encoding(key.into()),
            JwtKeyType::Decoding { key } => JwtKey::Decoding(key.into()),
            JwtKeyType::Autoresolve => JwtKey::Autoresolve,
        }
    }
}

impl From<JwtKey> for JwtKeyType {
    fn from(key: JwtKey) -> Self {
        match key {
            JwtKey::Encoding(k) => JwtKeyType::Encoding { key: k.into() },
            JwtKey::Decoding(k) => JwtKeyType::Decoding { key: k.into() },
            JwtKey::Autoresolve => JwtKeyType::Autoresolve,
        }
    }
}

/// JWT authentication configuration for client-side signing
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ClientJwtAuth {
    /// JWT key configuration (encoding key for signing)
    pub key: JwtKeyType,
    /// JWT audience claims to include
    pub audience: Option<Vec<String>>,
    /// JWT issuer to include
    pub issuer: Option<String>,
    /// JWT subject to include
    pub subject: Option<String>,
    /// Token validity duration (default: 3600 seconds)
    pub duration: Duration,
}

impl From<ClientJwtAuth> for JwtAuthConfig {
    fn from(config: ClientJwtAuth) -> Self {
        let mut claims = JwtClaims::default();

        if let Some(audience) = config.audience {
            claims = claims.with_audience(&audience);
        }

        if let Some(issuer) = config.issuer {
            claims = claims.with_issuer(issuer);
        }

        if let Some(subject) = config.subject {
            claims = claims.with_subject(subject);
        }

        JwtAuthConfig::new(claims, config.duration, config.key.into())
    }
}

impl From<JwtAuthConfig> for ClientJwtAuth {
    fn from(config: JwtAuthConfig) -> Self {
        let claims = config.claims();
        ClientJwtAuth {
            key: config.key().clone().into(),
            audience: claims.audience().clone(),
            issuer: claims.issuer().clone(),
            subject: claims.subject().clone(),
            duration: config.duration(),
        }
    }
}

/// JWT authentication configuration for server-side verification
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct JwtAuth {
    /// JWT key configuration (decoding key for verification)
    pub key: JwtKeyType,
    /// JWT audience claims to verify
    pub audience: Option<Vec<String>>,
    /// JWT issuer to verify
    pub issuer: Option<String>,
    /// JWT subject to verify
    pub subject: Option<String>,
    /// Token validity duration (default: 3600 seconds)
    pub duration: Duration,
}

impl From<JwtAuth> for JwtAuthConfig {
    fn from(config: JwtAuth) -> Self {
        let mut claims = JwtClaims::default();

        if let Some(audience) = config.audience {
            claims = claims.with_audience(&audience);
        }

        if let Some(issuer) = config.issuer {
            claims = claims.with_issuer(issuer);
        }

        if let Some(subject) = config.subject {
            claims = claims.with_subject(subject);
        }

        JwtAuthConfig::new(claims, config.duration, config.key.into())
    }
}

impl From<JwtAuthConfig> for JwtAuth {
    fn from(config: JwtAuthConfig) -> Self {
        let claims = config.claims();
        JwtAuth {
            key: config.key().clone().into(),
            audience: claims.audience().clone(),
            issuer: claims.issuer().clone(),
            subject: claims.subject().clone(),
            duration: config.duration(),
        }
    }
}

/// Identity provider configuration - used to prove identity to others
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum IdentityProviderConfig {
    /// Shared secret authentication (symmetric key)
    SharedSecret { id: String, data: String },
    /// Static JWT loaded from file with auto-reload
    StaticJwt { config: StaticJwtAuth },
    /// Dynamic JWT generation with signing key
    Jwt { config: ClientJwtAuth },
    /// SPIRE-based identity provider (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    Spire { config: SpireConfig },
    /// No identity provider configured
    None,
}

impl From<IdentityProviderConfig> for CoreIdentityProviderConfig {
    fn from(config: IdentityProviderConfig) -> Self {
        match config {
            IdentityProviderConfig::SharedSecret { id, data } => {
                CoreIdentityProviderConfig::SharedSecret { id, data }
            }
            IdentityProviderConfig::StaticJwt { config } => {
                CoreIdentityProviderConfig::StaticJwt(config.into())
            }
            IdentityProviderConfig::Jwt { config } => {
                CoreIdentityProviderConfig::Jwt(config.into())
            }
            #[cfg(not(target_family = "windows"))]
            IdentityProviderConfig::Spire { config } => {
                CoreIdentityProviderConfig::Spire(config.into())
            }
            IdentityProviderConfig::None => CoreIdentityProviderConfig::None,
        }
    }
}

impl From<CoreIdentityProviderConfig> for IdentityProviderConfig {
    fn from(config: CoreIdentityProviderConfig) -> Self {
        match config {
            CoreIdentityProviderConfig::SharedSecret { id, data } => {
                IdentityProviderConfig::SharedSecret { id, data }
            }
            CoreIdentityProviderConfig::StaticJwt(config) => IdentityProviderConfig::StaticJwt {
                config: config.into(),
            },
            CoreIdentityProviderConfig::Jwt(config) => IdentityProviderConfig::Jwt {
                config: config.into(),
            },
            #[cfg(not(target_family = "windows"))]
            CoreIdentityProviderConfig::Spire(config) => IdentityProviderConfig::Spire {
                config: config.into(),
            },
            CoreIdentityProviderConfig::None => IdentityProviderConfig::None,
        }
    }
}

/// Identity verifier configuration - used to verify identity of others
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum IdentityVerifierConfig {
    /// Shared secret verification (symmetric key)
    SharedSecret { id: String, data: String },
    /// JWT verification with decoding key
    Jwt { config: JwtAuth },
    /// SPIRE-based identity verifier (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    Spire { config: SpireConfig },
    /// No identity verifier configured
    None,
}

impl From<IdentityVerifierConfig> for CoreIdentityVerifierConfig {
    fn from(config: IdentityVerifierConfig) -> Self {
        match config {
            IdentityVerifierConfig::SharedSecret { id, data } => {
                CoreIdentityVerifierConfig::SharedSecret { id, data }
            }
            IdentityVerifierConfig::Jwt { config } => {
                CoreIdentityVerifierConfig::Jwt(config.into())
            }
            #[cfg(not(target_family = "windows"))]
            IdentityVerifierConfig::Spire { config } => {
                CoreIdentityVerifierConfig::Spire(config.into())
            }
            IdentityVerifierConfig::None => CoreIdentityVerifierConfig::None,
        }
    }
}

impl From<CoreIdentityVerifierConfig> for IdentityVerifierConfig {
    fn from(config: CoreIdentityVerifierConfig) -> Self {
        match config {
            CoreIdentityVerifierConfig::SharedSecret { id, data } => {
                IdentityVerifierConfig::SharedSecret { id, data }
            }
            CoreIdentityVerifierConfig::Jwt(config) => IdentityVerifierConfig::Jwt {
                config: config.into(),
            },
            #[cfg(not(target_family = "windows"))]
            CoreIdentityVerifierConfig::Spire(config) => IdentityVerifierConfig::Spire {
                config: config.into(),
            },
            CoreIdentityVerifierConfig::None => IdentityVerifierConfig::None,
        }
    }
}

impl TryFrom<IdentityProviderConfig> for AuthProvider {
    type Error = SlimError;

    fn try_from(config: IdentityProviderConfig) -> Result<Self, Self::Error> {
        match config {
            IdentityProviderConfig::SharedSecret { id, data } => {
                AuthProvider::shared_secret_from_str(&id, &data).map_err(|e| {
                    SlimError::InvalidArgument {
                        message: format!("Failed to create SharedSecret provider: {}", e),
                    }
                })
            }
            IdentityProviderConfig::StaticJwt { config } => {
                let core_config: StaticJwtConfig = config.into();
                let provider = core_config.build_static_token_provider().map_err(|e| {
                    SlimError::InvalidArgument {
                        message: format!("Failed to build StaticTokenProvider: {}", e),
                    }
                })?;
                Ok(AuthProvider::static_token(provider))
            }
            IdentityProviderConfig::Jwt { config } => {
                let core_config: JwtAuthConfig = config.into();
                let provider =
                    core_config
                        .get_provider()
                        .map_err(|e| SlimError::InvalidArgument {
                            message: format!("Failed to build JWT provider: {}", e),
                        })?;
                Ok(AuthProvider::jwt_signer(provider))
            }
            #[cfg(not(target_family = "windows"))]
            IdentityProviderConfig::Spire { config } => {
                let mut builder = slim_auth::spire::SpireIdentityManager::builder();

                if let Some(socket_path) = config.socket_path {
                    builder = builder.with_socket_path(socket_path);
                }

                if let Some(target_spiffe_id) = config.target_spiffe_id {
                    builder = builder.with_target_spiffe_id(target_spiffe_id);
                }

                if !config.jwt_audiences.is_empty() {
                    builder = builder.with_jwt_audiences(config.jwt_audiences);
                }

                let manager = builder.build();
                Ok(AuthProvider::spire(manager))
            }
            IdentityProviderConfig::None => Err(SlimError::InvalidArgument {
                message: "Cannot create AuthProvider from None variant".to_string(),
            }),
        }
    }
}

impl TryFrom<IdentityVerifierConfig> for AuthVerifier {
    type Error = SlimError;

    fn try_from(config: IdentityVerifierConfig) -> Result<Self, Self::Error> {
        match config {
            IdentityVerifierConfig::SharedSecret { id, data } => {
                AuthVerifier::shared_secret_from_str(&id, &data).map_err(|e| {
                    SlimError::InvalidArgument {
                        message: format!("Failed to create SharedSecret verifier: {}", e),
                    }
                })
            }
            IdentityVerifierConfig::Jwt { config } => {
                let core_config: JwtAuthConfig = config.into();
                let verifier =
                    core_config
                        .get_verifier()
                        .map_err(|e| SlimError::InvalidArgument {
                            message: format!("Failed to build JWT verifier: {}", e),
                        })?;
                Ok(AuthVerifier::jwt_verifier(verifier))
            }
            #[cfg(not(target_family = "windows"))]
            IdentityVerifierConfig::Spire { config } => {
                let mut builder = slim_auth::spire::SpireIdentityManager::builder();

                if let Some(socket_path) = config.socket_path {
                    builder = builder.with_socket_path(socket_path);
                }

                if let Some(target_spiffe_id) = config.target_spiffe_id {
                    builder = builder.with_target_spiffe_id(target_spiffe_id);
                }

                if !config.jwt_audiences.is_empty() {
                    builder = builder.with_jwt_audiences(config.jwt_audiences);
                }

                let manager = builder.build();
                Ok(AuthVerifier::spire(manager))
            }
            IdentityVerifierConfig::None => Err(SlimError::InvalidArgument {
                message: "Cannot create AuthVerifier from None variant".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Test StaticJwtAuth conversions
    #[test]
    fn test_static_jwt_auth_conversion() {
        let auth = StaticJwtAuth {
            token_file: "/path/to/token.jwt".to_string(),
            duration: Duration::from_secs(7200),
        };

        let core_config: StaticJwtConfig = auth.clone().into();
        assert_eq!(core_config.source().file, auth.token_file);
        assert_eq!(core_config.duration(), auth.duration);

        let back_to_auth: StaticJwtAuth = core_config.into();
        assert_eq!(back_to_auth, auth);
    }

    // Test JwtAlgorithm conversions
    #[test]
    fn test_jwt_algorithm_conversions() {
        let algorithms = vec![
            JwtAlgorithm::HS256,
            JwtAlgorithm::HS384,
            JwtAlgorithm::HS512,
            JwtAlgorithm::ES256,
            JwtAlgorithm::ES384,
            JwtAlgorithm::RS256,
            JwtAlgorithm::RS384,
            JwtAlgorithm::RS512,
            JwtAlgorithm::PS256,
            JwtAlgorithm::PS384,
            JwtAlgorithm::PS512,
            JwtAlgorithm::EdDSA,
        ];

        for algo in algorithms {
            let core_algo: slim_auth::jwt::Algorithm = algo.clone().into();
            let back: JwtAlgorithm = core_algo.into();
            assert_eq!(back, algo);
        }
    }

    // Test JwtKeyFormat conversions
    #[test]
    fn test_jwt_key_format_conversions() {
        let formats = vec![JwtKeyFormat::Pem, JwtKeyFormat::Jwk, JwtKeyFormat::Jwks];

        for format in formats {
            let core_format: slim_auth::jwt::KeyFormat = format.clone().into();
            let back: JwtKeyFormat = core_format.into();
            assert_eq!(back, format);
        }
    }

    // Test JwtKeyData conversions
    #[test]
    fn test_jwt_key_data_conversions() {
        let data_value = JwtKeyData::Data {
            value: "test-key-data".to_string(),
        };
        let core_data: slim_auth::jwt::KeyData = data_value.clone().into();
        let back: JwtKeyData = core_data.into();
        assert_eq!(back, data_value);

        let file_path = JwtKeyData::File {
            path: "/path/to/key.pem".to_string(),
        };
        let core_file: slim_auth::jwt::KeyData = file_path.clone().into();
        let back_file: JwtKeyData = core_file.into();
        assert_eq!(back_file, file_path);
    }

    // Test JwtKeyConfig conversions
    #[test]
    fn test_jwt_key_config_conversions() {
        let key_config = JwtKeyConfig {
            algorithm: JwtAlgorithm::RS256,
            format: JwtKeyFormat::Pem,
            key: JwtKeyData::File {
                path: "/path/to/key.pem".to_string(),
            },
        };

        let core_key: slim_auth::jwt::Key = key_config.clone().into();
        let back: JwtKeyConfig = core_key.into();
        assert_eq!(back, key_config);
    }

    // Test JwtKeyType conversions
    #[test]
    fn test_jwt_key_type_conversions() {
        let encoding_key = JwtKeyType::Encoding {
            key: JwtKeyConfig {
                algorithm: JwtAlgorithm::RS256,
                format: JwtKeyFormat::Pem,
                key: JwtKeyData::Data {
                    value: "encoding-key".to_string(),
                },
            },
        };

        let core_encoding: JwtKey = encoding_key.clone().into();
        let back_encoding: JwtKeyType = core_encoding.into();
        assert_eq!(back_encoding, encoding_key);

        let decoding_key = JwtKeyType::Decoding {
            key: JwtKeyConfig {
                algorithm: JwtAlgorithm::ES256,
                format: JwtKeyFormat::Jwk,
                key: JwtKeyData::File {
                    path: "/path/to/key.jwk".to_string(),
                },
            },
        };

        let core_decoding: JwtKey = decoding_key.clone().into();
        let back_decoding: JwtKeyType = core_decoding.into();
        assert_eq!(back_decoding, decoding_key);

        let autoresolve = JwtKeyType::Autoresolve;
        let core_auto: JwtKey = autoresolve.clone().into();
        let back_auto: JwtKeyType = core_auto.into();
        assert_eq!(back_auto, autoresolve);
    }

    // Test ClientJwtAuth conversions
    #[test]
    fn test_client_jwt_auth_conversions() {
        let client_auth = ClientJwtAuth {
            key: JwtKeyType::Autoresolve,
            audience: Some(vec!["api.example.com".to_string()]),
            issuer: Some("auth.example.com".to_string()),
            subject: Some("user@example.com".to_string()),
            duration: Duration::from_secs(1800),
        };

        let core_config: JwtAuthConfig = client_auth.clone().into();
        assert_eq!(core_config.duration(), client_auth.duration);

        let back: ClientJwtAuth = core_config.into();
        assert_eq!(back, client_auth);
    }

    // Test ClientJwtAuth with None fields
    #[test]
    fn test_client_jwt_auth_with_none_fields() {
        let client_auth = ClientJwtAuth {
            key: JwtKeyType::Autoresolve,
            audience: None,
            issuer: None,
            subject: None,
            duration: Duration::from_secs(3600),
        };

        let core_config: JwtAuthConfig = client_auth.clone().into();
        let back: ClientJwtAuth = core_config.into();
        assert_eq!(back.audience, None);
        assert_eq!(back.issuer, None);
        assert_eq!(back.subject, None);
    }

    // Test JwtAuth conversions
    #[test]
    fn test_jwt_auth_conversions() {
        let jwt_auth = JwtAuth {
            key: JwtKeyType::Decoding {
                key: JwtKeyConfig {
                    algorithm: JwtAlgorithm::RS256,
                    format: JwtKeyFormat::Pem,
                    key: JwtKeyData::File {
                        path: "/path/to/key.pem".to_string(),
                    },
                },
            },
            audience: Some(vec!["service.example.com".to_string()]),
            issuer: Some("issuer.example.com".to_string()),
            subject: Some("subject".to_string()),
            duration: Duration::from_secs(900),
        };

        let core_config: JwtAuthConfig = jwt_auth.clone().into();
        let back: JwtAuth = core_config.into();
        assert_eq!(back, jwt_auth);
    }

    // Test IdentityProviderConfig conversions
    #[test]
    fn test_identity_provider_config_shared_secret() {
        let config = IdentityProviderConfig::SharedSecret {
            id: "test-id".to_string(),
            data: "secret-data".to_string(),
        };

        let core_config: CoreIdentityProviderConfig = config.clone().into();
        let back: IdentityProviderConfig = core_config.into();
        assert_eq!(back, config);
    }

    #[test]
    fn test_identity_provider_config_static_jwt() {
        let config = IdentityProviderConfig::StaticJwt {
            config: StaticJwtAuth {
                token_file: "/path/to/token.jwt".to_string(),
                duration: Duration::from_secs(3600),
            },
        };

        let core_config: CoreIdentityProviderConfig = config.clone().into();
        let back: IdentityProviderConfig = core_config.into();
        assert_eq!(back, config);
    }

    #[test]
    fn test_identity_provider_config_jwt() {
        let config = IdentityProviderConfig::Jwt {
            config: ClientJwtAuth {
                key: JwtKeyType::Autoresolve,
                audience: Some(vec!["test".to_string()]),
                issuer: None,
                subject: None,
                duration: Duration::from_secs(3600),
            },
        };

        let core_config: CoreIdentityProviderConfig = config.clone().into();
        let back: IdentityProviderConfig = core_config.into();
        assert_eq!(back, config);
    }

    #[test]
    fn test_identity_provider_config_none() {
        let config = IdentityProviderConfig::None;
        let core_config: CoreIdentityProviderConfig = config.clone().into();
        let back: IdentityProviderConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test IdentityVerifierConfig conversions
    #[test]
    fn test_identity_verifier_config_shared_secret() {
        let config = IdentityVerifierConfig::SharedSecret {
            id: "verifier-id".to_string(),
            data: "verifier-secret".to_string(),
        };

        let core_config: CoreIdentityVerifierConfig = config.clone().into();
        let back: IdentityVerifierConfig = core_config.into();
        assert_eq!(back, config);
    }

    #[test]
    fn test_identity_verifier_config_jwt() {
        let config = IdentityVerifierConfig::Jwt {
            config: JwtAuth {
                key: JwtKeyType::Autoresolve,
                audience: Some(vec!["verifier".to_string()]),
                issuer: Some("auth".to_string()),
                subject: None,
                duration: Duration::from_secs(7200),
            },
        };

        let core_config: CoreIdentityVerifierConfig = config.clone().into();
        let back: IdentityVerifierConfig = core_config.into();
        assert_eq!(back, config);
    }

    #[test]
    fn test_identity_verifier_config_none() {
        let config = IdentityVerifierConfig::None;
        let core_config: CoreIdentityVerifierConfig = config.clone().into();
        let back: IdentityVerifierConfig = core_config.into();
        assert_eq!(back, config);
    }

    // Test TryFrom for IdentityProviderConfig to AuthProvider
    #[test]
    fn test_identity_provider_to_auth_provider_shared_secret() {
        let config = IdentityProviderConfig::SharedSecret {
            id: "test-id".to_string(),
            data: "shared-secret-value-0123456789abcdef".to_string(), // Must be 32+ chars
        };

        let result: Result<AuthProvider, SlimError> = config.try_into();
        assert!(result.is_ok());
    }

    #[test]
    fn test_identity_provider_to_auth_provider_none_fails() {
        let config = IdentityProviderConfig::None;
        let result: Result<AuthProvider, SlimError> = config.try_into();
        assert!(result.is_err());
        if let Err(SlimError::InvalidArgument { message }) = result {
            assert!(message.contains("Cannot create AuthProvider from None variant"));
        }
    }

    // Test TryFrom for IdentityVerifierConfig to AuthVerifier
    #[test]
    fn test_identity_verifier_to_auth_verifier_shared_secret() {
        let config = IdentityVerifierConfig::SharedSecret {
            id: "verifier-id".to_string(),
            data: "verifier-shared-secret-0123456789abcdef".to_string(), // Must be 32+ chars
        };

        let result: Result<AuthVerifier, SlimError> = config.try_into();
        assert!(result.is_ok());
    }

    #[test]
    fn test_identity_verifier_to_auth_verifier_none_fails() {
        let config = IdentityVerifierConfig::None;
        let result: Result<AuthVerifier, SlimError> = config.try_into();
        assert!(result.is_err());
        if let Err(SlimError::InvalidArgument { message }) = result {
            assert!(message.contains("Cannot create AuthVerifier from None variant"));
        }
    }

    // Test invalid shared secret (too short)
    #[test]
    fn test_identity_provider_invalid_shared_secret() {
        let config = IdentityProviderConfig::SharedSecret {
            id: "test-id".to_string(),
            data: "tooshort".to_string(), // Must be at least 32 characters
        };

        let result: Result<AuthProvider, SlimError> = config.try_into();
        assert!(result.is_err());
        if let Err(SlimError::InvalidArgument { message }) = result {
            assert!(message.contains("Failed to create SharedSecret provider"));
        }
    }

    #[test]
    fn test_identity_verifier_invalid_shared_secret() {
        let config = IdentityVerifierConfig::SharedSecret {
            id: "test-id".to_string(),
            data: "tooshort".to_string(), // Too short - must be 32+ chars
        };

        let result: Result<AuthVerifier, SlimError> = config.try_into();
        assert!(result.is_err());
        if let Err(SlimError::InvalidArgument { message }) = result {
            assert!(message.contains("Failed to create SharedSecret verifier"));
        }
    }

    // Test SPIRE config conversions (non-Windows only)
    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_identity_provider_config_spire() {
        let spire_config = SpireConfig {
            socket_path: Some("/tmp/spire.sock".to_string()),
            target_spiffe_id: Some("spiffe://example.com/service".to_string()),
            jwt_audiences: vec!["audience1".to_string(), "audience2".to_string()],
            trust_domains: vec![],
        };

        let config = IdentityProviderConfig::Spire {
            config: spire_config.clone(),
        };

        let core_config: CoreIdentityProviderConfig = config.clone().into();
        let back: IdentityProviderConfig = core_config.into();

        if let IdentityProviderConfig::Spire {
            config: back_config,
        } = back
        {
            assert_eq!(back_config.socket_path, spire_config.socket_path);
            assert_eq!(back_config.target_spiffe_id, spire_config.target_spiffe_id);
            assert_eq!(back_config.jwt_audiences, spire_config.jwt_audiences);
        } else {
            panic!("Expected Spire variant");
        }
    }

    #[cfg(not(target_family = "windows"))]
    #[test]
    fn test_identity_verifier_config_spire() {
        let spire_config = SpireConfig {
            socket_path: None,
            target_spiffe_id: None,
            jwt_audiences: vec!["slim".to_string()],
            trust_domains: vec!["example.com".to_string()],
        };

        let config = IdentityVerifierConfig::Spire {
            config: spire_config.clone(),
        };

        let core_config: CoreIdentityVerifierConfig = config.clone().into();
        let back: IdentityVerifierConfig = core_config.into();

        if let IdentityVerifierConfig::Spire {
            config: back_config,
        } = back
        {
            assert_eq!(back_config.socket_path, spire_config.socket_path);
            assert_eq!(back_config.jwt_audiences, spire_config.jwt_audiences);
        } else {
            panic!("Expected Spire variant");
        }
    }

    // Test complex JWT key configurations
    #[test]
    fn test_complex_jwt_key_configurations() {
        let configurations = vec![
            JwtKeyConfig {
                algorithm: JwtAlgorithm::HS256,
                format: JwtKeyFormat::Pem,
                key: JwtKeyData::Data {
                    value: "secret-key".to_string(),
                },
            },
            JwtKeyConfig {
                algorithm: JwtAlgorithm::ES384,
                format: JwtKeyFormat::Jwk,
                key: JwtKeyData::File {
                    path: "/etc/keys/ec-key.jwk".to_string(),
                },
            },
            JwtKeyConfig {
                algorithm: JwtAlgorithm::PS512,
                format: JwtKeyFormat::Jwks,
                key: JwtKeyData::File {
                    path: "/etc/keys/jwks.json".to_string(),
                },
            },
        ];

        for config in configurations {
            let core_key: slim_auth::jwt::Key = config.clone().into();
            let back: JwtKeyConfig = core_key.into();
            assert_eq!(back, config);
        }
    }

    // Test all JWT algorithms are covered
    #[test]
    fn test_all_jwt_algorithms_covered() {
        let all_algorithms = vec![
            (JwtAlgorithm::HS256, slim_auth::jwt::Algorithm::HS256),
            (JwtAlgorithm::HS384, slim_auth::jwt::Algorithm::HS384),
            (JwtAlgorithm::HS512, slim_auth::jwt::Algorithm::HS512),
            (JwtAlgorithm::ES256, slim_auth::jwt::Algorithm::ES256),
            (JwtAlgorithm::ES384, slim_auth::jwt::Algorithm::ES384),
            (JwtAlgorithm::RS256, slim_auth::jwt::Algorithm::RS256),
            (JwtAlgorithm::RS384, slim_auth::jwt::Algorithm::RS384),
            (JwtAlgorithm::RS512, slim_auth::jwt::Algorithm::RS512),
            (JwtAlgorithm::PS256, slim_auth::jwt::Algorithm::PS256),
            (JwtAlgorithm::PS384, slim_auth::jwt::Algorithm::PS384),
            (JwtAlgorithm::PS512, slim_auth::jwt::Algorithm::PS512),
            (JwtAlgorithm::EdDSA, slim_auth::jwt::Algorithm::EdDSA),
        ];

        for (adapter_algo, core_algo) in all_algorithms {
            let converted_core: slim_auth::jwt::Algorithm = adapter_algo.clone().into();
            let converted_back: JwtAlgorithm = converted_core.into();
            assert_eq!(converted_back, adapter_algo);

            let direct_back: JwtAlgorithm = core_algo.into();
            assert_eq!(direct_back, adapter_algo);
        }
    }
}

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
            IdentityProviderConfig::Spire { .. } => {
                // Spire not supported in core config, will be handled in TryFrom<IdentityProviderConfig> for AuthProvider
                CoreIdentityProviderConfig::None
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
            IdentityVerifierConfig::Spire { .. } => {
                // Spire not supported in core config, will be handled in TryFrom<IdentityVerifierConfig> for AuthVerifier
                CoreIdentityVerifierConfig::None
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

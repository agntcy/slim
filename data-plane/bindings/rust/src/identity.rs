// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0
//
// Identity & cryptography related UniFFI bindings.
// These provide a UniFFI-facing configuration surface for supplying
// identity (token generation) and verification logic to the Slim service.

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::builder::JwtBuilder;
use slim_auth::jwt::{Algorithm, Key, KeyData, KeyFormat, StaticTokenProvider};
use slim_auth::shared_secret::SharedSecret;
use std::sync::Arc;

/// Error type for identity operations
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum IdentityError {
    #[error("Failed to create identity provider: {reason}")]
    ProviderCreationFailed { reason: String },
    #[error("Failed to create identity verifier: {reason}")]
    VerifierCreationFailed { reason: String },
    #[error("Invalid configuration: {reason}")]
    InvalidConfiguration { reason: String },
}

/// JWT / signature algorithms exposed via UniFFI.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum BindingsAlgorithm {
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    ES256,
    ES384,
    EdDSA,
}

impl From<BindingsAlgorithm> for Algorithm {
    fn from(value: BindingsAlgorithm) -> Self {
        match value {
            BindingsAlgorithm::HS256 => Algorithm::HS256,
            BindingsAlgorithm::HS384 => Algorithm::HS384,
            BindingsAlgorithm::HS512 => Algorithm::HS512,
            BindingsAlgorithm::RS256 => Algorithm::RS256,
            BindingsAlgorithm::RS384 => Algorithm::RS384,
            BindingsAlgorithm::RS512 => Algorithm::RS512,
            BindingsAlgorithm::PS256 => Algorithm::PS256,
            BindingsAlgorithm::PS384 => Algorithm::PS384,
            BindingsAlgorithm::PS512 => Algorithm::PS512,
            BindingsAlgorithm::ES256 => Algorithm::ES256,
            BindingsAlgorithm::ES384 => Algorithm::ES384,
            BindingsAlgorithm::EdDSA => Algorithm::EdDSA,
        }
    }
}

/// Key material origin - either file path or inline content.
#[derive(uniffi::Enum, Clone, PartialEq, Eq)]
pub enum BindingsKeyData {
    File { path: String },
    Content { content: String },
}

impl From<BindingsKeyData> for KeyData {
    fn from(value: BindingsKeyData) -> Self {
        match value {
            BindingsKeyData::File { path } => KeyData::File(path),
            BindingsKeyData::Content { content } => KeyData::Data(content),
        }
    }
}

/// Supported key encoding formats.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum BindingsKeyFormat {
    Pem,
    Jwk,
    Jwks,
}

impl From<BindingsKeyFormat> for KeyFormat {
    fn from(value: BindingsKeyFormat) -> Self {
        match value {
            BindingsKeyFormat::Pem => KeyFormat::Pem,
            BindingsKeyFormat::Jwk => KeyFormat::Jwk,
            BindingsKeyFormat::Jwks => KeyFormat::Jwks,
        }
    }
}

/// Composite key description used for signing or verification.
#[derive(uniffi::Record, Clone, PartialEq)]
pub struct BindingsKey {
    pub algorithm: BindingsAlgorithm,
    pub format: BindingsKeyFormat,
    pub key: BindingsKeyData,
}

impl From<BindingsKey> for Key {
    fn from(value: BindingsKey) -> Self {
        Key {
            algorithm: value.algorithm.into(),
            format: value.format.into(),
            key: value.key.into(),
        }
    }
}

/// Identity provider - strategies for producing authentication tokens.
#[derive(uniffi::Object, Clone)]
pub struct BindingsIdentityProvider {
    pub(crate) inner: Arc<AuthProvider>,
}

impl BindingsIdentityProvider {
    pub fn new(provider: AuthProvider) -> Self {
        Self {
            inner: Arc::new(provider),
        }
    }
}

/// Identity verifier - strategies for validating authentication tokens.
#[derive(uniffi::Object, Clone)]
pub struct BindingsIdentityVerifier {
    pub(crate) inner: Arc<AuthVerifier>,
}

impl BindingsIdentityVerifier {
    pub fn new(verifier: AuthVerifier) -> Self {
        Self {
            inner: Arc::new(verifier),
        }
    }
}

/// Helper function to create a static JWT identity provider from a file path.
///
/// The token file will be read and cached. If the file changes, it will be reloaded.
#[uniffi::export]
pub fn create_identity_provider_static_jwt(path: String) -> Result<Arc<BindingsIdentityProvider>, IdentityError> {
    let jwt = JwtBuilder::new()
        .token_file(path)
        .build()
        .map_err(|e| IdentityError::ProviderCreationFailed {
            reason: format!("Failed to create static JWT provider: {}", e),
        })?;
    
    let provider = AuthProvider::StaticToken(StaticTokenProvider::from(jwt));
    Ok(Arc::new(BindingsIdentityProvider::new(provider)))
}

/// Helper function to create a JWT identity provider that signs tokens dynamically.
///
/// Args:
///   private_key: The private key for signing
///   duration_secs: Token validity duration in seconds
///   issuer: Optional issuer claim (iss)
///   audience: Optional audience claim list (aud)
///   subject: Optional subject claim (sub)
#[uniffi::export]
pub fn create_identity_provider_jwt(
    private_key: BindingsKey,
    duration_secs: u64,
    issuer: Option<String>,
    audience: Option<Vec<String>>,
    subject: Option<String>,
) -> Result<Arc<BindingsIdentityProvider>, IdentityError> {
    let mut builder = JwtBuilder::new();

    if let Some(iss) = issuer {
        builder = builder.issuer(iss);
    }
    if let Some(aud) = audience {
        builder = builder.audience(&aud);
    }
    if let Some(sub) = subject {
        builder = builder.subject(sub);
    }

    let signer = builder
        .private_key(&private_key.into())
        .token_duration(std::time::Duration::from_secs(duration_secs))
        .build()
        .map_err(|e| IdentityError::ProviderCreationFailed {
            reason: format!("Failed to create JWT provider: {}", e),
        })?;

    let provider = AuthProvider::JwtSigner(signer);
    Ok(Arc::new(BindingsIdentityProvider::new(provider)))
}

/// Helper function to create a shared secret identity provider.
#[uniffi::export]
pub fn create_identity_provider_shared_secret(
    identity: String,
    shared_secret: String,
) -> Result<Arc<BindingsIdentityProvider>, IdentityError> {
    let secret = SharedSecret::new(&identity, &shared_secret)
        .map_err(|e| IdentityError::ProviderCreationFailed {
            reason: format!("Failed to create shared secret provider: {}", e),
        })?;
    
    let provider = AuthProvider::SharedSecret(secret);
    Ok(Arc::new(BindingsIdentityProvider::new(provider)))
}

/// Helper function to create a SPIRE-based identity provider.
///
/// Args:
///   socket_path: Optional SPIRE Workload API socket path
///   target_spiffe_id: Optional target SPIFFE ID to request
///   jwt_audiences: Optional list of JWT audiences
#[uniffi::export]
pub fn create_identity_provider_spire(
    socket_path: Option<String>,
    target_spiffe_id: Option<String>,
    jwt_audiences: Option<Vec<String>>,
) -> Result<Arc<BindingsIdentityProvider>, IdentityError> {
    #[cfg(not(target_family = "windows"))]
    {
        let mut builder = slim_auth::spire::SpireIdentityManager::builder();
        if let Some(sp) = socket_path {
            builder = builder.with_socket_path(sp);
        }
        if let Some(id) = target_spiffe_id {
            builder = builder.with_target_spiffe_id(id);
        }
        if let Some(auds) = jwt_audiences {
            builder = builder.with_jwt_audiences(auds);
        }
        let mgr = builder.build();
        let provider = AuthProvider::Spire(mgr);
        Ok(Arc::new(BindingsIdentityProvider::new(provider)))
    }
    #[cfg(target_family = "windows")]
    {
        let _ = (socket_path, target_spiffe_id, jwt_audiences);
        Err(IdentityError::InvalidConfiguration {
            reason: "SPIRE is not supported on Windows".to_string(),
        })
    }
}

/// Helper function to create a key with JWKS content.
///
/// This is a convenience function for creating a key with RS256 algorithm and JWKS format.
#[uniffi::export]
pub fn create_key_with_jwks(content: String) -> BindingsKey {
    BindingsKey {
        algorithm: BindingsAlgorithm::RS256,
        format: BindingsKeyFormat::Jwks,
        key: BindingsKeyData::Content { content },
    }
}

/// Helper function to create a JWT identity verifier.
///
/// Args:
///   public_key: Optional public key for verification (if None, must use autoresolve)
///   autoresolve: Enable automatic JWKS resolution
///   issuer: Optional required issuer claim
///   audience: Optional required audience list
///   subject: Optional required subject claim
///   require_iss: Require issuer claim to be present
///   require_aud: Require audience claim to be present
///   require_sub: Require subject claim to be present
#[uniffi::export]
pub fn create_identity_verifier_jwt(
    public_key: Option<BindingsKey>,
    autoresolve: Option<bool>,
    issuer: Option<String>,
    audience: Option<Vec<String>>,
    subject: Option<String>,
    require_iss: Option<bool>,
    require_aud: Option<bool>,
    require_sub: Option<bool>,
) -> Result<Arc<BindingsIdentityVerifier>, IdentityError> {
    let mut builder = JwtBuilder::new();

    if let Some(iss) = issuer {
        builder = builder.issuer(iss);
    }
    if let Some(aud) = audience {
        builder = builder.audience(&aud);
    }
    if let Some(sub) = subject {
        builder = builder.subject(sub);
    }
    if require_iss.unwrap_or(false) {
        builder = builder.require_iss();
    }
    if require_aud.unwrap_or(false) {
        builder = builder.require_aud();
    }
    if require_sub.unwrap_or(false) {
        builder = builder.require_sub();
    }

    builder = builder.require_exp();

    let verifier = match (public_key, autoresolve.unwrap_or(false)) {
        (Some(key), _) => builder.public_key(&key.into()).build(),
        (_, true) => builder.auto_resolve_keys(true).build(),
        (_, _) => {
            return Err(IdentityError::InvalidConfiguration {
                reason: "Either public_key or autoresolve=true must be provided".to_string(),
            });
        }
    }
    .map_err(|e| IdentityError::VerifierCreationFailed {
        reason: format!("Failed to create JWT verifier: {}", e),
    })?;

    let verifier = AuthVerifier::JwtVerifier(verifier);
    Ok(Arc::new(BindingsIdentityVerifier::new(verifier)))
}

/// Helper function to create a shared secret identity verifier.
#[uniffi::export]
pub fn create_identity_verifier_shared_secret(
    identity: String,
    shared_secret: String,
) -> Result<Arc<BindingsIdentityVerifier>, IdentityError> {
    let secret = SharedSecret::new(&identity, &shared_secret)
        .map_err(|e| IdentityError::VerifierCreationFailed {
            reason: format!("Failed to create shared secret verifier: {}", e),
        })?;
    
    let verifier = AuthVerifier::SharedSecret(secret);
    Ok(Arc::new(BindingsIdentityVerifier::new(verifier)))
}

/// Helper function to create a SPIRE-based identity verifier.
///
/// Args:
///   socket_path: Optional SPIRE Workload API socket path
///   target_spiffe_id: Optional target SPIFFE ID
///   jwt_audiences: Optional list of JWT audiences
#[uniffi::export]
pub fn create_identity_verifier_spire(
    socket_path: Option<String>,
    target_spiffe_id: Option<String>,
    jwt_audiences: Option<Vec<String>>,
) -> Result<Arc<BindingsIdentityVerifier>, IdentityError> {
    #[cfg(not(target_family = "windows"))]
    {
        let mut builder = slim_auth::spire::SpireIdentityManager::builder();
        if let Some(sp) = socket_path {
            builder = builder.with_socket_path(sp);
        }
        if let Some(id) = target_spiffe_id {
            builder = builder.with_target_spiffe_id(id);
        }
        if let Some(auds) = jwt_audiences {
            builder = builder.with_jwt_audiences(auds);
        }
        let mgr = builder.build();
        let verifier = AuthVerifier::Spire(mgr);
        Ok(Arc::new(BindingsIdentityVerifier::new(verifier)))
    }
    #[cfg(target_family = "windows")]
    {
        let _ = (socket_path, target_spiffe_id, jwt_audiences);
        Err(IdentityError::InvalidConfiguration {
            reason: "SPIRE is not supported on Windows".to_string(),
        })
    }
}


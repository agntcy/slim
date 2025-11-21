// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::error::IntoAnyError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MlsError {
    // I/O / serialization
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization/Deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    // Underlying MLS library
    #[error("MLS error")]
    Mls(#[from] mls_rs::error::MlsError),

    // Crypto / provider
    #[error("crypto provider error: {0}")]
    CryptoProviderError(String),

    // Ciphersuite / client / group lifecycle
    #[error("Requested ciphersuite is unavailable")]
    CiphersuiteUnavailable,
    #[error("MLS client not initialized")]
    ClientNotInitialized,
    #[error("MLS group does not exist")]
    GroupNotExists,

    // Payload expectations
    #[error("no mls add payload found")]
    NoGroupAddPayload,
    #[error("no mls remove payload found")]
    NoGroupRemovePayload,
    #[error("No welcome message generated")]
    NoWelcomeMessage,
    #[error("Unknown payload type")]
    UnknownPayloadType,

    // Storage / identity persistence
    #[error("Failed to create storage directory: {0}")]
    StorageDirectoryCreation(std::io::Error),
    #[error("Failed to get token: {0}")]
    TokenRetrievalFailed(String),
    #[error("Failed to sync file: {0}")]
    FileSyncFailed(String),
    #[error("Identifier not found: {0}")]
    IdentifierNotFound(String),
    #[error("Credential not found in stored identity")]
    CredentialNotFound,

    // Identity / claims (merged from SlimIdentityError)
    #[error("Not a basic credential")]
    NotBasicCredential,
    #[error("Invalid UTF-8 in credential: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("Identity verification failed: {0}")]
    VerificationFailed(String),
    #[error("External sender validation failed: {0}")]
    ExternalSenderFailed(String),
    #[error("Public key not found in signed identity")]
    PublicKeyNotFound,
    #[error(
        "Public key mismatch: identity public key does not match provided public key: expected: {expected}, found: {found}"
    )]
    PublicKeyMismatch { expected: String, found: String },
    #[error("Subject not found")]
    SubjectNotFound,
    #[error("External commit not supported")]
    ExternalCommitNotSupported,
}

impl IntoAnyError for MlsError {}

impl MlsError {
    /// Helper to construct a crypto provider error from any error implementing std::error::Error.
    pub fn crypto_provider<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
        MlsError::CryptoProviderError(e.to_string())
    }

    /// Helper to construct a token retrieval failure from any displayable value.
    pub fn token_retrieval_failed<T: std::fmt::Display>(t: T) -> Self {
        MlsError::TokenRetrievalFailed(t.to_string())
    }

    /// Helper to construct an identifier not found error.
    pub fn identifier_not_found<I: std::fmt::Display>(id: I) -> Self {
        MlsError::IdentifierNotFound(id.to_string())
    }

    /// Helper to construct a verification failed error.
    pub fn verification_failed<R: std::fmt::Display>(reason: R) -> Self {
        MlsError::VerificationFailed(reason.to_string())
    }

    /// Helper to construct an external sender validation failure.
    pub fn external_sender_failed<R: std::fmt::Display>(reason: R) -> Self {
        MlsError::ExternalSenderFailed(reason.to_string())
    }
}

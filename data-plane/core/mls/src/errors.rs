// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::error::IntoAnyError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MlsError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("Serialization/Deserialization error: {0}")]
    Serde(String),
    #[error("MLS error: {0}")]
    Mls(String),
    #[error("Requested ciphersuite is unavailable")]
    CiphersuiteUnavailable,
    #[error("MLS client not initialized")]
    ClientNotInitialized,
    #[error("MLS group does not exist")]
    GroupNotExists,
    #[error("No welcome message generated")]
    NoWelcomeMessage,
    #[error("Failed to create storage directory: {0}")]
    StorageDirectoryCreation(#[from] std::io::Error),
    #[error("Failed to get token: {0}")]
    TokenRetrievalFailed(String),
    #[error("Failed to sync file: {0}")]
    FileSyncFailed(String),
    #[error("Identifier not found: {0}")]
    IdentifierNotFound(String),
    #[error("Credential not found in stored identity")]
    CredentialNotFound,
}

#[derive(Error, Debug)]
pub enum SlimIdentityError {
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

impl IntoAnyError for SlimIdentityError {}

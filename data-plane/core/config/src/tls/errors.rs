// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use rustls::server::VerifierBuilderError;
use thiserror::Error;

#[cfg(not(target_family = "windows"))]
use crate::auth::spire;

/// Errors for Config
#[derive(Error, Debug)]
pub enum ConfigError {
    // Version / format parsing
    // TLS version validation
    #[error("invalid tls version: {0}")]
    InvalidTlsVersion(String),
    // PEM / certificate/key parsing
    #[error("invalid pem format: {0}")]
    InvalidPem(#[from] rustls_pki_types::pem::Error),
    // File content/read validation
    #[error("error reading cert/key from file: {0}")]
    InvalidFile(String),
    // Low-level I/O
    #[error("file I/O error: {0}")]
    FileIo(#[from] std::io::Error),
    // SPIRE integration / configuration
    #[error("error in spire configuration: {details}, config={config:?}")]
    #[cfg(not(target_family = "windows"))]
    InvalidSpireConfig {
        details: Box<String>,
        config: Box<spire::SpireConfig>,
    },
    #[error("auth error: {0}")]
    AuthError(#[from] slim_auth::errors::AuthError),

    #[error("auth config error: {0}")]
    ConfigAuthError(#[from] crate::auth::ConfigAuthError),

    // rustls library errors
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    // Builder pattern errors
    #[error("config builder error: {0}")]
    ConfigBuilder(String),
    // Required artifacts
    #[error("missing server cert or key")]
    MissingServerCertAndKey,
    // Verifier construction errors
    #[error("verifier builder error: {0}")]
    VerifierBuilder(#[from] VerifierBuilderError),
    // Unknown / catch-all
    #[error("unknown error")]
    Unknown,

    // SPIRE runtime errors
    #[error("spire error: {0}")]
    #[cfg(not(target_family = "windows"))]
    Spire(String),
}

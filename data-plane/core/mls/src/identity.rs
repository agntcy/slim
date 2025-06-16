// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::MlsError;
use async_trait::async_trait;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, CryptoProvider,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    identity::SigningIdentity,
    identity::basic::BasicCredential,
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    name: String,
    public_key_bytes: Vec<u8>,
    private_key_bytes: Vec<u8>,
}

impl Identity {
    pub fn generate_new_identity(name: &str) -> Result<Self, MlsError> {
        let crypto_provider = AwsLcCryptoProvider::default();

        let cipher_suite_provider = crypto_provider
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or(MlsError::CiphersuiteUnavailable)?;

        let (private_key, public_key) = cipher_suite_provider
            .signature_key_generate()
            .map_err(|e| MlsError::Mls(e.to_string()))?;
        let public_key_bytes = public_key.as_bytes().to_vec();
        let private_key_bytes = private_key.as_bytes().to_vec();

        Ok(Identity {
            name: name.to_owned(),
            public_key_bytes,
            private_key_bytes,
        })
    }

    pub fn into_signing_identity(self) -> Result<(SigningIdentity, SignatureSecretKey), MlsError> {
        let public_key = SignaturePublicKey::new(self.public_key_bytes.clone());
        let private_key = SignatureSecretKey::new(self.private_key_bytes.clone());

        let basic_cred = BasicCredential::new(self.name.as_bytes().to_vec());

        let signing_identity = SigningIdentity::new(basic_cred.into_credential(), public_key);

        Ok((signing_identity, private_key))
    }

    pub fn cipher_suite(&self) -> CipherSuite {
        CIPHERSUITE
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn get_identity(&self, identifier: &str) -> Result<Identity, MlsError>;
    async fn has_identity(&self, identifier: &str) -> Result<bool, MlsError>;
    async fn store_identity(&self, identifier: &str, identity: &Identity) -> Result<(), MlsError>;
    async fn remove_identity(&self, identifier: &str) -> Result<(), MlsError>;
}

pub struct FileBasedIdentityProvider {
    base_path: std::path::PathBuf,
}

impl FileBasedIdentityProvider {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Result<Self, MlsError> {
        let base_path = base_path.as_ref().to_path_buf();
        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)
                .map_err(|e| MlsError::Io(format!("Failed to create identity directory: {}", e)))?;
        }
        Ok(Self { base_path })
    }

    fn identity_path(&self, identifier: &str) -> std::path::PathBuf {
        self.base_path.join(format!("{}_identity.json", identifier))
    }

    fn load_identity_from_file(&self, path: &Path) -> Result<Identity, MlsError> {
        let mut file = File::open(path).map_err(|e| MlsError::Io(e.to_string()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| MlsError::Io(e.to_string()))?;
        let identity: Identity =
            serde_json::from_slice(&buf).map_err(|e| MlsError::Serde(e.to_string()))?;
        Ok(identity)
    }

    fn save_identity_to_file(&self, path: &Path, identity: &Identity) -> Result<(), MlsError> {
        let json =
            serde_json::to_vec_pretty(identity).map_err(|e| MlsError::Serde(e.to_string()))?;
        let mut file = File::create(path).map_err(|e| MlsError::Io(e.to_string()))?;
        file.write_all(&json)
            .map_err(|e| MlsError::Io(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl IdentityProvider for FileBasedIdentityProvider {
    async fn get_identity(&self, identifier: &str) -> Result<Identity, MlsError> {
        let path = self.identity_path(identifier);
        if path.exists() {
            self.load_identity_from_file(&path)
        } else {
            let identity = Identity::generate_new_identity(identifier)?;
            self.save_identity_to_file(&path, &identity)?;
            Ok(identity)
        }
    }

    async fn has_identity(&self, identifier: &str) -> Result<bool, MlsError> {
        let path = self.identity_path(identifier);
        Ok(path.exists())
    }

    async fn store_identity(&self, identifier: &str, identity: &Identity) -> Result<(), MlsError> {
        let path = self.identity_path(identifier);
        self.save_identity_to_file(&path, identity)
    }

    async fn remove_identity(&self, identifier: &str) -> Result<(), MlsError> {
        let path = self.identity_path(identifier);
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                MlsError::Io(format!(
                    "Failed to remove identity for {}: {}",
                    identifier, e
                ))
            })?;
        }
        Ok(())
    }
}

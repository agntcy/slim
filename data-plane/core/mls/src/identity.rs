// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::errors::MlsError;

use serde::{Deserialize, Serialize};

use mls_rs::{
    CipherSuite, CipherSuiteProvider, CryptoProvider,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    identity::SigningIdentity,
    identity::basic::BasicCredential,
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

#[derive(Debug, Serialize, Deserialize)]
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

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), MlsError> {
        let json = serde_json::to_vec_pretty(&self).map_err(|e| MlsError::Serde(e.to_string()))?;

        let mut file = File::create(path).map_err(|e| MlsError::Io(e.to_string()))?;
        file.write_all(&json)
            .map_err(|e| MlsError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, MlsError> {
        let mut file = File::open(path).map_err(|e| MlsError::Io(e.to_string()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| MlsError::Io(e.to_string()))?;

        let stored: Identity =
            serde_json::from_slice(&buf).map_err(|e| MlsError::Serde(e.to_string()))?;
        Ok(stored)
    }

    pub fn into_signing_identity(self) -> Result<(SigningIdentity, SignatureSecretKey), MlsError> {
        let public_key = SignaturePublicKey::new(self.public_key_bytes.clone());

        let private_key = SignatureSecretKey::new(self.private_key_bytes.clone());

        let basic_cred = BasicCredential::new(self.name.as_bytes().to_vec());
        let signing_identity = SigningIdentity::new(basic_cred.into_credential(), public_key);

        Ok((signing_identity, private_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    use mls_rs::{Client, identity::basic::BasicIdentityProvider};
    use mls_rs_crypto_awslc::AwsLcCryptoProvider;

    #[test]
    fn identity_save_and_load() -> Result<(), Box<dyn std::error::Error>> {
        let identity = Identity::generate_new_identity("alice")?;

        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();
        identity.save_to_file(&path)?;

        let loaded = Identity::load_from_file(&path)?;

        let (signing_identity, private_key) = loaded.into_signing_identity()?;

        let crypto_provider = AwsLcCryptoProvider::default();
        let _client = Client::builder()
            .identity_provider(BasicIdentityProvider)
            .crypto_provider(crypto_provider.clone())
            .signing_identity(signing_identity, private_key, CIPHERSUITE)
            .build();

        Ok(())
    }
}

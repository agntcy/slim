// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::MlsError;
use crate::identity::{Identity, StoredIdentity};
use crate::identity_provider::JwtIdentityProvider;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, Group, MlsMessage,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    error::MlsError as MlsRsError,
    group::ReceivedMessage,
    identity::SigningIdentity,
    identity::basic::BasicCredential,
    identity::basic::BasicIdentityProvider,
    mls_rs_codec::{MlsDecode, MlsEncode},
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

trait MlsClientTrait: Send + Sync {
    fn create_group(&self, extensions: ExtensionList)
    -> Result<Box<dyn MlsGroupTrait>, MlsRsError>;
    fn generate_key_package_message(&self) -> Result<Vec<u8>, MlsRsError>;
    fn join_group(&self, welcome: &[u8]) -> Result<Box<dyn MlsGroupTrait>, MlsRsError>;
}

trait MlsGroupTrait: Send + Sync {
    fn group_id(&self) -> &[u8];
    fn encrypt_application_message(&mut self, message: &[u8]) -> Result<Vec<u8>, MlsRsError>;
    fn process_incoming_message(
        &mut self,
        message: &[u8],
    ) -> Result<Option<ReceivedMessage>, MlsRsError>;
    fn add_member(&mut self, key_package: &[u8]) -> Result<Vec<u8>, MlsRsError>;
    fn write_to_storage(&mut self) -> Result<(), MlsRsError>;
}

type BasicClient = Client<
    mls_rs::client_builder::WithIdentityProvider<
        BasicIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

type JwtClient = Client<
    mls_rs::client_builder::WithIdentityProvider<
        JwtIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

type BasicGroup = Group<
    mls_rs::client_builder::WithIdentityProvider<
        BasicIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

type JwtGroup = Group<
    mls_rs::client_builder::WithIdentityProvider<
        JwtIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

impl MlsClientTrait for BasicClient {
    fn create_group(
        &self,
        extensions: ExtensionList,
    ) -> Result<Box<dyn MlsGroupTrait>, MlsRsError> {
        let group = self.create_group(extensions, Default::default())?;
        Ok(Box::new(group))
    }

    fn generate_key_package_message(&self) -> Result<Vec<u8>, MlsRsError> {
        let key_package =
            self.generate_key_package_message(Default::default(), Default::default())?;
        Ok(key_package.mls_encode_to_vec()?)
    }

    fn join_group(&self, welcome: &[u8]) -> Result<Box<dyn MlsGroupTrait>, MlsRsError> {
        let welcome_msg = MlsMessage::mls_decode(&mut &welcome[..])?;
        let (group, _) = self.join_group(None, &welcome_msg)?;
        Ok(Box::new(group))
    }
}

impl MlsClientTrait for JwtClient {
    fn create_group(
        &self,
        extensions: ExtensionList,
    ) -> Result<Box<dyn MlsGroupTrait>, MlsRsError> {
        let group = self.create_group(extensions, Default::default())?;
        Ok(Box::new(group))
    }

    fn generate_key_package_message(&self) -> Result<Vec<u8>, MlsRsError> {
        let key_package =
            self.generate_key_package_message(Default::default(), Default::default())?;
        Ok(key_package.mls_encode_to_vec()?)
    }

    fn join_group(&self, welcome: &[u8]) -> Result<Box<dyn MlsGroupTrait>, MlsRsError> {
        let welcome_msg = MlsMessage::mls_decode(&mut &welcome[..])?;
        let (group, _) = self.join_group(None, &welcome_msg)?;
        Ok(Box::new(group))
    }
}

impl MlsGroupTrait for BasicGroup {
    fn group_id(&self) -> &[u8] {
        self.group_id()
    }

    fn encrypt_application_message(&mut self, message: &[u8]) -> Result<Vec<u8>, MlsRsError> {
        let mls_message = self.encrypt_application_message(message, Default::default())?;
        Ok(mls_message.mls_encode_to_vec()?)
    }

    fn process_incoming_message(
        &mut self,
        message: &[u8],
    ) -> Result<Option<ReceivedMessage>, MlsRsError> {
        let mls_message = MlsMessage::mls_decode(&mut &message[..])?;
        Ok(Some(self.process_incoming_message(mls_message)?))
    }

    fn add_member(&mut self, key_package: &[u8]) -> Result<Vec<u8>, MlsRsError> {
        let key_package_msg = MlsMessage::mls_decode(&mut &key_package[..])?;
        let commit_output = self.commit_builder().add_member(key_package_msg)?.build()?;
        self.apply_pending_commit()?;
        Ok(commit_output
            .welcome_messages
            .first()
            .unwrap()
            .mls_encode_to_vec()?)
    }

    fn write_to_storage(&mut self) -> Result<(), MlsRsError> {
        self.write_to_storage()
    }
}

impl MlsGroupTrait for JwtGroup {
    fn group_id(&self) -> &[u8] {
        self.group_id()
    }

    fn encrypt_application_message(&mut self, message: &[u8]) -> Result<Vec<u8>, MlsRsError> {
        let mls_message = self.encrypt_application_message(message, Default::default())?;
        Ok(mls_message.mls_encode_to_vec()?)
    }

    fn process_incoming_message(
        &mut self,
        message: &[u8],
    ) -> Result<Option<ReceivedMessage>, MlsRsError> {
        let mls_message = MlsMessage::mls_decode(&mut &message[..])?;
        Ok(Some(self.process_incoming_message(mls_message)?))
    }

    fn add_member(&mut self, key_package: &[u8]) -> Result<Vec<u8>, MlsRsError> {
        let key_package_msg = MlsMessage::mls_decode(&mut &key_package[..])?;
        let commit_output = self.commit_builder().add_member(key_package_msg)?.build()?;
        self.apply_pending_commit()?;
        Ok(commit_output
            .welcome_messages
            .first()
            .unwrap()
            .mls_encode_to_vec()?)
    }

    fn write_to_storage(&mut self) -> Result<(), MlsRsError> {
        self.write_to_storage()
    }
}

pub struct Mls {
    identity: Identity,
    storage_path: Option<std::path::PathBuf>,
    client: Option<Box<dyn MlsClientTrait>>,
    groups: HashMap<Vec<u8>, Box<dyn MlsGroupTrait>>,
}

impl std::fmt::Debug for Mls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("mls")
            .field("identity", &self.identity)
            .field("has_client", &self.client.is_some())
            .field("num_groups", &self.groups.len())
            .finish()
    }
}

impl Mls {
    pub fn new(identity: Identity) -> Self {
        Self {
            identity,
            storage_path: None,
            client: None,
            groups: HashMap::new(),
        }
    }

    pub fn set_storage_path<P: Into<std::path::PathBuf>>(&mut self, path: P) -> &mut Self {
        self.storage_path = Some(path.into());
        self
    }

    fn get_storage_path(&self) -> std::path::PathBuf {
        self.storage_path.clone().unwrap_or_else(|| {
            std::path::PathBuf::from(format!(
                "/tmp/mls_identities_{}",
                self.identity.storage_identifier()
            ))
        })
    }

    fn map_mls_error<T>(result: Result<T, impl std::fmt::Display>) -> Result<T, MlsError> {
        result.map_err(|e| MlsError::Mls(e.to_string()))
    }

    pub async fn initialize(&mut self) -> Result<(), MlsError> {
        let storage_path = self.get_storage_path();
        std::fs::create_dir_all(&storage_path)
            .map_err(|e| MlsError::Io(format!("Failed to create storage directory: {}", e)))?;

        let identity_file = storage_path.join("identity.json");

        let stored_identity = if identity_file.exists() {
            let mut file = File::open(&identity_file).map_err(|e| MlsError::Io(e.to_string()))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| MlsError::Io(e.to_string()))?;
            serde_json::from_slice(&buf).map_err(|e| MlsError::Serde(e.to_string()))?
        } else {
            let crypto_provider = AwsLcCryptoProvider::default();
            let cipher_suite_provider = crypto_provider
                .cipher_suite_provider(CIPHERSUITE)
                .ok_or(MlsError::CiphersuiteUnavailable)?;

            let (private_key, public_key) = cipher_suite_provider
                .signature_key_generate()
                .map_err(|e| MlsError::Mls(e.to_string()))?;

            let stored = StoredIdentity {
                identifier: self.identity.identifier().to_string(),
                public_key_bytes: public_key.as_bytes().to_vec(),
                private_key_bytes: private_key.as_bytes().to_vec(),
            };

            let json =
                serde_json::to_vec_pretty(&stored).map_err(|e| MlsError::Serde(e.to_string()))?;
            let mut file = File::create(&identity_file).map_err(|e| MlsError::Io(e.to_string()))?;
            file.write_all(&json)
                .map_err(|e| MlsError::Io(e.to_string()))?;

            stored
        };

        let public_key = SignaturePublicKey::new(stored_identity.public_key_bytes);
        let private_key = SignatureSecretKey::new(stored_identity.private_key_bytes);

        let credential_data = match &self.identity {
            Identity::Simple(name) => name.as_bytes().to_vec(),
            Identity::Jwt(token) => token.as_bytes().to_vec(),
        };

        let basic_cred = BasicCredential::new(credential_data);
        let signing_identity = SigningIdentity::new(basic_cred.into_credential(), public_key);

        let crypto_provider = AwsLcCryptoProvider::default();

        let client: Box<dyn MlsClientTrait> = match &self.identity {
            Identity::Simple(_) => {
                let basic_client = Client::builder()
                    .identity_provider(BasicIdentityProvider)
                    .crypto_provider(crypto_provider)
                    .signing_identity(signing_identity, private_key, CIPHERSUITE)
                    .build();
                Box::new(basic_client)
            }
            Identity::Jwt(_) => {
                let jwt_verifier = slim_auth::builder::JwtBuilder::new()
                    .auto_resolve_keys(true)
                    .build()
                    .map_err(|e| MlsError::Mls(format!("Failed to create JWT verifier: {}", e)))?;
                let jwt_provider = JwtIdentityProvider::new(jwt_verifier);

                let jwt_client = Client::builder()
                    .identity_provider(jwt_provider)
                    .crypto_provider(crypto_provider)
                    .signing_identity(signing_identity, private_key, CIPHERSUITE)
                    .build();
                Box::new(jwt_client)
            }
        };

        self.client = Some(client);
        Ok(())
    }

    pub fn create_group(&mut self) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        let group = Self::map_mls_error(client.create_group(ExtensionList::default()))?;

        let group_id = group.group_id().to_vec();
        self.groups.insert(group_id.clone(), group);

        Ok(group_id)
    }

    pub fn generate_key_package(&self) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        Self::map_mls_error(client.generate_key_package_message())
    }

    pub fn add_member(
        &mut self,
        group_id: &[u8],
        key_package_bytes: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        let group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| MlsError::Mls(format!("Group not found: {:?}", group_id)))?;

        Self::map_mls_error(group.add_member(key_package_bytes))
    }

    pub fn join_group(&mut self, welcome_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        let group = Self::map_mls_error(client.join_group(welcome_message))?;
        let group_id = group.group_id().to_vec();
        self.groups.insert(group_id.clone(), group);

        Ok(group_id)
    }

    pub fn is_group_member(&self, group_id: &[u8]) -> bool {
        self.groups.contains_key(group_id)
    }

    pub fn has_any_groups(&self) -> bool {
        !self.groups.is_empty()
    }

    pub fn encrypt_message(
        &mut self,
        group_id: &[u8],
        message: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        let group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| MlsError::Mls(format!("Group not found: {:?}", group_id)))?;

        Self::map_mls_error(group.encrypt_application_message(message))
    }

    pub fn decrypt_message(
        &mut self,
        group_id: &[u8],
        encrypted_message: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        let group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| MlsError::Mls(format!("Group not found: {:?}", group_id)))?;

        match Self::map_mls_error(group.process_incoming_message(encrypted_message))? {
            Some(ReceivedMessage::ApplicationMessage(app_msg)) => Ok(app_msg.data().to_vec()),
            Some(_) => Err(MlsError::Mls(
                "Message was not an application message".to_string(),
            )),
            None => Err(MlsError::Mls("No message received".to_string())),
        }
    }

    pub fn write_to_storage(&mut self) -> Result<(), MlsError> {
        for group in self.groups.values_mut() {
            Self::map_mls_error(group.write_to_storage())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mls_creation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(Identity::simple("alice"));
        mls.set_storage_path("/tmp/test_mls");

        mls.initialize().await?;
        assert!(!mls.has_any_groups());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_creation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(Identity::simple("alice"));
        mls.set_storage_path("/tmp/test_mls_group");

        mls.initialize().await?;
        let group_id = mls.create_group()?;
        assert!(mls.is_group_member(&group_id));
        Ok(())
    }

    #[tokio::test]
    async fn test_key_package_generation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(Identity::simple("alice"));
        mls.set_storage_path("/tmp/test_mls_keypack");

        mls.initialize().await?;
        let key_package = mls.generate_key_package()?;
        assert!(!key_package.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_messaging() -> Result<(), Box<dyn std::error::Error>> {
        let mut alice = Mls::new(Identity::simple("alice"));
        alice.set_storage_path("/tmp/test_mls_alice");

        let mut bob = Mls::new(Identity::simple("bob"));
        bob.set_storage_path("/tmp/test_mls_bob");

        alice.initialize().await?;
        bob.initialize().await?;

        let group_id = alice.create_group()?;
        assert!(alice.is_group_member(&group_id));

        let bob_key_package = bob.generate_key_package()?;
        let welcome_message = alice.add_member(&group_id, &bob_key_package)?;

        let bob_group_id = bob.join_group(&welcome_message)?;
        assert!(bob.is_group_member(&bob_group_id));
        assert_eq!(group_id, bob_group_id);

        let original_message = b"Hello from Alice!";
        let encrypted = alice.encrypt_message(&group_id, original_message)?;
        let decrypted = bob.decrypt_message(&bob_group_id, &encrypted)?;

        assert_eq!(original_message, decrypted.as_slice());
        assert_ne!(original_message.to_vec(), encrypted);

        Ok(())
    }

    #[tokio::test]
    async fn test_decrypt_message() -> Result<(), Box<dyn std::error::Error>> {
        let mut alice = Mls::new(Identity::simple("alice"));
        alice.set_storage_path("/tmp/test_mls_decrypt_alice");

        let mut bob = Mls::new(Identity::simple("bob"));
        bob.set_storage_path("/tmp/test_mls_decrypt_bob");

        alice.initialize().await?;
        bob.initialize().await?;
        let group_id = alice.create_group()?;

        let bob_key_package = bob.generate_key_package()?;
        let welcome_message = alice.add_member(&group_id, &bob_key_package)?;
        let bob_group_id = bob.join_group(&welcome_message)?;

        let message = b"Test message";
        let encrypted = alice.encrypt_message(&group_id, message)?;

        let decrypted = bob.decrypt_message(&bob_group_id, &encrypted)?;
        assert_eq!(decrypted, message);

        Ok(())
    }
}

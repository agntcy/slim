// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::MlsError;
use crate::identity::IdentityProvider;
use mls_rs::{
    Client, ExtensionList, Group, MlsMessage, group::ReceivedMessage,
    identity::basic::BasicIdentityProvider,
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use std::collections::HashMap;
use std::sync::Arc;

type MlsClient = Client<
    mls_rs::client_builder::WithIdentityProvider<
        BasicIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

type MlsGroup = Group<
    mls_rs::client_builder::WithIdentityProvider<
        BasicIdentityProvider,
        mls_rs::client_builder::WithCryptoProvider<
            AwsLcCryptoProvider,
            mls_rs::client_builder::BaseConfig,
        >,
    >,
>;

pub struct Mls {
    identity_provider: Arc<dyn IdentityProvider>,
    participant_id: String,
    client: Option<MlsClient>,
    groups: HashMap<Vec<u8>, MlsGroup>,
}

impl std::fmt::Debug for Mls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("mls")
            .field("participant_id", &self.participant_id)
            .field("has_client", &self.client.is_some())
            .field("num_groups", &self.groups.len())
            .finish()
    }
}

impl Mls {
    pub fn new(participant_id: String, identity_provider: Arc<dyn IdentityProvider>) -> Self {
        Self {
            identity_provider,
            participant_id,
            client: None,
            groups: HashMap::new(),
        }
    }

    fn map_mls_error<T>(result: Result<T, impl std::fmt::Display>) -> Result<T, MlsError> {
        result.map_err(|e| MlsError::Mls(e.to_string()))
    }

    pub async fn initialize(&mut self) -> Result<(), MlsError> {
        let identity = self
            .identity_provider
            .get_identity(&self.participant_id)
            .await?;

        let crypto_provider = AwsLcCryptoProvider::default();
        let (signing_identity_data, secret_key_data) = identity
            .clone()
            .into_signing_identity()
            .map_err(|e| MlsError::Mls(format!("Failed to get signing identity: {}", e)))?;

        let client = Client::builder()
            .identity_provider(BasicIdentityProvider)
            .crypto_provider(crypto_provider)
            .signing_identity(
                signing_identity_data,
                secret_key_data,
                identity.cipher_suite(),
            )
            .build();

        self.client = Some(client);
        Ok(())
    }

    pub fn create_group(&mut self) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        let group =
            Self::map_mls_error(client.create_group(ExtensionList::default(), Default::default()))?;

        let group_id = group.group_id().to_vec();
        self.groups.insert(group_id.clone(), group);

        Ok(group_id)
    }

    pub fn generate_key_package(&self) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        let key_package = Self::map_mls_error(
            client.generate_key_package_message(Default::default(), Default::default()),
        )?;
        Self::map_mls_error(key_package.to_bytes())
    }

    pub fn add_member(
        &mut self,
        group_id: &[u8],
        key_package_bytes: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), MlsError> {
        let group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| MlsError::Mls(format!("Group not found: {:?}", group_id)))?;
        let key_package = Self::map_mls_error(MlsMessage::from_bytes(key_package_bytes))?;

        let commit = Self::map_mls_error(
            group
                .commit_builder()
                .add_member(key_package)
                .and_then(|builder| builder.build()),
        )?;

        // create the commit message to broadcast in the group
        let commit_msg = Self::map_mls_error(commit.commit_message.to_bytes())?;

        // create the welcome message
        let welcome = commit
            .welcome_messages
            .first()
            .ok_or_else(|| MlsError::Mls("No welcome message generated".to_string()))
            .and_then(|welcome| Self::map_mls_error(welcome.to_bytes()))?;

        // apply the commit locally
        Self::map_mls_error(group.apply_pending_commit())?;

        Ok((commit_msg, welcome))
    }

    pub fn process_commit(
        &mut self,
        group_id: &[u8],
        commit_message: &[u8],
    ) -> Result<(), MlsError> {
        let group = self
            .groups
            .get_mut(group_id)
            .ok_or_else(|| MlsError::Mls(format!("Group not found: {:?}", group_id)))?;
        let commit = Self::map_mls_error(MlsMessage::from_bytes(commit_message))?;

        // process an incoming commit message
        Self::map_mls_error(group.process_incoming_message(commit))?;
        Ok(())
    }

    pub fn process_welcome(&mut self, welcome_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

        // process the welcome message and connect to the group
        let welcome = Self::map_mls_error(MlsMessage::from_bytes(welcome_message))?;
        let (group, _) = Self::map_mls_error(client.join_group(None, &welcome))?;

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
        let encrypted_msg =
            Self::map_mls_error(group.encrypt_application_message(message, Default::default()))?;

        Self::map_mls_error(encrypted_msg.to_bytes())
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

        let message = Self::map_mls_error(MlsMessage::from_bytes(encrypted_message))?;

        match Self::map_mls_error(group.process_incoming_message(message))? {
            ReceivedMessage::ApplicationMessage(app_msg) => Ok(app_msg.data().to_vec()),
            _ => Err(MlsError::Mls(
                "Message was not an application message".to_string(),
            )),
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
    use tokio::time;

    use super::*;
    use crate::identity::FileBasedIdentityProvider;
    use std::{sync::Arc, thread};

    #[tokio::test]
    async fn test_mls_creation() -> Result<(), Box<dyn std::error::Error>> {
        let identity_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls")?);
        let mut mls = Mls::new("alice".to_string(), identity_provider);

        mls.initialize().await?;
        assert!(!mls.has_any_groups());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_creation() -> Result<(), Box<dyn std::error::Error>> {
        let identity_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_group")?);
        let mut mls = Mls::new("alice".to_string(), identity_provider);

        mls.initialize().await?;
        let group_id = mls.create_group()?;
        assert!(mls.is_group_member(&group_id));
        Ok(())
    }

    #[tokio::test]
    async fn test_key_package_generation() -> Result<(), Box<dyn std::error::Error>> {
        let identity_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_keypack")?);
        let mut mls = Mls::new("alice".to_string(), identity_provider);

        mls.initialize().await?;
        let key_package = mls.generate_key_package()?;
        assert!(!key_package.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_messaging() -> Result<(), Box<dyn std::error::Error>> {
        let alice_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_alice")?);
        let bob_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_bob")?);
        let charlie_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_charlie")?);

        // alice will work as moderator
        let mut alice = Mls::new("alice".to_string(), alice_provider);
        let mut bob = Mls::new("bob".to_string(), bob_provider);
        let mut charlie = Mls::new("charlie".to_string(), charlie_provider);

        alice.initialize().await?;
        bob.initialize().await?;
        charlie.initialize().await?;

        let group_id = alice.create_group()?;
        assert!(alice.is_group_member(&group_id));

        // add bob to the group
        let bob_key_package = bob.generate_key_package()?;
        let (_, welcome_message) = alice.add_member(&group_id, &bob_key_package)?;

        let bob_group_id = bob.process_welcome(&welcome_message)?;
        assert!(bob.is_group_member(&bob_group_id));
        assert_eq!(group_id, bob_group_id);

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(&group_id, original_message)?;
        let decrypted = bob.decrypt_message(&bob_group_id, &encrypted)?;

        assert_eq!(original_message, decrypted.as_slice());
        assert_ne!(original_message.to_vec(), encrypted);

        let ag = alice.groups.get(&group_id).unwrap();
        let bg = bob.groups.get(&group_id).unwrap();
        assert_eq!(ag.current_epoch(), bg.current_epoch());
        assert_eq!(ag.group_id(), bg.group_id());

        thread::sleep(time::Duration::from_millis(1000));

        // add charlie
        let charlie_key_package = charlie.generate_key_package()?;
        let (commit_message, welcome_message) =
            alice.add_member(&group_id, &charlie_key_package)?;

        bob.process_commit(&group_id, &commit_message)?;

        let charlie_group_id = charlie.process_welcome(&welcome_message)?;
        assert!(charlie.is_group_member(&charlie_group_id));
        assert_eq!(group_id, charlie_group_id);

        let ag = alice.groups.get(&group_id).unwrap();
        let bg = bob.groups.get(&group_id).unwrap();
        let cg = charlie.groups.get(&group_id).unwrap();
        assert_eq!(ag.current_epoch(), bg.current_epoch());
        assert_eq!(ag.current_epoch(), cg.current_epoch());
        assert_eq!(ag.group_id(), bg.group_id());
        assert_eq!(ag.group_id(), cg.group_id());

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(&group_id, original_message)?;
        let decrypted_1 = bob.decrypt_message(&bob_group_id, &encrypted)?;
        let decrypted_2 = charlie.decrypt_message(&charlie_group_id, &encrypted)?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        let original_message = b"Hello from Charlie!";
        let encrypted = charlie.encrypt_message(&group_id, original_message)?;
        let decrypted_1 = bob.decrypt_message(&bob_group_id, &encrypted)?;
        let decrypted_2 = alice.decrypt_message(&group_id, &encrypted)?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        Ok(())
    }

    #[tokio::test]
    async fn test_decrypt_message() -> Result<(), Box<dyn std::error::Error>> {
        let alice_provider = Arc::new(FileBasedIdentityProvider::new(
            "/tmp/test_mls_decrypt_alice",
        )?);
        let bob_provider = Arc::new(FileBasedIdentityProvider::new("/tmp/test_mls_decrypt_bob")?);

        let mut alice = Mls::new("alice".to_string(), alice_provider);
        let mut bob = Mls::new("bob".to_string(), bob_provider);

        alice.initialize().await?;
        bob.initialize().await?;
        let group_id = alice.create_group()?;

        let bob_key_package = bob.generate_key_package()?;
        let (_, welcome_message) = alice.add_member(&group_id, &bob_key_package)?;
        let bob_group_id = bob.process_welcome(&welcome_message)?;

        let message = b"Test message";
        let encrypted = alice.encrypt_message(&group_id, message)?;

        let decrypted = bob.decrypt_message(&bob_group_id, &encrypted)?;
        assert_eq!(decrypted, message);

        Ok(())
    }
}

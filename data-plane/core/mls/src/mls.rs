// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::MlsError;
use crate::identity_provider::SlimIdentityProvider;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, Group, MlsMessage,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    group::ReceivedMessage,
    identity::SigningIdentity,
    identity::basic::BasicCredential,
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredIdentity {
    identifier: String,
    public_key_bytes: Vec<u8>,
    private_key_bytes: Vec<u8>,
}

pub struct Mls<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    agent: slim_datapath::messages::Agent,
    storage_path: Option<std::path::PathBuf>,
    client: Option<
        Client<
            mls_rs::client_builder::WithIdentityProvider<
                SlimIdentityProvider<V>,
                mls_rs::client_builder::WithCryptoProvider<
                    AwsLcCryptoProvider,
                    mls_rs::client_builder::BaseConfig,
                >,
            >,
        >,
    >,
    groups: HashMap<
        Vec<u8>,
        Group<
            mls_rs::client_builder::WithIdentityProvider<
                SlimIdentityProvider<V>,
                mls_rs::client_builder::WithCryptoProvider<
                    AwsLcCryptoProvider,
                    mls_rs::client_builder::BaseConfig,
                >,
            >,
        >,
    >,
    identity_provider: P,
    identity_verifier: V,
}

impl<P, V> std::fmt::Debug for Mls<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("mls")
            .field("agent", &self.agent)
            .field("has_client", &self.client.is_some())
            .field("num_groups", &self.groups.len())
            .finish()
    }
}

impl<P, V> Mls<P, V>
where
    P: slim_auth::traits::TokenProvider + Send + Sync + Clone + 'static,
    V: slim_auth::traits::Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(
        agent: slim_datapath::messages::Agent,
        identity_provider: P,
        identity_verifier: V,
    ) -> Self {
        // Hash the agent for storage path
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        agent.to_string().hash(&mut hasher);
        let hashed_agent = hasher.finish();

        let storage_path = Some(std::path::PathBuf::from(format!(
            "/tmp/mls_identities_{}",
            hashed_agent
        )));

        Self {
            agent,
            storage_path,
            client: None,
            groups: HashMap::new(),
            identity_provider,
            identity_verifier,
        }
    }

    pub fn set_storage_path<T: Into<std::path::PathBuf>>(&mut self, path: T) -> &mut Self {
        self.storage_path = Some(path.into());
        self
    }

    fn get_storage_path(&self) -> std::path::PathBuf {
        self.storage_path
            .clone()
            .expect("Storage path should always be set in constructor")
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
                identifier: self.agent.to_string(),
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

        // Get the token from the identity provider
        let token = self
            .identity_provider
            .get_token()
            .await
            .map_err(|e| MlsError::Mls(format!("Failed to get token: {}", e)))?;
        let credential_data = token.as_bytes().to_vec();
        let basic_cred = BasicCredential::new(credential_data);
        let signing_identity = SigningIdentity::new(basic_cred.into_credential(), public_key);

        let crypto_provider = AwsLcCryptoProvider::default();

        let identity_provider = SlimIdentityProvider::new(self.identity_verifier.clone());

        let client = Client::builder()
            .identity_provider(identity_provider)
            .crypto_provider(crypto_provider)
            .signing_identity(signing_identity, private_key, CIPHERSUITE)
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
    ) -> Result<Vec<u8>, MlsError> {
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

        Self::map_mls_error(group.apply_pending_commit())?;

        commit
            .welcome_messages
            .first()
            .ok_or_else(|| MlsError::Mls("No welcome message generated".to_string()))
            .and_then(|welcome| Self::map_mls_error(welcome.to_bytes()))
    }

    pub fn join_group(&mut self, welcome_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MlsError::Mls("MLS client not initialized".to_string()))?;

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
    use super::*;
    use slim_auth::simple::Simple;

    #[tokio::test]
    async fn test_mls_creation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "test_agent", 0);
        let mut mls = Mls::new(
            agent,
            Simple::new("test_secret"),
            Simple::new("test_secret"),
        );
        mls.set_storage_path("/tmp/test_mls");

        mls.initialize().await?;
        assert!(!mls.has_any_groups());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_creation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "test_agent", 0);
        let mut mls = Mls::new(
            agent,
            Simple::new("test_secret"),
            Simple::new("test_secret"),
        );
        mls.set_storage_path("/tmp/test_mls_group");

        mls.initialize().await?;
        let group_id = mls.create_group()?;
        assert!(mls.is_group_member(&group_id));
        Ok(())
    }

    #[tokio::test]
    async fn test_key_package_generation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "test_agent", 0);
        let mut mls = Mls::new(
            agent,
            Simple::new("test_secret"),
            Simple::new("test_secret"),
        );
        mls.set_storage_path("/tmp/test_mls_keypack");

        mls.initialize().await?;
        let key_package = mls.generate_key_package()?;
        assert!(!key_package.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_messaging() -> Result<(), Box<dyn std::error::Error>> {
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "test_agent_alice", 0);
        let mut alice = Mls::new(
            alice_agent,
            Simple::new("alice_secret"),
            Simple::new("alice_secret"),
        );
        alice.set_storage_path("/tmp/test_mls_alice");

        let bob_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "test_agent_bob", 1);
        let mut bob = Mls::new(
            bob_agent,
            Simple::new("bob_secret"),
            Simple::new("bob_secret"),
        );
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
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "test_agent_alice", 0);
        let mut alice = Mls::new(
            alice_agent,
            Simple::new("alice_secret"),
            Simple::new("alice_secret"),
        );
        alice.set_storage_path("/tmp/test_mls_decrypt_alice");

        let bob_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "test_agent_bob", 1);
        let mut bob = Mls::new(
            bob_agent,
            Simple::new("bob_secret"),
            Simple::new("bob_secret"),
        );
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

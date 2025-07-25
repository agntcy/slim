// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::errors::MlsError;
use crate::identity_provider::SlimIdentityProvider;
use mls_rs::IdentityProvider;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, Group, MlsMessage,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    group::ReceivedMessage,
    identity::{
        SigningIdentity,
        basic::{self, BasicCredential},
    },
};
use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use serde::{Deserialize, Serialize};
use slim_auth::traits::{TokenProvider, Verifier};
use std::fs::File;
use std::io::{Read, Write};
use tracing::debug;

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;
const IDENTITY_FILENAME: &str = "identity.json";

pub type CommitMsg = Vec<u8>;
pub type WelcomeMsg = Vec<u8>;
pub type ProposalMsg = Vec<u8>;
pub type KeyPackageMsg = Vec<u8>;
pub type MlsIdentity = Vec<u8>;
pub struct MlsAddMemberResult {
    pub welcome_message: WelcomeMsg,
    pub commit_message: CommitMsg,
    pub member_identity: MlsIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredIdentity {
    identifier: String,
    public_key_bytes: Vec<u8>,
    private_key_bytes: Vec<u8>,
    last_credential: Option<String>,
    #[serde(default)]
    credential_version: u64,
}

impl StoredIdentity {
    fn exists(storage_path: &std::path::Path) -> bool {
        storage_path.join(IDENTITY_FILENAME).exists()
    }

    fn load_from_storage(storage_path: &std::path::Path) -> Result<Self, MlsError> {
        let identity_file = storage_path.join(IDENTITY_FILENAME);
        let mut file = File::open(&identity_file).map_err(|e| MlsError::Io(e.to_string()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| MlsError::Io(e.to_string()))?;
        serde_json::from_slice(&buf).map_err(|e| MlsError::Serde(e.to_string()))
    }

    fn save_to_storage(&self, storage_path: &std::path::Path) -> Result<(), MlsError> {
        let identity_file = storage_path.join(IDENTITY_FILENAME);
        let json = serde_json::to_vec_pretty(self).map_err(|e| MlsError::Serde(e.to_string()))?;
        let mut file = File::create(&identity_file).map_err(|e| MlsError::Io(e.to_string()))?;
        file.write_all(&json)
            .map_err(|e| MlsError::Io(e.to_string()))?;
        file.sync_all()
            .map_err(|e| MlsError::FileSyncFailed(e.to_string()))?;
        Ok(())
    }
}

pub struct Mls<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    agent: slim_datapath::messages::Agent,
    storage_path: Option<std::path::PathBuf>,
    stored_identity: Option<StoredIdentity>,
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
    group: Option<
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
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("mls");
        debug_struct
            .field("agent", &self.agent)
            .field("has_client", &self.client.is_some())
            .field("has_group", &self.group.is_some());

        if let Some(group) = &self.group {
            debug_struct
                .field("group_id", &hex::encode(group.group_id()))
                .field("epoch", &group.current_epoch());
        }

        debug_struct.finish()
    }
}

impl<P, V> Mls<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub fn new(
        agent: slim_datapath::messages::Agent,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
    ) -> Self {
        let mls_storage_path = Some(storage_path.join("mls"));

        Self {
            agent,
            storage_path: mls_storage_path,
            stored_identity: None,
            client: None,
            group: None,
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

    fn generate_key_pair() -> Result<(SignatureSecretKey, SignaturePublicKey), MlsError> {
        let crypto_provider = AwsLcCryptoProvider::default();
        let cipher_suite_provider = crypto_provider
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or(MlsError::CiphersuiteUnavailable)?;

        cipher_suite_provider
            .signature_key_generate()
            .map_err(|e| MlsError::Mls(e.to_string()))
    }

    pub fn initialize(&mut self) -> Result<(), MlsError> {
        debug!("Initializing MLS client for agent: {}", self.agent);
        let storage_path = self.get_storage_path();
        debug!("Using storage path: {}", storage_path.display());
        std::fs::create_dir_all(&storage_path).map_err(MlsError::StorageDirectoryCreation)?;

        let token = self
            .identity_provider
            .get_token()
            .map_err(|e| MlsError::TokenRetrievalFailed(e.to_string()))?;

        let stored_identity = if StoredIdentity::exists(&storage_path) {
            debug!("Loading existing identity from file");
            StoredIdentity::load_from_storage(&storage_path)?
        } else {
            debug!("Creating new identity for agent");
            let (private_key, public_key) = Self::generate_key_pair()?;

            let stored = StoredIdentity {
                identifier: self.agent.to_string(),
                public_key_bytes: public_key.as_bytes().to_vec(),
                private_key_bytes: private_key.as_bytes().to_vec(),
                last_credential: Some(token.clone()),
                credential_version: 1,
            };

            stored.save_to_storage(&storage_path)?;

            stored
        };

        let public_key = SignaturePublicKey::new(stored_identity.public_key_bytes.clone());
        let private_key = SignatureSecretKey::new(stored_identity.private_key_bytes.clone());

        self.stored_identity = Some(stored_identity);

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
        debug!("MLS client initialization completed successfully");
        Ok(())
    }

    pub fn create_group(&mut self) -> Result<Vec<u8>, MlsError> {
        debug!("Creating new MLS group");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        let group = Self::map_mls_error(client.create_group(
            ExtensionList::default(),
            Default::default(),
            None,
        ))?;

        let group_id = group.group_id().to_vec();
        self.group = Some(group);
        debug!(
            "MLS group created successfully with ID: {:?}",
            hex::encode(&group_id)
        );

        Ok(group_id)
    }

    pub fn generate_key_package(&self) -> Result<KeyPackageMsg, MlsError> {
        debug!("Generating key package");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        let key_package = Self::map_mls_error(client.generate_key_package_message(
            Default::default(),
            Default::default(),
            None,
        ))?;
        Self::map_mls_error(key_package.to_bytes())
    }

    pub fn add_member(&mut self, key_package_bytes: &[u8]) -> Result<MlsAddMemberResult, MlsError> {
        debug!("Adding member to the MLS group");
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
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
            .ok_or(MlsError::NoWelcomeMessage)
            .and_then(|welcome| Self::map_mls_error(welcome.to_bytes()))?;

        // apply the commit locally
        Self::map_mls_error(group.apply_pending_commit())?;

        let binding = group.roster().members();
        let member = binding.last().unwrap();
        let identifier = Self::map_mls_error(
            basic::BasicIdentityProvider::new()
                .identity(&member.signing_identity, &member.extensions),
        )?;

        let ret = MlsAddMemberResult {
            welcome_message: welcome,
            commit_message: commit_msg,
            member_identity: identifier,
        };
        Ok(ret)
    }

    pub fn remove_member(&mut self, identity: &[u8]) -> Result<CommitMsg, MlsError> {
        debug!("Removing member from the  MLS group");
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let m = Self::map_mls_error(group.member_with_identity(identity))?;

        let commit = Self::map_mls_error(
            group
                .commit_builder()
                .remove_member(m.index)
                .and_then(|builder| builder.build()),
        )?;

        let commit_msg = Self::map_mls_error(commit.commit_message.to_bytes())?;

        Self::map_mls_error(group.apply_pending_commit())?;

        Ok(commit_msg)
    }

    pub fn process_commit(&mut self, commit_message: &[u8]) -> Result<(), MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        let commit = Self::map_mls_error(MlsMessage::from_bytes(commit_message))?;

        // process an incoming commit message
        Self::map_mls_error(group.process_incoming_message(commit))?;
        Ok(())
    }

    pub fn process_welcome(&mut self, welcome_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Processing welcome message and joining MLS group");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        // process the welcome message and connect to the group
        let welcome = Self::map_mls_error(MlsMessage::from_bytes(welcome_message))?;
        let (group, _) = Self::map_mls_error(client.join_group(None, &welcome, None))?;

        let group_id = group.group_id().to_vec();
        self.group = Some(group);
        debug!(
            "Successfully joined MLS group with ID: {:?}",
            hex::encode(&group_id)
        );

        Ok(group_id)
    }

    pub fn process_proposal(
        &mut self,
        proposal_message: &[u8],
        create_commit: bool,
    ) -> Result<CommitMsg, MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        let proposal = Self::map_mls_error(MlsMessage::from_bytes(proposal_message))?;

        Self::map_mls_error(group.process_incoming_message(proposal))?;

        if !create_commit {
            debug!("process proposal but do not create commit. return empty commit");
            return Ok(vec![]);
        }

        // create commit message from proposal
        let commit = Self::map_mls_error(group.commit_builder().build())?;

        // apply the commit locally
        Self::map_mls_error(group.apply_pending_commit())?;

        // return the commit message
        let commit_msg = Self::map_mls_error(commit.commit_message.to_bytes())?;
        Ok(commit_msg)
    }

    pub fn process_local_pending_proposal(&mut self) -> Result<CommitMsg, MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        // create commit message from proposal
        let commit = Self::map_mls_error(group.commit_builder().build())?;

        // apply the commit locally
        Self::map_mls_error(group.apply_pending_commit())?;

        // return the commit message
        let commit_msg = Self::map_mls_error(commit.commit_message.to_bytes())?;
        Ok(commit_msg)
    }

    pub fn encrypt_message(&mut self, message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Encrypting MLS message");

        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let encrypted_msg =
            Self::map_mls_error(group.encrypt_application_message(message, Default::default()))?;

        let msg = Self::map_mls_error(encrypted_msg.to_bytes())?;
        Ok(msg)
    }

    pub fn decrypt_message(&mut self, encrypted_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Decrypting MLS message");

        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let message = Self::map_mls_error(MlsMessage::from_bytes(encrypted_message))?;

        match Self::map_mls_error(group.process_incoming_message(message))? {
            ReceivedMessage::ApplicationMessage(app_msg) => Ok(app_msg.data().to_vec()),
            _ => Err(MlsError::Mls(
                "Message was not an application message".to_string(),
            )),
        }
    }

    pub fn write_to_storage(&mut self) -> Result<(), MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        Self::map_mls_error(group.write_to_storage())?;
        Ok(())
    }

    pub fn get_group_id(&self) -> Option<Vec<u8>> {
        self.group.as_ref().map(|g| g.group_id().to_vec())
    }

    pub fn get_epoch(&self) -> Option<u64> {
        self.group.as_ref().map(|g| g.current_epoch())
    }

    pub fn create_rotation_proposal(&mut self) -> Result<ProposalMsg, MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        // get the current credentials
        let token = self
            .identity_provider
            .get_token()
            .map_err(|e| MlsError::TokenRetrievalFailed(e.to_string()))?;

        let credential_data = token.as_bytes().to_vec();
        let basic_cred = BasicCredential::new(credential_data);

        let (new_private_key, new_public_key) = Self::generate_key_pair()?;

        let new_signing_identity =
            SigningIdentity::new(basic_cred.into_credential(), new_public_key.clone());

        let update_proposal = Self::map_mls_error(group.propose_update_with_identity(
            new_private_key.clone(),
            new_signing_identity,
            vec![],
        ))?;

        debug!(
            "Created credential rotation proposal, store it and return the message to the caller"
        );

        let storage_path = self.get_storage_path();
        if let Some(stored) = self.stored_identity.as_mut() {
            stored.last_credential = Some(token);
            stored.credential_version = stored.credential_version.saturating_add(1);
            stored.public_key_bytes = new_public_key.as_bytes().to_vec();
            stored.private_key_bytes = new_private_key.as_bytes().to_vec();

            stored.save_to_storage(&storage_path)?;
        }

        Self::map_mls_error(update_proposal.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use tokio::time;

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use std::thread;

    #[test]
    fn test_mls_creation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let mut mls = Mls::new(
            agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_test_creation"),
        );

        mls.initialize()?;
        assert!(mls.client.is_some());
        assert!(mls.group.is_none());
        Ok(())
    }

    #[test]
    fn test_group_creation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let mut mls = Mls::new(
            agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_test_group_creation"),
        );

        mls.initialize()?;
        let _group_id = mls.create_group()?;
        assert!(mls.client.is_some());
        assert!(mls.group.is_some());
        Ok(())
    }

    #[test]
    fn test_key_package_generation() -> Result<(), Box<dyn std::error::Error>> {
        let agent = slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let mut mls = Mls::new(
            agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_test_key_package"),
        );

        mls.initialize()?;
        let key_package = mls.generate_key_package()?;
        assert!(!key_package.is_empty());
        Ok(())
    }

    #[test]
    fn test_messaging() -> Result<(), Box<dyn std::error::Error>> {
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let bob_agent = slim_datapath::messages::Agent::from_strings("org", "default", "bob", 0);
        let charlie_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "charlie", 0);
        let daniel_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "daniel", 0);

        // alice will work as moderator
        let mut alice = Mls::new(
            alice_agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_test_messaging_alice"),
        );
        let mut bob = Mls::new(
            bob_agent,
            SharedSecret::new("bob", "group"),
            SharedSecret::new("bob", "group"),
            std::path::PathBuf::from("/tmp/mls_test_messaging_bob"),
        );
        let mut charlie = Mls::new(
            charlie_agent,
            SharedSecret::new("charlie", "group"),
            SharedSecret::new("charlie", "group"),
            std::path::PathBuf::from("/tmp/mls_test_messaging_charlie"),
        );
        let mut daniel = Mls::new(
            daniel_agent,
            SharedSecret::new("daniel", "group"),
            SharedSecret::new("daniel", "group"),
            std::path::PathBuf::from("/tmp/mls_test_messaging_daniel"),
        );

        alice.initialize()?;
        bob.initialize()?;
        charlie.initialize()?;
        daniel.initialize()?;

        let group_id = alice.create_group()?;

        // add bob to the group
        let bob_key_package = bob.generate_key_package()?;
        let bob_add_res = alice.add_member(&bob_key_package)?;

        let bob_group_id = bob.process_welcome(&bob_add_res.welcome_message)?;
        assert_eq!(group_id, bob_group_id);

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message)?;
        let decrypted = bob.decrypt_message(&encrypted)?;

        assert_eq!(original_message, decrypted.as_slice());
        assert_ne!(original_message.to_vec(), encrypted);

        assert_eq!(alice.get_epoch().unwrap(), bob.get_epoch().unwrap());
        assert_eq!(alice.get_group_id().unwrap(), bob.get_group_id().unwrap());

        thread::sleep(time::Duration::from_millis(1000));

        // add charlie
        let charlie_key_package = charlie.generate_key_package()?;
        let charlie_add_res = alice.add_member(&charlie_key_package)?;

        bob.process_commit(&charlie_add_res.commit_message)?;

        let charlie_group_id = charlie.process_welcome(&charlie_add_res.welcome_message)?;
        assert_eq!(group_id, charlie_group_id);

        assert_eq!(alice.get_epoch().unwrap(), bob.get_epoch().unwrap());
        assert_eq!(alice.get_epoch().unwrap(), charlie.get_epoch().unwrap());
        assert_eq!(alice.get_group_id().unwrap(), bob.get_group_id().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            charlie.get_group_id().unwrap()
        );

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message)?;
        let decrypted_1 = bob.decrypt_message(&encrypted)?;
        let decrypted_2 = charlie.decrypt_message(&encrypted)?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        let original_message = b"Hello from Charlie!";
        let encrypted = charlie.encrypt_message(original_message)?;
        let decrypted_1 = bob.decrypt_message(&encrypted)?;
        let decrypted_2 = alice.decrypt_message(&encrypted)?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        // remove bob
        let remove_msg = alice.remove_member(&bob_add_res.member_identity)?;
        charlie.process_commit(&remove_msg)?;
        bob.process_commit(&remove_msg)?;
        assert_eq!(alice.get_epoch().unwrap(), charlie.get_epoch().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            charlie.get_group_id().unwrap()
        );

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message)?;
        let decrypted = charlie.decrypt_message(&encrypted)?;
        assert_eq!(original_message, decrypted.as_slice());

        let original_message = b"Hello from Charlie!";
        let encrypted = charlie.encrypt_message(original_message)?;
        let decrypted = alice.decrypt_message(&encrypted)?;
        assert_eq!(original_message, decrypted.as_slice());

        // add daniel and remove charlie
        let daniel_key_package = daniel.generate_key_package()?;
        let daniel_add_res = alice.add_member(&daniel_key_package)?;

        charlie.process_commit(&daniel_add_res.commit_message)?;

        let daniel_group_id = daniel.process_welcome(&daniel_add_res.welcome_message)?;
        assert_eq!(group_id, daniel_group_id);
        assert_eq!(alice.get_epoch().unwrap(), charlie.get_epoch().unwrap());
        assert_eq!(alice.get_epoch().unwrap(), daniel.get_epoch().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            daniel.get_group_id().unwrap()
        );
        assert_eq!(
            alice.get_group_id().unwrap(),
            charlie.get_group_id().unwrap()
        );

        let commit = alice.remove_member(&charlie_add_res.member_identity)?;

        daniel.process_commit(&commit)?;
        assert_eq!(alice.get_epoch().unwrap(), daniel.get_epoch().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            daniel.get_group_id().unwrap()
        );

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message)?;
        let decrypted = daniel.decrypt_message(&encrypted)?;
        assert_eq!(original_message, decrypted.as_slice());

        Ok(())
    }

    #[test]
    fn test_decrypt_message() -> Result<(), Box<dyn std::error::Error>> {
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let bob_agent = slim_datapath::messages::Agent::from_strings("org", "default", "bob", 1);

        let mut alice = Mls::new(
            alice_agent,
            SharedSecret::new("alice", "group"),
            SharedSecret::new("alice", "group"),
            std::path::PathBuf::from("/tmp/mls_test_decrypt_alice"),
        );
        let mut bob = Mls::new(
            bob_agent,
            SharedSecret::new("bob", "group"),
            SharedSecret::new("bob", "group"),
            std::path::PathBuf::from("/tmp/mls_test_decrypt_bob"),
        );

        alice.initialize()?;
        bob.initialize()?;
        let _group_id = alice.create_group()?;

        let bob_key_package = bob.generate_key_package()?;
        let res = alice.add_member(&bob_key_package)?;
        let _bob_group_id = bob.process_welcome(&res.welcome_message)?;

        let message = b"Test message";
        let encrypted = alice.encrypt_message(message)?;

        let decrypted = bob.decrypt_message(&encrypted)?;
        assert_eq!(decrypted, message);

        Ok(())
    }

    #[test]
    fn test_shared_secret_rotation_same_identity() -> Result<(), Box<dyn std::error::Error>> {
        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let bob_agent = slim_datapath::messages::Agent::from_strings("org", "default", "bob", 1);

        let mut alice = Mls::new(
            alice_agent.clone(),
            SharedSecret::new("alice", "secret_v1"),
            SharedSecret::new("alice", "secret_v1"),
            std::path::PathBuf::from("/tmp/mls_test_rotation_alice"),
        );
        let mut bob = Mls::new(
            bob_agent.clone(),
            SharedSecret::new("bob", "secret_v1"),
            SharedSecret::new("bob", "secret_v1"),
            std::path::PathBuf::from("/tmp/mls_test_rotation_bob"),
        );

        alice.initialize()?;
        bob.initialize()?;
        let _group_id = alice.create_group()?;

        let bob_key_package = bob.generate_key_package()?;
        let result = alice.add_member(&bob_key_package)?;
        let welcome_message = result.welcome_message;
        let _bob_group_id = bob.process_welcome(&welcome_message)?;

        let message1 = b"Message with secret_v1";
        let encrypted1 = alice.encrypt_message(message1)?;
        let decrypted1 = bob.decrypt_message(&encrypted1)?;
        assert_eq!(decrypted1, message1);

        let mut alice_rotated_secret = Mls::new(
            alice_agent,
            SharedSecret::new("alice", "secret_v2"),
            SharedSecret::new("alice", "secret_v2"),
            std::path::PathBuf::from("/tmp/mls_test_rotation_alice_v2"),
        );
        alice_rotated_secret.initialize()?;

        let message2 = b"Message with rotated secret";
        let encrypted2_result = alice_rotated_secret.encrypt_message(message2);
        assert!(encrypted2_result.is_err());

        let message3 = b"Message from original alice after secret rotation";
        let encrypted3 = alice.encrypt_message(message3)?;
        let decrypted3 = bob.decrypt_message(&encrypted3)?;
        assert_eq!(decrypted3, message3);

        Ok(())
    }

    #[test]
    fn test_full_credential_rotation_flow() -> Result<(), Box<dyn std::error::Error>> {
        let alice_path = "/tmp/mls_test_full_rotation_alice";
        let bob_path = "/tmp/mls_test_full_rotation_bob";
        let moderator_path = "/tmp/mls_test_full_rotation_moderator";
        let _ = std::fs::remove_dir_all(alice_path);
        let _ = std::fs::remove_dir_all(bob_path);
        let _ = std::fs::remove_dir_all(moderator_path);

        let alice_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "alice", 0);
        let bob_agent = slim_datapath::messages::Agent::from_strings("org", "default", "bob", 1);
        let moderator_agent =
            slim_datapath::messages::Agent::from_strings("org", "default", "moderator", 2);

        let mut moderator = Mls::new(
            moderator_agent.clone(),
            SharedSecret::new("moderator", "secret_v1"),
            SharedSecret::new("moderator", "secret_v1"),
            std::path::PathBuf::from("/tmp/mls_test_moderator"),
        );
        moderator.initialize()?;

        // Moderator creates the group
        let _group_id = moderator.create_group()?;

        let mut alice = Mls::new(
            alice_agent.clone(),
            SharedSecret::new("alice", "secret_v1"),
            SharedSecret::new("alice", "secret_v1"),
            alice_path.into(),
        );
        alice.initialize()?;

        let mut bob = Mls::new(
            bob_agent.clone(),
            SharedSecret::new("bob", "secret_v1"),
            SharedSecret::new("bob", "secret_v1"),
            bob_path.into(),
        );
        bob.initialize()?;

        // Moderator adds Alice to the group
        let alice_key_package = alice.generate_key_package()?;
        let result = moderator.add_member(&alice_key_package)?;
        let welcome_alice = result.welcome_message;
        let _alice_group_id = alice.process_welcome(&welcome_alice)?;

        // Moderator adds Bob to the group
        let bob_key_package = bob.generate_key_package()?;
        let result = moderator.add_member(&bob_key_package)?;
        let commit_bob = result.commit_message;
        let welcome_bob = result.welcome_message;
        let _bob_group_id = bob.process_welcome(&welcome_bob)?;

        // Only Alice needs to process Bob's addition (Bob wasn't in the group when Alice was added)
        alice.process_commit(&commit_bob)?;

        let message1 = b"Message before rotation";
        let encrypted1 = alice.encrypt_message(message1)?;
        let decrypted1 = bob.decrypt_message(&encrypted1)?;
        assert_eq!(decrypted1, message1);

        // Alice create a proposal
        let rotation_proposal = alice.create_rotation_proposal()?;

        // send proposal to the moderator
        let commit = moderator.process_proposal(&rotation_proposal, true)?;
        // send proposal also to bob
        bob.process_proposal(&rotation_proposal, false)?;

        // broadcast the commit message
        alice.process_commit(&commit)?;
        bob.process_commit(&commit)?;

        // Test messaging after rotation
        // Bob can decrypt Alice's encrypted message
        let message2 = b"Message after rotation from alice";
        let encrypted2 = alice.encrypt_message(message2)?;
        let decrypted2 = bob.decrypt_message(&encrypted2)?;
        assert_eq!(decrypted2, message2);

        // ... and Alice can decrypt Bob's encrypted message
        let message3 = b"Message after rotation from bob";
        let encrypted3 = bob.encrypt_message(message3)?;
        let decrypted3 = alice.decrypt_message(&encrypted3)?;
        assert_eq!(decrypted3, message3);

        // Verify epochs are synchronized
        assert_eq!(
            alice.get_epoch(),
            bob.get_epoch(),
            "Alice and Bob epochs should match after rotation"
        );
        assert_eq!(
            alice.get_epoch(),
            moderator.get_epoch(),
            "Alice and Moderator epochs should match after rotation"
        );

        // The end.
        Ok(())
    }
}

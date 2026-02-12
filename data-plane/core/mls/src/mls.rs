// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use mls_rs::IdentityProvider;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, Group, MlsMessage,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    group::ReceivedMessage,
    identity::{SigningIdentity, basic::BasicCredential},
};

use mls_rs_crypto_awslc::AwsLcCryptoProvider;
use std::collections::HashSet;
use tracing::debug;

use slim_auth::traits::{TokenProvider, Verifier};

use crate::errors::MlsError;
use crate::identity_claims::IdentityClaims;
use crate::identity_provider::SlimIdentityProvider;

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

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

#[derive(Clone, Debug)]
struct InMemoryIdentity {
    #[allow(dead_code)]
    identifier: String,
    public_key_bytes: Vec<u8>,
    private_key_bytes: Vec<u8>,
    last_credential: Option<String>,
    credential_version: u64,
}

pub struct Mls<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    identity: Option<String>,
    stored_identity: Option<InMemoryIdentity>,
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
            .field("identity", &self.identity)
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
    pub fn new(identity_provider: P, identity_verifier: V) -> Self {
        Self {
            identity: None,
            stored_identity: None,
            client: None,
            group: None,
            identity_provider,
            identity_verifier,
        }
    }

    /// Helper method to create a signing identity from key pair
    /// Generates a token with the public key in the claims, creates a BasicCredential
    async fn create_signing_identity(
        &mut self,
        private_key: &SignatureSecretKey,
        public_key: &SignaturePublicKey,
        is_rotation: bool,
    ) -> Result<SigningIdentity, MlsError> {
        // Generate a fresh token with the public key in claims
        let token = self
            .identity_provider
            .get_token_with_claims(IdentityClaims::from_public_key_bytes(public_key.as_ref()))
            .await?;

        let credential_data = token.as_bytes().to_vec();

        // Notice that we use BasicCredentials just as a way to transport the token,
        // the actual verification is done via the SlimIdentityProvider
        let basic_cred = BasicCredential::new(credential_data);
        let signing_identity =
            SigningIdentity::new(basic_cred.into_credential(), public_key.clone());

        // Update in-memory identity
        if let Some(stored) = self.stored_identity.as_mut() {
            stored.last_credential = Some(token);
            stored.public_key_bytes = public_key.as_bytes().to_vec();
            stored.private_key_bytes = private_key.as_bytes().to_vec();

            if is_rotation {
                stored.credential_version = stored.credential_version.saturating_add(1);
            }
        }

        Ok(signing_identity)
    }

    async fn generate_key_pair() -> Result<(SignatureSecretKey, SignaturePublicKey), MlsError> {
        let crypto_provider = AwsLcCryptoProvider::default();
        let cipher_suite_provider = crypto_provider
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or(MlsError::CiphersuiteUnavailable)?;

        cipher_suite_provider
            .signature_key_generate()
            .await
            .map_err(MlsError::crypto_provider)
    }

    pub async fn initialize(&mut self) -> Result<(), MlsError> {
        debug!("Initializing MLS with ephemeral keys (in-memory only)");

        // Generate ephemeral keys
        let (private_key, public_key) = Self::generate_key_pair().await?;

        self.identity = Some(self.identity_provider.get_id()?);

        let stored_identity = InMemoryIdentity {
            identifier: self
                .identity
                .clone()
                .map(|id| id.to_string())
                .expect("MLS identity could not be determined from identity provider"),
            public_key_bytes: public_key.as_bytes().to_vec(),
            private_key_bytes: private_key.as_bytes().to_vec(),
            last_credential: None,
            credential_version: 1,
        };

        self.stored_identity = Some(stored_identity);

        // Generate signing identity
        let signing_identity = self
            .create_signing_identity(&private_key, &public_key, false)
            .await?;

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

    pub async fn create_group(&mut self) -> Result<Vec<u8>, MlsError> {
        debug!("Creating new MLS group");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        let group = client
            .create_group(ExtensionList::default(), Default::default(), None)
            .await?;

        let group_id = group.group_id().to_vec();
        self.group = Some(group);
        debug!(
            id = ?hex::encode(&group_id),
            "MLS group created successfully",
        );

        Ok(group_id)
    }

    pub async fn generate_key_package(&self) -> Result<KeyPackageMsg, MlsError> {
        debug!("Generating key package");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        let key_package = client
            .generate_key_package_message(Default::default(), Default::default(), None)
            .await?;

        let ret = key_package.to_bytes()?;

        Ok(ret)
    }

    pub async fn add_member(
        &mut self,
        key_package_bytes: &[u8],
    ) -> Result<MlsAddMemberResult, MlsError> {
        debug!("Adding member to the MLS group");
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        let key_package = MlsMessage::from_bytes(key_package_bytes)?;

        let identity_provider = SlimIdentityProvider::new(self.identity_verifier.clone());

        // create a set of the current identifiers in the group
        // to detect the new one after the insertion
        let old_roster = group.roster().members();
        let mut ids = HashSet::new();
        for m in old_roster {
            let identifier = identity_provider
                .identity(&m.signing_identity, &m.extensions)
                .await?;
            ids.insert(identifier);
        }

        let commit = group.commit_builder().add_member(key_package)?;
        let commit = commit.build().await?;

        // create the commit message to broadcast in the group
        let commit_msg = commit.commit_message.to_bytes()?;

        // extract and serialize the first welcome message
        let welcome = commit
            .welcome_messages
            .first()
            .ok_or(MlsError::NoWelcomeMessage)
            .and_then(|w| w.to_bytes().map_err(MlsError::from))?;

        // apply the commit locally
        group.apply_pending_commit().await?;

        let new_roster = group.roster().members();
        let mut new_id = vec![];
        for m in new_roster {
            let identifier = identity_provider
                .identity(&m.signing_identity, &m.extensions)
                .await?;
            if !ids.contains(&identifier) {
                new_id = identifier;
                break;
            }
        }

        let ret = MlsAddMemberResult {
            welcome_message: welcome,
            commit_message: commit_msg,
            member_identity: new_id,
        };
        Ok(ret)
    }

    pub async fn remove_member(&mut self, identity: &[u8]) -> Result<CommitMsg, MlsError> {
        debug!("Removing member from the MLS group");
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let m = group.member_with_identity(identity).await?;

        let commit = group.commit_builder().remove_member(m.index)?;
        let commit = commit.build().await?;

        let commit_msg = commit.commit_message.to_bytes()?;

        group.apply_pending_commit().await?;

        Ok(commit_msg)
    }

    pub async fn process_commit(&mut self, commit_message: &[u8]) -> Result<(), MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        let commit = MlsMessage::from_bytes(commit_message)?;

        // process an incoming commit message
        group.process_incoming_message(commit).await?;
        Ok(())
    }

    pub async fn process_welcome(&mut self, welcome_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Processing welcome message and joining MLS group");
        let client = self.client.as_ref().ok_or(MlsError::ClientNotInitialized)?;

        // process the welcome message and connect to the group
        let welcome = MlsMessage::from_bytes(welcome_message)?;
        let (group, _) = client.join_group(None, &welcome, None).await?;

        let group_id = group.group_id().to_vec();
        self.group = Some(group);
        debug!(
            id = %hex::encode(&group_id),
            "Successfully joined MLS group",
        );

        Ok(group_id)
    }

    pub async fn process_proposal(
        &mut self,
        proposal_message: &[u8],
        create_commit: bool,
    ) -> Result<CommitMsg, MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        let proposal = MlsMessage::from_bytes(proposal_message)?;

        group.process_incoming_message(proposal).await?;

        if !create_commit {
            debug!("process proposal but do not create commit. return empty commit");
            return Ok(vec![]);
        }

        // create commit message from proposal
        let commit = group.commit_builder().build().await?;

        // apply the commit locally
        group.apply_pending_commit().await?;

        // return the commit message
        let commit_msg = commit.commit_message.to_bytes()?;
        Ok(commit_msg)
    }

    pub async fn process_local_pending_proposal(&mut self) -> Result<CommitMsg, MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        // create commit message from proposal
        let commit = group.commit_builder().build().await?;

        // apply the commit locally
        group.apply_pending_commit().await?;

        // return the commit message
        let commit_msg = commit.commit_message.to_bytes()?;
        Ok(commit_msg)
    }

    pub async fn encrypt_message(&mut self, message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Encrypting MLS message");

        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let encrypted_msg = group
            .encrypt_application_message(message, Default::default())
            .await?;

        let msg = encrypted_msg.to_bytes()?;
        Ok(msg)
    }

    pub async fn decrypt_message(&mut self, encrypted_message: &[u8]) -> Result<Vec<u8>, MlsError> {
        debug!("Decrypting MLS message");

        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let message = MlsMessage::from_bytes(encrypted_message)?;

        match group.process_incoming_message(message).await? {
            ReceivedMessage::ApplicationMessage(app_msg) => Ok(app_msg.data().to_vec()),
            _ => Err(MlsError::verification_failed(
                "Message was not an application message",
            )),
        }
    }

    pub async fn write_to_storage(&mut self) -> Result<(), MlsError> {
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;
        group.write_to_storage().await?;
        Ok(())
    }

    pub fn get_group_id(&self) -> Option<Vec<u8>> {
        self.group.as_ref().map(|g| g.group_id().to_vec())
    }

    pub fn get_epoch(&self) -> Option<u64> {
        self.group.as_ref().map(|g| g.current_epoch())
    }

    pub async fn create_rotation_proposal(&mut self) -> Result<ProposalMsg, MlsError> {
        // Generate new key pair
        let (new_private_key, new_public_key) = Self::generate_key_pair().await?;

        // Create signing identity with token containing the new public key (will save to storage if path is set)
        let new_signing_identity = self
            .create_signing_identity(&new_private_key, &new_public_key, true)
            .await?;

        // Now get mutable reference to group after creating signing identity
        let group = self.group.as_mut().ok_or(MlsError::GroupNotExists)?;

        let update_proposal = group
            .propose_update_with_identity(new_private_key.clone(), new_signing_identity, vec![])
            .await?;

        debug!(
            "Created credential rotation proposal, stored new keys and incremented credential version"
        );

        let ret = update_proposal.to_bytes()?;

        Ok(ret)
    }

    /// Get a token from the identity provider
    pub fn get_token(&self) -> Result<String, MlsError> {
        let ret = self.identity_provider.get_token()?;

        Ok(ret)
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use mls_rs_core::identity::MemberValidationContext;
    use tokio::time;

    use crate::errors::MlsError;

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use std::thread;

    const SHARED_SECRET: &str = "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas";

    #[tokio::test]
    async fn test_mls_creation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
        );

        mls.initialize().await?;
        assert!(mls.client.is_some());
        assert!(mls.group.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_creation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
        );

        mls.initialize().await?;
        let _group_id = mls.create_group().await?;
        assert!(mls.client.is_some());
        assert!(mls.group.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_key_package_generation() -> Result<(), Box<dyn std::error::Error>> {
        let mut mls = Mls::new(
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
        );

        mls.initialize().await?;
        let key_package = mls.generate_key_package().await?;
        assert!(!key_package.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_messaging() -> Result<(), Box<dyn std::error::Error>> {
        // alice will work as moderator
        let mut alice = Mls::new(
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
        );
        let mut bob = Mls::new(
            SharedSecret::new("bob", SHARED_SECRET).unwrap(),
            SharedSecret::new("bob", SHARED_SECRET).unwrap(),
        );
        let mut charlie = Mls::new(
            SharedSecret::new("charlie", SHARED_SECRET).unwrap(),
            SharedSecret::new("charlie", SHARED_SECRET).unwrap(),
        );
        let mut daniel = Mls::new(
            SharedSecret::new("daniel", SHARED_SECRET).unwrap(),
            SharedSecret::new("daniel", SHARED_SECRET).unwrap(),
        );

        alice.initialize().await?;
        bob.initialize().await?;
        charlie.initialize().await?;
        daniel.initialize().await?;

        let group_id = alice.create_group().await?;

        // add bob to the group
        let bob_key_package = bob.generate_key_package().await?;
        let bob_add_res = alice.add_member(&bob_key_package).await?;

        let bob_group_id = bob.process_welcome(&bob_add_res.welcome_message).await?;
        assert_eq!(group_id, bob_group_id);

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message).await?;
        let decrypted = bob.decrypt_message(&encrypted).await?;

        assert_eq!(original_message, decrypted.as_slice());
        assert_ne!(original_message.to_vec(), encrypted);

        assert_eq!(alice.get_epoch().unwrap(), bob.get_epoch().unwrap());
        assert_eq!(alice.get_group_id().unwrap(), bob.get_group_id().unwrap());

        thread::sleep(time::Duration::from_millis(1000));

        // add charlie
        let charlie_key_package = charlie.generate_key_package().await?;
        let charlie_add_res = alice.add_member(&charlie_key_package).await?;

        bob.process_commit(&charlie_add_res.commit_message).await?;

        let charlie_group_id = charlie
            .process_welcome(&charlie_add_res.welcome_message)
            .await?;
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
        let encrypted = alice.encrypt_message(original_message).await?;
        let decrypted_1 = bob.decrypt_message(&encrypted).await?;
        let decrypted_2 = charlie.decrypt_message(&encrypted).await?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        let original_message = b"Hello from Charlie!";
        let encrypted = charlie.encrypt_message(original_message).await?;
        let decrypted_1 = bob.decrypt_message(&encrypted).await?;
        let decrypted_2 = alice.decrypt_message(&encrypted).await?;
        assert_eq!(original_message, decrypted_1.as_slice());
        assert_eq!(original_message, decrypted_2.as_slice());

        // remove bob
        let remove_msg = alice.remove_member(&bob_add_res.member_identity).await?;
        charlie.process_commit(&remove_msg).await?;
        bob.process_commit(&remove_msg).await?;
        assert_eq!(alice.get_epoch().unwrap(), charlie.get_epoch().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            charlie.get_group_id().unwrap()
        );

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message).await?;
        let decrypted = charlie.decrypt_message(&encrypted).await?;
        assert_eq!(original_message, decrypted.as_slice());

        let original_message = b"Hello from Charlie!";
        let encrypted = charlie.encrypt_message(original_message).await?;
        let decrypted = alice.decrypt_message(&encrypted).await?;
        assert_eq!(original_message, decrypted.as_slice());

        // add daniel and remove charlie
        let daniel_key_package = daniel.generate_key_package().await?;
        let daniel_add_res = alice.add_member(&daniel_key_package).await?;

        charlie
            .process_commit(&daniel_add_res.commit_message)
            .await?;

        let daniel_group_id = daniel
            .process_welcome(&daniel_add_res.welcome_message)
            .await?;
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

        let commit = alice
            .remove_member(&charlie_add_res.member_identity)
            .await?;

        daniel.process_commit(&commit).await?;
        assert_eq!(alice.get_epoch().unwrap(), daniel.get_epoch().unwrap());
        assert_eq!(
            alice.get_group_id().unwrap(),
            daniel.get_group_id().unwrap()
        );

        // test encrypt decrypt
        let original_message = b"Hello from Alice 1!";
        let encrypted = alice.encrypt_message(original_message).await?;
        let decrypted = daniel.decrypt_message(&encrypted).await?;
        assert_eq!(original_message, decrypted.as_slice());

        Ok(())
    }

    #[tokio::test]
    async fn test_decrypt_message() -> Result<(), Box<dyn std::error::Error>> {
        let mut alice = Mls::new(
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
            SharedSecret::new("alice", SHARED_SECRET).unwrap(),
        );
        let mut bob = Mls::new(
            SharedSecret::new("bob", SHARED_SECRET).unwrap(),
            SharedSecret::new("bob", SHARED_SECRET).unwrap(),
        );

        alice.initialize().await?;
        bob.initialize().await?;
        let _group_id = alice.create_group().await?;

        let bob_key_package = bob.generate_key_package().await?;
        let res = alice.add_member(&bob_key_package).await?;
        let _bob_group_id = bob.process_welcome(&res.welcome_message).await?;

        let message = b"Test message";
        let encrypted = alice.encrypt_message(message).await?;

        let decrypted = bob.decrypt_message(&encrypted).await?;
        assert_eq!(decrypted, message);

        Ok(())
    }

    #[tokio::test]
    async fn test_shared_secret_rotation_same_identity() -> Result<(), Box<dyn std::error::Error>> {
        let identity_a = SharedSecret::new("alice", SHARED_SECRET).unwrap();

        // make sure the token provider is rotating the tokens
        let token_a = identity_a.get_token().unwrap();
        let token_b = identity_a.get_token().unwrap();
        assert!(token_a != token_b);

        let mut alice = Mls::new(identity_a.clone(), identity_a.clone());

        let identity_b = SharedSecret::new("bob", SHARED_SECRET).unwrap();
        let mut bob = Mls::new(identity_b.clone(), identity_b.clone());

        alice.initialize().await?;
        bob.initialize().await?;
        let _group_id = alice.create_group().await?;

        let bob_key_package = bob.generate_key_package().await?;
        let result = alice.add_member(&bob_key_package).await?;
        let welcome_message = result.welcome_message;
        let _bob_group_id = bob.process_welcome(&welcome_message).await?;

        let message1 = b"Message with secret_v1";
        let encrypted1 = alice.encrypt_message(message1).await?;
        let decrypted1 = bob.decrypt_message(&encrypted1).await?;
        assert_eq!(decrypted1, message1);

        let mut alice_rotated_secret = Mls::new(
            SharedSecret::new(
                "alice",
                "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas123",
            )
            .unwrap(),
            SharedSecret::new(
                "alice",
                "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas123",
            )
            .unwrap(),
        );
        alice_rotated_secret.initialize().await?;

        let message2 = b"Message with rotated secret";
        let encrypted2_result = alice_rotated_secret.encrypt_message(message2).await;
        assert!(encrypted2_result.is_err());

        let message3 = b"Message from original alice after secret rotation";
        let encrypted3 = alice.encrypt_message(message3).await?;
        let decrypted3 = bob.decrypt_message(&encrypted3).await?;
        assert_eq!(decrypted3, message3);

        Ok(())
    }

    #[tokio::test]
    async fn test_full_credential_rotation_flow() -> Result<(), Box<dyn std::error::Error>> {
        let secret_m = SharedSecret::new("moderator", SHARED_SECRET).unwrap();
        let mut moderator = Mls::new(secret_m.clone(), secret_m.clone());
        moderator.initialize().await?;
        let _group_id = moderator.create_group().await?;

        let secret_a = SharedSecret::new("alice", SHARED_SECRET).unwrap();
        let mut alice = Mls::new(secret_a.clone(), secret_a.clone());
        alice.initialize().await?;

        let secret_b = SharedSecret::new("bob", SHARED_SECRET).unwrap();
        let mut bob = Mls::new(secret_b.clone(), secret_b.clone());
        bob.initialize().await?;

        // Moderator adds Alice to the group
        let alice_key_package = alice.generate_key_package().await?;
        let result = moderator.add_member(&alice_key_package).await?;
        let welcome_alice = result.welcome_message;
        let _alice_group_id = alice.process_welcome(&welcome_alice).await?;

        // Moderator adds Bob to the group
        let bob_key_package = bob.generate_key_package().await?;
        let result = moderator.add_member(&bob_key_package).await?;
        let commit_bob = result.commit_message;
        let welcome_bob = result.welcome_message;
        let _bob_group_id = bob.process_welcome(&welcome_bob).await?;

        // Only Alice needs to process Bob's addition (Bob wasn't in the group when Alice was added)
        alice.process_commit(&commit_bob).await?;

        let message1 = b"Message before rotation";
        let encrypted1 = alice.encrypt_message(message1).await?;
        let decrypted1 = bob.decrypt_message(&encrypted1).await?;
        assert_eq!(decrypted1, message1);

        // Alice create a proposal
        let rotation_proposal = alice.create_rotation_proposal().await?;

        // send proposal to the moderator
        let commit = moderator.process_proposal(&rotation_proposal, true).await?;
        // send proposal also to bob
        bob.process_proposal(&rotation_proposal, false).await?;

        // broadcast the commit message
        alice.process_commit(&commit).await?;
        bob.process_commit(&commit).await?;

        // Test messaging after rotation
        // Bob can decrypt Alice's encrypted message
        let message2 = b"Message after rotation from alice";
        let encrypted2 = alice.encrypt_message(message2).await?;
        let decrypted2 = bob.decrypt_message(&encrypted2).await?;
        assert_eq!(decrypted2, message2);

        // ... and Alice can decrypt Bob's encrypted message
        let message3 = b"Message after rotation from bob";
        let encrypted3 = bob.encrypt_message(message3).await?;
        let decrypted3 = alice.decrypt_message(&encrypted3).await?;
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

    // -------------------------------------------------------------------------
    // Test Helpers
    // -------------------------------------------------------------------------
    async fn init_identity(
        name: &str,
        _path: &str,
    ) -> Result<Mls<SharedSecret, SharedSecret>, Box<dyn std::error::Error>> {
        let mut mls = Mls::new(
            SharedSecret::new(name, SHARED_SECRET).unwrap(),
            SharedSecret::new(name, SHARED_SECRET).unwrap(),
        );
        mls.initialize().await?;
        Ok(mls)
    }

    fn extract_token_and_pubkey(mls: &Mls<SharedSecret, SharedSecret>) -> (&String, Vec<u8>) {
        let stored = mls
            .stored_identity
            .as_ref()
            .expect("stored identity exists");
        let token = stored
            .last_credential
            .as_ref()
            .expect("stored credential exists");
        (token, stored.public_key_bytes.clone())
    }

    async fn build_fake_identity_with_other_key(
        stolen_token: &str,
    ) -> (SigningIdentity, SignaturePublicKey) {
        let (_priv, attacker_pub) = Mls::<SharedSecret, SharedSecret>::generate_key_pair()
            .await
            .expect("key gen");
        let stolen_cred = BasicCredential::new(stolen_token.as_bytes().to_vec());
        let signing_id = SigningIdentity::new(stolen_cred.into_credential(), attacker_pub.clone());
        (signing_id, attacker_pub)
    }

    fn verify_token_embeds_pubkey(token: &str, expected_pubkey_bytes: &[u8]) {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD as BASE64;
        let verifier = SharedSecret::new("alice", SHARED_SECRET).unwrap();
        let claims_json: serde_json::Value = verifier.try_get_claims(token).expect("claims");
        let claims = IdentityClaims::from_json(&claims_json).expect("identity claims");
        assert_eq!(
            claims.public_key,
            BASE64.encode(expected_pubkey_bytes),
            "Token must embed expected public key"
        );
    }

    // -------------------------------------------------------------------------
    // TEST: Token-only theft with different attacker key triggers PublicKeyMismatch
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_identity_theft_attack() -> Result<(), Box<dyn std::error::Error>> {
        // Threat model: attacker steals Alice's token, NOT her private key.

        let mut alice = init_identity("alice", "/tmp/mls_test_security_alice").await?;
        let mut charlie = init_identity("charlie", "/tmp/mls_test_security_charlie").await?;

        // Group setup
        let _group_id = alice.create_group().await?;
        let charlie_key_package = charlie.generate_key_package().await?;
        let charlie_add_res = alice.add_member(&charlie_key_package).await?;
        charlie
            .process_welcome(&charlie_add_res.welcome_message)
            .await?;

        // Sanity application message
        let msg = b"Hello from the real Alice!";
        let encrypted = alice.encrypt_message(msg).await?;
        let decrypted = charlie.decrypt_message(&encrypted).await?;
        assert_eq!(decrypted, msg);

        // Extract stolen artifacts
        let (alice_token, alice_pub_bytes) = extract_token_and_pubkey(&alice);

        // Build fake identity with a different key
        let (fake_identity, attacker_pub) = build_fake_identity_with_other_key(alice_token).await;

        let alice_pub_b64 = BASE64.encode(&alice_pub_bytes);
        let attacker_pub_b64 = BASE64.encode(attacker_pub.as_ref());
        assert_ne!(
            alice_pub_b64, attacker_pub_b64,
            "Precondition: attacker key must differ"
        );

        // Validate
        let verifier = SharedSecret::new("alice", SHARED_SECRET).unwrap();
        let provider = SlimIdentityProvider::new(verifier.clone());
        let validation_res = provider
            .validate_member(&fake_identity, None, MemberValidationContext::None)
            .await;

        assert!(
            matches!(validation_res, Err(MlsError::PublicKeyMismatch { .. })),
            "Expected PublicKeyMismatch for stolen token + different key"
        );

        // Show claims mismatch
        let claims_json: serde_json::Value = verifier
            .try_get_claims(alice_token.as_str())
            .expect("claims parse");
        let claims = IdentityClaims::from_json(&claims_json).expect("claims map");
        assert_ne!(
            claims.public_key, attacker_pub_b64,
            "Token-bound key must differ from attacker key"
        );

        Ok(())
    }

    // -------------------------------------------------------------------------
    // TEST: Token + correct public key but wrong private key => MLS signature failure
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn test_signature_mismatch_stolen_token_wrong_private_key()
    -> Result<(), Box<dyn std::error::Error>> {
        // Attacker has Alice's token + public key, but not her private key.

        let alice = init_identity("alice", "/tmp/mls_test_sig_mismatch_alice").await?;
        let (alice_token, alice_pub_bytes) = extract_token_and_pubkey(&alice);

        // Verify token correctness
        verify_token_embeds_pubkey(alice_token, &alice_pub_bytes);

        // Attacker key pair
        let (attacker_priv, _attacker_pub) =
            Mls::<SharedSecret, SharedSecret>::generate_key_pair().await?;

        // Build signing identity using Alice's token + Alice's public key (public part matches)
        let alice_pub = SignaturePublicKey::new(alice_pub_bytes.clone());
        let stolen_cred = BasicCredential::new(alice_token.as_bytes().to_vec());
        let fake_identity = SigningIdentity::new(stolen_cred.into_credential(), alice_pub.clone());

        // Build MLS client with mismatched private key
        let verifier = SharedSecret::new("alice", SHARED_SECRET).unwrap();
        let crypto_provider = AwsLcCryptoProvider::default();
        let identity_provider = SlimIdentityProvider::new(verifier.clone());
        let client = Client::builder()
            .identity_provider(identity_provider)
            .crypto_provider(crypto_provider)
            .signing_identity(fake_identity, attacker_priv.clone(), CIPHERSUITE)
            .build();

        // Operations expected to fail
        let group_res = client
            .create_group(ExtensionList::default(), Default::default(), None)
            .await;

        let keypkg_res = client
            .generate_key_package_message(Default::default(), Default::default(), None)
            .await;

        assert!(
            group_res.is_err() || keypkg_res.is_err(),
            "Expected at least one MLS operation failure (signature mismatch)"
        );

        Ok(())
    }
}

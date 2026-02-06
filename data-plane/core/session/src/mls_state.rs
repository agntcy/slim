// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{BTreeMap, HashMap, btree_map::Entry};

// Third-party crates
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{
    ApplicationPayload, MlsPayload, ProtoMessage as Message, ProtoSessionMessageType,
};

// Local crate
use crate::{SessionError, common::MessageDirection};
use slim_datapath::messages::Name;
use slim_mls::{
    errors::MlsError,
    mls::{CommitMsg, KeyPackageMsg, Mls, MlsIdentity, ProposalMsg, WelcomeMsg},
};

#[derive(Debug)]
pub struct MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state for the channel of this endpoint
    /// the mls state should be created and initiated in the app
    /// so that it can be shared with the channel and the interceptors
    pub(crate) mls: Mls<P, V>,

    /// used only if Some(mls)
    pub(crate) group: Vec<u8>,

    /// last mls message id
    pub(crate) last_mls_msg_id: u32,

    /// map of stored commits and proposals
    pub(crate) stored_commits_proposals: BTreeMap<u32, Message>,
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) async fn new(mut mls: Mls<P, V>) -> Result<Self, SessionError> {
        mls.initialize().await?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        })
    }

    pub(crate) async fn generate_key_package(&mut self) -> Result<KeyPackageMsg, SessionError> {
        let ret = self.mls.generate_key_package().await?;
        Ok(ret)
    }

    pub(crate) async fn process_welcome_message(
        &mut self,
        msg: &Message,
    ) -> Result<(), SessionError> {
        if self.last_mls_msg_id != 0 {
            debug!("Welcome message already received, drop");
            // we already got a welcome message, ignore this one
            return Ok(());
        }

        let payload = msg.extract_group_welcome()?;
        let mls_payload = payload
            .mls
            .as_ref()
            .ok_or(SessionError::WelcomeMessageMissingMlsPayload)?;
        self.last_mls_msg_id = mls_payload.commit_id;
        let welcome = &mls_payload.mls_content;

        self.group = self.mls.process_welcome(welcome).await?;

        Ok(())
    }

    pub(crate) async fn process_control_message(
        &mut self,
        msg: Message,
        local_name: &Name,
    ) -> Result<bool, SessionError> {
        if !self.is_valid_msg_id(msg)? {
            // message already processed, drop it
            return Ok(false);
        }

        // process all messages in map until the numbering is not continuous
        while let Some(msg) = self
            .stored_commits_proposals
            .remove(&(self.last_mls_msg_id + 1))
        {
            trace!(id = %msg.get_id(), "processing stored message");

            // increment the last mls message id
            self.last_mls_msg_id += 1;

            // base on the message type, process it
            match msg.get_session_header().session_message_type() {
                ProtoSessionMessageType::GroupProposal => {
                    self.process_proposal_message(msg, local_name).await?;
                }
                ProtoSessionMessageType::GroupAdd => {
                    let payload = msg.extract_group_add()?;
                    let mls_payload = payload.mls.as_ref().ok_or(MlsError::NoGroupAddPayload)?;
                    self.process_commit_message(mls_payload).await?;
                }
                ProtoSessionMessageType::GroupRemove => {
                    let payload = msg.extract_group_remove()?;
                    let mls_payload = payload.mls.as_ref().ok_or(MlsError::NoGroupRemovePayload)?;

                    self.process_commit_message(mls_payload).await?;
                }
                _type => {
                    error!(?_type, "unknown control message type, drop it");
                    return Err(SessionError::SessionMessageTypeUnknown(
                        msg.get_session_header().session_message_type(),
                    ));
                }
            }
        }

        Ok(true)
    }

    async fn process_commit_message(
        &mut self,
        mls_payload: &MlsPayload,
    ) -> Result<(), SessionError> {
        trace!(id = %mls_payload.commit_id,  "processing stored commit",);

        // process the commit message
        self.mls.process_commit(&mls_payload.mls_content).await?;

        Ok(())
    }

    async fn process_proposal_message(
        &mut self,
        proposal: Message,
        local_name: &Name,
    ) -> Result<(), SessionError> {
        trace!(id = proposal.get_id(), "processing stored proposal");

        let payload = proposal.extract_group_proposal()?;

        let original_source = Name::from(payload.source.as_ref().ok_or(
            SessionError::MissingPayload {
                context: "proposal source",
            },
        )?);
        if original_source == *local_name {
            // drop the message as we are the original source
            debug!("Known proposal, drop the message");
            return Ok(());
        }

        self.mls
            .process_proposal(&payload.mls_proposal, false)
            .await?;

        Ok(())
    }

    fn is_valid_msg_id(&mut self, msg: Message) -> Result<bool, SessionError> {
        // the first message to be received should be a welcome message
        // this message will init the last_mls_msg_id. so if last_mls_msg_id = 0
        // drop the commits
        if self.last_mls_msg_id == 0 {
            debug!("welcome message not received yet, drop mls message");
            return Ok(false);
        }

        let command_payload = msg.extract_command_payload()?;

        let commit_id = match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::GroupAdd => {
                command_payload
                    .as_group_add_payload()?
                    .mls
                    .as_ref()
                    .ok_or(MlsError::NoGroupAddPayload)?
                    .commit_id
            }
            ProtoSessionMessageType::GroupRemove => {
                command_payload
                    .as_group_remove_payload()?
                    .mls
                    .as_ref()
                    .ok_or(MlsError::NoGroupRemovePayload)?
                    .commit_id
            }
            _ => {
                return Err(MlsError::UnknownPayloadType.into());
            }
        };

        if commit_id <= self.last_mls_msg_id {
            debug!(
                %commit_id, last_message_id = self.last_mls_msg_id,
                "Message already processed, drop it.",
            );
            return Ok(false);
        }

        // store commit in hash map
        match self.stored_commits_proposals.entry(commit_id) {
            Entry::Occupied(_) => {
                debug!(%commit_id, "Message already exists, drop it");
                Ok(false)
            }
            Entry::Vacant(entry) => {
                entry.insert(msg);
                Ok(true)
            }
        }
    }

    /// Checks if a message should be processed by MLS encryption/decryption
    fn should_process_message(msg: &Message) -> bool {
        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type");
            return false;
        }

        // Only process actual application data messages (Msg type)
        // Skip all control/session management messages
        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::Msg => {
                // This is an application data message, process it
                true
            }
            _ => {
                // Skip all other message types (control messages, ACKs, etc.)
                debug!(
                    "Skipping non-data message type: {:?}",
                    msg.get_session_header().session_message_type()
                );
                false
            }
        }
    }

    /// Processes a message based on direction (encrypt for South, decrypt for North)
    ///
    /// # Arguments
    /// * `msg` - Mutable reference to the message to process
    /// * `direction` - Direction of the message (North from SLIM, South to SLIM)
    ///
    /// # Returns
    /// * `Ok(())` if processing succeeds
    /// * `Err(SessionError)` if processing fails or message format is invalid
    pub async fn process_message(
        &mut self,
        msg: &mut Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match direction {
            MessageDirection::South => {
                // Encrypting message going to SLIM
                self.encrypt_message(msg).await
            }
            MessageDirection::North => {
                // Decrypting message coming from SLIM
                self.decrypt_message(msg).await
            }
        }
    }

    /// Encrypts a message payload using MLS
    ///
    /// # Arguments
    /// * `msg` - Mutable reference to the message to encrypt
    ///
    /// # Returns
    /// * `Ok(())` if encryption succeeds
    /// * `Err(SessionError)` if encryption fails or message format is invalid
    async fn encrypt_message(&mut self, msg: &mut Message) -> Result<(), SessionError> {
        if !Self::should_process_message(msg) {
            return Ok(());
        }

        let payload = msg.get_payload().unwrap().as_application_payload()?;

        debug!("Encrypting message for group member");
        let encrypted_payload = self.mls.encrypt_message(&payload.blob).await?;

        msg.set_payload(
            ApplicationPayload::new(&payload.payload_type, encrypted_payload.to_vec()).as_content(),
        );

        Ok(())
    }

    /// Decrypts a message payload using MLS
    ///
    /// # Arguments
    /// * `msg` - Mutable reference to the message to decrypt
    ///
    /// # Returns
    /// * `Ok(())` if decryption succeeds
    /// * `Err(SessionError)` if decryption fails or message format is invalid
    async fn decrypt_message(&mut self, msg: &mut Message) -> Result<(), SessionError> {
        if !Self::should_process_message(msg) {
            return Ok(());
        }

        let payload = msg.get_payload().unwrap().as_application_payload()?;

        debug!("Decrypting message for group member");
        let decrypted_payload = self.mls.decrypt_message(&payload.blob).await?;

        msg.set_payload(
            ApplicationPayload::new(&payload.payload_type, decrypted_payload.to_vec()).as_content(),
        );
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct MlsModeratorState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state in common between moderator and participants
    pub(crate) common: MlsState<P, V>,

    /// map of the participants (with real ids) with package keys
    /// used to remove participants from the channel
    pub(crate) participants: HashMap<Name, MlsIdentity>,

    /// message id of the next msl message to send
    pub(crate) next_msg_id: u32,
}

impl<P, V> MlsModeratorState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(mls: MlsState<P, V>) -> Self {
        MlsModeratorState {
            common: mls,
            participants: HashMap::new(),
            next_msg_id: 0,
        }
    }

    pub(crate) async fn init_moderator(&mut self) -> Result<(), SessionError> {
        self.common.mls.create_group().await?;
        Ok(())
    }

    pub(crate) async fn add_participant(
        &mut self,
        msg: &Message,
    ) -> Result<(CommitMsg, WelcomeMsg), SessionError> {
        let payload = msg.extract_join_reply()?;

        // Propagate MlsError directly (will become SessionError::MlsOp via #[from])
        let ret = self.common.mls.add_member(payload.key_package()).await?;

        // add participant to the list
        self.participants
            .insert(msg.get_source(), ret.member_identity);

        Ok((ret.commit_message, ret.welcome_message))
    }

    pub(crate) async fn remove_participant(
        &mut self,
        msg: &Message,
    ) -> Result<CommitMsg, SessionError> {
        debug!("Remove participant from the MLS group");
        let name = msg.get_dst();
        let id = match self.participants.get(&name) {
            Some(id) => id,
            None => {
                error!("the name does not exists in the group");
                return Err(SessionError::ParticipantNotFound(name));
            }
        };

        let ret = self.common.mls.remove_member(id).await?;

        // remove the participant from the list
        self.participants.remove(&name);

        Ok(ret)
    }

    #[allow(dead_code)]
    pub(crate) async fn process_proposal_message(
        &mut self,
        proposal: &ProposalMsg,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self.common.mls.process_proposal(proposal, true).await?;

        Ok(commit)
    }

    #[allow(dead_code)]
    pub(crate) async fn process_local_pending_proposal(
        &mut self,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self.common.mls.process_local_pending_proposal().await?;

        Ok(commit)
    }

    pub(crate) fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use slim_testing::utils::TEST_VALID_SECRET;

    #[tokio::test]
    async fn test_encrypt_without_group() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_encrypt_without_group"),
        );
        mls.initialize().await.unwrap();

        let mut mls_state = MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        };

        let mut msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "target",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", b"test message".to_vec())
            .build_publish()
            .unwrap();

        let result = mls_state.encrypt_message(&mut msg).await;
        assert!(result.is_err_and(|e| matches!(e, SessionError::MlsOp(_))));
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_with_group() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_alice"),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_bob"),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().await.unwrap();
        let bob_key_package = bob_mls.generate_key_package().await.unwrap();
        let ret = alice_mls.add_member(&bob_key_package).await.unwrap();
        bob_mls.process_welcome(&ret.welcome_message).await.unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        };

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "bob",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_state.encrypt_message(&mut alice_msg).await.unwrap();

        assert_ne!(
            alice_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );

        let mut bob_msg = alice_msg.clone();
        bob_state.decrypt_message(&mut bob_msg).await.unwrap();

        assert_eq!(
            bob_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );
    }

    #[tokio::test]
    async fn test_skip_non_publish_messages() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            std::path::PathBuf::from("/tmp/mls_helpers_test_skip_non_publish"),
        );
        mls.initialize().await.unwrap();
        let _group_id = mls.create_group().await.unwrap();

        let mut mls_state = MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        };

        // Create a message with a control message type (not Msg)
        let mut msg = Message::builder()
            .source(
                slim_datapath::messages::Name::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::messages::Name::from_strings([
                "org", "default", "target",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::JoinRequest)
            .application_payload("text", b"test message".to_vec())
            .build_publish()
            .unwrap();

        let original_payload = msg
            .get_payload()
            .unwrap()
            .as_application_payload()
            .unwrap()
            .blob
            .clone();

        // Should not encrypt
        mls_state.encrypt_message(&mut msg).await.unwrap();

        // Payload should remain unchanged
        assert_eq!(
            msg.get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            original_payload
        );
    }
}

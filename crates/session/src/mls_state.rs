// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{BTreeMap, HashMap, btree_map::Entry};

// Third-party crates
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{
    ApplicationPayload, HeaderIntegrityAad, MlsPayload, ProtoMessage as Message,
    ProtoSessionMessageType,
};

use crate::{
    SessionError, common::MessageDirection, common::OutboundMessage, common::SessionOutput,
    runtime::maybe_await,
};
use prost::Message as _;
use slim_datapath::api::ProtoName;
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

    /// percent of messages to verify after decrypt
    pub(crate) header_integrity_validation_percent: u32,
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub async fn new(
        mut mls: Mls<P, V>,
        header_integrity_validation_percent: u32,
    ) -> Result<Self, SessionError> {
        mls.initialize().await?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent,
        })
    }
}

#[maybe_async::maybe_async]
impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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
        local_name: &ProtoName,
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
        local_name: &ProtoName,
    ) -> Result<(), SessionError> {
        trace!(id = proposal.get_id(), "processing stored proposal");

        let payload = proposal.extract_group_proposal()?;

        let original_source = payload
            .source
            .as_ref()
            .ok_or(SessionError::MissingPayload {
                context: "proposal source",
            })?
            .clone();
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
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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
            ProtoSessionMessageType::Msg | ProtoSessionMessageType::RtxReply => {
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
}

/// Builds the Authenticated Data (AAD) for header integrity checks
pub(crate) fn build_aad(msg: &Message) -> Vec<u8> {
    let slim_header = msg.get_slim_header();
    let session_header = msg.get_session_header();

    let payload_type = if let Some(payload) = msg.get_payload() {
        if let Ok(app_payload) = payload.as_application_payload() {
            app_payload.payload_type.clone()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let aad = HeaderIntegrityAad {
        version: 1,
        source: Some(slim_header.get_source().clone()),
        destination: Some(slim_header.get_dst().clone()),
        identity: slim_header.get_identity().to_string(),
        session_type: session_header.session_type() as i32,
        session_message_type: session_header.session_message_type() as i32,
        session_id: session_header.get_session_id(),
        message_id: session_header.get_message_id(),
        payload_type,
    };

    aad.encode_to_vec()
}

/// Async MLS state operations (sync on native via `is_sync`, async on wasm32).
#[maybe_async::maybe_async]
impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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

    /// Apply MLS encryption to all outbound ToSlim messages in the output.
    pub async fn encrypt_output(&mut self, output: &mut SessionOutput) -> Result<(), SessionError> {
        for msg in &mut output.messages {
            if let OutboundMessage::ToSlim(m) = msg {
                self.process_message(m, MessageDirection::South).await?;
            }
        }
        Ok(())
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

        let payload = msg.get_payload().unwrap().into_application_payload()?;

        debug!("Encrypting message for group member");
        let aad = build_aad(msg);
        let encrypted_payload = self.mls.encrypt_message(&payload.blob, aad).await?;

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

        let payload = msg.get_payload().unwrap().into_application_payload()?;

        debug!("Decrypting message for group member");
        let (decrypted_payload, auth_data) = self.mls.decrypt_message(&payload.blob).await?;

        // Validate header integrity if enabled
        if self.header_integrity_validation_percent > 0 {
            let should_validate = if self.header_integrity_validation_percent >= 100 {
                true
            } else {
                (rand::random::<u32>() % 100) < self.header_integrity_validation_percent
            };

            if should_validate {
                let expected_aad = build_aad(msg);
                if expected_aad != auth_data {
                    let expected_decoded = HeaderIntegrityAad::decode(&expected_aad[..]);
                    let got_decoded = HeaderIntegrityAad::decode(&auth_data[..]);
                    error!(
                        "Header integrity validation failed! Expected AAD: {:?}, Got AAD: {:?}",
                        expected_decoded, got_decoded
                    );
                    return Err(MlsError::verification_failed("Header integrity mismatch").into());
                }
            }
        }

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
    pub(crate) participants: HashMap<ProtoName, MlsIdentity>,

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
        maybe_await!(self.common.mls.create_group())?;
        Ok(())
    }

    pub(crate) fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }
}

#[maybe_async::maybe_async]
impl<P, V> MlsModeratorState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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
        );
        mls.initialize().await.unwrap();

        let mut mls_state = MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        let mut msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
                "org", "default", "target",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", b"test message".to_vec())
            .build_publish()
            .unwrap();

        let result = mls_state.encrypt_message(&mut msg);
        assert!(result.is_err_and(|e| matches!(e, SessionError::MlsOp(_))));
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_with_group() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
                "org", "default", "bob",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_state.encrypt_message(&mut alice_msg).unwrap();

        assert_ne!(
            alice_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob.as_ref(),
            original_payload.as_ref()
        );

        let mut bob_msg = alice_msg.clone();
        bob_state.decrypt_message(&mut bob_msg).unwrap();

        assert_eq!(
            bob_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob.as_ref(),
            original_payload.as_ref()
        );
    }

    #[tokio::test]
    async fn test_skip_non_publish_messages() {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
        );
        mls.initialize().await.unwrap();
        let _group_id = mls.create_group().unwrap();

        let mut mls_state = MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        // Create a message with a control message type (not Msg)
        let mut msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "test"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
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
        mls_state.encrypt_message(&mut msg).unwrap();

        // Payload should remain unchanged
        assert_eq!(
            msg.get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob.as_ref(),
            original_payload.as_ref()
        );
    }

    #[tokio::test]
    async fn test_header_integrity_0_percent_skips_validation() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 0,
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 0,
        };

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
                "org", "default", "bob",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_state.encrypt_message(&mut alice_msg).unwrap();

        let mut tampered_msg = alice_msg.clone();
        tampered_msg.get_session_header_mut().message_id += 1;

        // Decryption should succeed because 0% means validation is skipped entirely.
        bob_state.decrypt_message(&mut tampered_msg).unwrap();

        assert_eq!(
            tampered_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob.as_ref(),
            original_payload.as_ref()
        );
    }

    #[tokio::test]
    async fn test_header_integrity_100_percent_always_fails_on_tampered() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        };

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
                "org", "default", "bob",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_state.encrypt_message(&mut alice_msg).unwrap();

        let mut tampered_msg = alice_msg.clone();
        tampered_msg.get_session_header_mut().message_id += 1;

        // Decryption should fail because validation runs and detects tampered header.
        let decrypt_result = bob_state.decrypt_message(&mut tampered_msg);
        assert!(decrypt_result.is_err());
    }

    #[tokio::test]
    async fn test_header_integrity_50_percent_stochastic_validation() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 50,
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 50,
        };

        let original_payload = b"Hello from Alice!";

        let mut failure_count = 0;
        let trials = 1000;

        for i in 0..trials {
            let mut alice_msg = Message::builder()
                .source(
                    slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"])
                        .with_id(0),
                )
                .destination(slim_datapath::api::ProtoName::from_strings([
                    "org", "default", "bob",
                ]))
                .session_id(1)
                .message_id(i as u32 + 1)
                .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
                .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
                .application_payload("text", original_payload.to_vec())
                .build_publish()
                .unwrap();

            alice_state.encrypt_message(&mut alice_msg).unwrap();

            let mut tampered_msg = alice_msg.clone();
            tampered_msg.get_session_header_mut().message_id += 1;

            if bob_state.decrypt_message(&mut tampered_msg).is_err() {
                failure_count += 1;
            }
        }

        // 50% rate over 1000 trials means we expect ~500 failures.
        // Assert we are within a very safe margin (e.g. 400 to 600) to prevent any test flakiness.
        assert!(
            (400..=600).contains(&failure_count),
            "Failure count {} was not within expected stochastic range [400, 600]",
            failure_count
        );
    }

    #[tokio::test]
    async fn test_header_integrity_always_passes_for_valid_messages() {
        for percent in [0, 50, 100, 150] {
            let mut alice_mls = Mls::new(
                SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            );
            let mut bob_mls = Mls::new(
                SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
                SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            );

            alice_mls.initialize().await.unwrap();
            bob_mls.initialize().await.unwrap();

            let _group_id = alice_mls.create_group().unwrap();
            let bob_key_package = bob_mls.generate_key_package().unwrap();
            let ret = alice_mls.add_member(&bob_key_package).unwrap();
            bob_mls.process_welcome(&ret.welcome_message).unwrap();

            let mut alice_state = MlsState {
                mls: alice_mls,
                group: vec![],
                last_mls_msg_id: 0,
                stored_commits_proposals: BTreeMap::new(),
                header_integrity_validation_percent: percent,
            };

            let mut bob_state = MlsState {
                mls: bob_mls,
                group: vec![],
                last_mls_msg_id: 0,
                stored_commits_proposals: BTreeMap::new(),
                header_integrity_validation_percent: percent,
            };

            let original_payload = b"Valid Message";

            let mut alice_msg = Message::builder()
                .source(
                    slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"])
                        .with_id(0),
                )
                .destination(slim_datapath::api::ProtoName::from_strings([
                    "org", "default", "bob",
                ]))
                .session_id(1)
                .message_id(1)
                .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
                .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
                .application_payload("text", original_payload.to_vec())
                .build_publish()
                .unwrap();

            alice_state.encrypt_message(&mut alice_msg).unwrap();

            let mut bob_msg = alice_msg.clone();
            // Decryption should always succeed for non-tampered messages, regardless of percent setting
            bob_state.decrypt_message(&mut bob_msg).unwrap();

            assert_eq!(
                bob_msg
                    .get_payload()
                    .unwrap()
                    .as_application_payload()
                    .unwrap()
                    .blob.as_ref(),
                original_payload.as_ref()
            );
        }
    }

    #[tokio::test]
    async fn test_header_integrity_clamped_above_100() {
        let mut alice_mls = Mls::new(
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("alice", TEST_VALID_SECRET).unwrap(),
        );
        let mut bob_mls = Mls::new(
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("bob", TEST_VALID_SECRET).unwrap(),
        );

        alice_mls.initialize().await.unwrap();
        bob_mls.initialize().await.unwrap();

        let _group_id = alice_mls.create_group().unwrap();
        let bob_key_package = bob_mls.generate_key_package().unwrap();
        let ret = alice_mls.add_member(&bob_key_package).unwrap();
        bob_mls.process_welcome(&ret.welcome_message).unwrap();

        let mut alice_state = MlsState {
            mls: alice_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 150, // above 100
        };

        let mut bob_state = MlsState {
            mls: bob_mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 150, // above 100
        };

        let original_payload = b"Hello from Alice!";

        let mut alice_msg = Message::builder()
            .source(
                slim_datapath::api::ProtoName::from_strings(["org", "default", "alice"]).with_id(0),
            )
            .destination(slim_datapath::api::ProtoName::from_strings([
                "org", "default", "bob",
            ]))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::Msg)
            .application_payload("text", original_payload.to_vec())
            .build_publish()
            .unwrap();

        alice_state.encrypt_message(&mut alice_msg).unwrap();

        let mut tampered_msg = alice_msg.clone();
        tampered_msg.get_session_header_mut().message_id += 1;

        // Decryption should fail because percent is clamped to behaves like 100%
        let decrypt_result = bob_state.decrypt_message(&mut tampered_msg);
        assert!(decrypt_result.is_err());
    }

    // ---- Control-message handling (no MLS crypto required) ----------------
    //
    // These exercise the participant-side ordering/dedup logic and the
    // moderator error path that surround the maybe_async MLS seam, without
    // needing a live MLS group.

    async fn new_test_mls_state() -> MlsState<SharedSecret, SharedSecret> {
        let mut mls = Mls::new(
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("test", TEST_VALID_SECRET).unwrap(),
        );
        mls.initialize().await.unwrap();
        MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            header_integrity_validation_percent: 100,
        }
    }

    fn test_name(leaf: &str) -> ProtoName {
        slim_datapath::api::ProtoName::from_strings(["org", "default", leaf])
    }

    /// Build a GroupAdd/GroupRemove control message carrying an MLS payload
    /// with the given `commit_id` (the MLS content itself is empty, which is
    /// fine because the ordering checks never inspect it).
    fn control_msg(msg_type: ProtoSessionMessageType, commit_id: u32) -> Message {
        use slim_datapath::api::{CommandPayload, Participant, ParticipantSettings};

        let mls = Some(MlsPayload {
            commit_id,
            mls_content: vec![],
        });
        let payload = match msg_type {
            ProtoSessionMessageType::GroupRemove => CommandPayload::builder()
                .group_remove(test_name("rem"), vec![], mls)
                .as_content(),
            _ => {
                let participant = Participant::new(
                    test_name("new").with_id(9),
                    ParticipantSettings::bidirectional(),
                );
                CommandPayload::builder()
                    .group_add(participant, vec![], mls)
                    .as_content()
            }
        };

        Message::builder()
            .source(test_name("mod").with_id(1))
            .destination(test_name("grp"))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::Multicast)
            .session_message_type(msg_type)
            .payload(payload)
            .build_publish()
            .unwrap()
    }

    #[tokio::test]
    async fn test_process_control_message_drops_before_welcome() {
        // last_mls_msg_id == 0 means no welcome processed yet, so any commit
        // must be dropped (it cannot be applied without the group state).
        let mut state = new_test_mls_state().await;
        let local = test_name("bob");
        let msg = control_msg(ProtoSessionMessageType::GroupAdd, 5);

        let processed = state.process_control_message(msg, &local).unwrap();

        assert!(!processed);
        assert_eq!(state.last_mls_msg_id, 0);
        assert!(state.stored_commits_proposals.is_empty());
    }

    #[tokio::test]
    async fn test_process_control_message_drops_already_processed_commit() {
        // commit_id (5) <= last_mls_msg_id (10): a stale/replayed commit.
        let mut state = new_test_mls_state().await;
        state.last_mls_msg_id = 10;
        let local = test_name("bob");
        let msg = control_msg(ProtoSessionMessageType::GroupRemove, 5);

        let processed = state.process_control_message(msg, &local).unwrap();

        assert!(!processed);
        assert!(state.stored_commits_proposals.is_empty());
    }

    #[tokio::test]
    async fn test_process_control_message_buffers_out_of_order_and_dedups() {
        // Expected next id is 2 but we receive 3: it must be buffered (not
        // applied) and a duplicate of it must be ignored.
        let mut state = new_test_mls_state().await;
        state.last_mls_msg_id = 1;
        let local = test_name("bob");
        let msg = control_msg(ProtoSessionMessageType::GroupAdd, 3);

        let processed = state.process_control_message(msg.clone(), &local).unwrap();
        assert!(processed);
        assert!(state.stored_commits_proposals.contains_key(&3));
        assert_eq!(state.last_mls_msg_id, 1, "gap means nothing is applied yet");

        let processed_again = state.process_control_message(msg, &local).unwrap();
        assert!(!processed_again, "duplicate commit_id is ignored");
        assert_eq!(state.stored_commits_proposals.len(), 1);
    }

    #[tokio::test]
    async fn test_process_welcome_message_ignored_when_already_joined() {
        use slim_datapath::api::CommandPayload;

        let mut state = new_test_mls_state().await;
        state.last_mls_msg_id = 7;
        let welcome = Message::builder()
            .source(test_name("mod").with_id(1))
            .destination(test_name("bob"))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        state.process_welcome_message(&welcome).unwrap();

        // A second welcome must not reset the already-established state.
        assert_eq!(state.last_mls_msg_id, 7);
    }

    #[tokio::test]
    async fn test_process_proposal_message_drops_local_origin() {
        use slim_datapath::api::CommandPayload;

        let mut state = new_test_mls_state().await;
        let local = test_name("self").with_id(2);
        let proposal = Message::builder()
            .source(local.clone())
            .destination(test_name("grp"))
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupProposal)
            .payload(
                CommandPayload::builder()
                    .group_proposal(Some(local.clone()), vec![])
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // Proposal originated locally: it is dropped without touching MLS.
        state.process_proposal_message(proposal, &local).unwrap();
    }

    #[tokio::test]
    async fn test_moderator_remove_participant_not_found() {
        use slim_datapath::api::CommandPayload;

        let mut moderator = MlsModeratorState::new(new_test_mls_state().await);
        let ghost = test_name("ghost");
        let msg = Message::builder()
            .source(test_name("mod").with_id(1))
            .destination(ghost.clone())
            .session_id(1)
            .message_id(1)
            .session_type(slim_datapath::api::ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupRemove)
            .payload(
                CommandPayload::builder()
                    .group_remove(ghost, vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let err = moderator.remove_participant(&msg).unwrap_err();
        assert!(matches!(err, SessionError::ParticipantNotFound(_)));
    }

    #[tokio::test]
    async fn test_build_aad_falls_back_to_empty_payload_type() {
        // A control message has no application payload, so build_aad must use
        // an empty payload_type rather than panicking.
        let msg = control_msg(ProtoSessionMessageType::GroupAdd, 1);

        let aad = build_aad(&msg);

        assert!(!aad.is_empty());
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{BTreeMap, HashMap, btree_map::Entry};

// Third-party crates
use bincode::{Decode, Encode};
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType};

// Local crate
use crate::SessionError;
use slim_datapath::messages::Name;
use slim_mls::mls::{CommitMsg, KeyPackageMsg, Mls, MlsIdentity, ProposalMsg, WelcomeMsg};

pub(crate) trait MlsEndpoint {
    /// check whether MLS is up
    fn is_mls_up(&self) -> Result<bool, SessionError>;

    /// rotate MLS keys
    async fn update_mls_keys(&mut self) -> Result<(), SessionError>;
}

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

    /// track if MLS is UP. For moderator this is true as soon as at least one participant
    /// has sent back an ack after the welcome message, while for participant
    /// this is true as soon as the welcome message is received and correctly processed
    pub(crate) mls_up: bool,
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(mls: Mls<P, V>) -> Self {
        MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            mls_up: false,
        }
    }

    pub(crate) fn generate_key_package(&mut self) -> Result<KeyPackageMsg, SessionError> {
        self.mls

            .generate_key_package()
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    pub(crate) async fn process_welcome_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.last_mls_msg_id != 0 {
            debug!("Welcome message already received, drop");
            // we already got a welcome message, ignore this one
            return Ok(());
        }

        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_welcome_payload();
        self.last_mls_msg_id = payload.msl_commit_id();
        let welcome = payload.mls_welcome();

        self.group = self
            .mls
            .process_welcome(welcome)
            .await
            .map_err(|e| SessionError::WelcomeMessage(e.to_string()))?;

        self.mls_up = true;

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
            trace!("processing stored message {}", msg.get_id());

            // increment the last mls message id
            self.last_mls_msg_id += 1;

            // base on the message type, process it
            match msg.get_session_header().session_message_type() {
                ProtoSessionMessageType::GroupProposal => {
                    self.process_proposal_message(msg, local_name).await?;
                }
                ProtoSessionMessageType::GroupUpdate => {
                    self.process_commit_message(msg).await?;
                }
                _ => {
                    error!("unknown control message type, drop it");
                    return Err(SessionError::Processing(
                        "unknown control message type".to_string(),
                    ));
                }
            }
        }

        Ok(true)
    }

    async fn process_commit_message(&mut self, commit: Message) -> Result<(), SessionError> {
        trace!("processing stored commit {}", commit.get_id());

        let payalod = commit
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_group_update_payload();
        // get the payload
        let commit = payalod.mls_commit();

        // process the commit message
        self.mls
            .process_commit(commit)
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))
    }

    async fn process_proposal_message(
        &mut self,
        proposal: Message,
        local_name: &Name,
    ) -> Result<(), SessionError> {
        trace!("processing stored proposal {}", proposal.get_id());

        let payload = proposal
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_group_proposal_payload();

        let original_source = Name::from(payload.source.as_ref().ok_or_else(|| {
            SessionError::Processing("missing source in proposal payload".to_string())
        })?);
        if original_source == *local_name {
            // drop the message as we are the original source
            debug!("Known proposal, drop the message");
            return Ok(());
        }

        self.mls
            .process_proposal(&payload.mls_proposal, false)
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(())
    }

    fn is_valid_msg_id(&mut self, msg: Message) -> Result<bool, SessionError> {
        // the first message to be received should be a welcome message
        // this message will init the last_mls_msg_id. so if last_mls_msg_id = 0
        // drop the commits
        if self.last_mls_msg_id == 0 {
            error!("welcome message not received yet, drop mls message");
            return Err(SessionError::MLSIdMessage(
                "welcome message not received yet, drop mls message".to_string(),
            ));
        }

        if msg.get_id() <= self.last_mls_msg_id {
            debug!(
                "Message with id {} already processed, drop it. last message id {}",
                msg.get_id(),
                self.last_mls_msg_id
            );
            return Ok(false);
        }

        // store commit in hash map
        match self.stored_commits_proposals.entry(msg.get_id()) {
            Entry::Occupied(_) => {
                debug!("Message with id {} already exists, drop it", msg.get_id());
                Ok(false)
            }
            Entry::Vacant(entry) => {
                entry.insert(msg);
                Ok(true)
            }
        }
    }

    async fn decrypt_message<P, V>(
        &self,
        message: &mut Message,
    ) -> Result<(), SessionError>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        let mls = match mls {
            Some(mls) => mls,
            None => {
                debug!("No MLS instance available for decryption, skipping");
                return Ok(());
            }
        };

        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in decryption path");
            return Ok(());
        }

        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck => {
                debug!("Skipping channel messages type in decryption path");
                return Ok(());
            }
            _ => {}
        }

        let payload = &msg.get_payload().unwrap().as_application_payload().blob;

        debug!("Decrypting message for group member");
        let decrypted_payload = match self.mls.decrypt_message(payload).await {
            Ok(decrypted_payload) => decrypted_payload,
            Err(e) => {
                error!("Failed to decrypt message with MLS: {}", e);
                return Err(SessionError::MlsDecryptionFailed(e.to_string()));
            }
        };

        msg.set_payload(ApplicationPayload::new("", decrypted_payload.to_vec()).as_content());

        Ok(())
    }

    async fn encrypt_messags(
        &self,
        message: &mut Message,
    ) -> Result<(), SessionError>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        let mls = match mls {
            Some(mls) => mls,
            None => {
                debug!("No MLS instance available for enryption, skipping");
                return Ok(());
            }
        };

        // Only process Publish message types
        if !msg.is_publish() {
            debug!("Skipping non-Publish message type in encryption path");
            return Ok(());
        }

        match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck => {
                debug!("Skipping channel messages type in encryption path");
                return Ok(());
            }
            _ => {}
        }

        let payload = &msg.get_payload().unwrap().as_application_payload().blob;

        debug!("Encrypting message for group member");
        let binding = self.mls.encrypt_message(payload).await;
        let encrypted_payload = match &binding {
            Ok(res) => res,
            Err(e) => {
                error!(
                    "Failed to encrypt message with MLS: {}, dropping message",
                    e
                );
                return Err(SessionError::MlsEncryptionFailed(e.to_string()));
            }
        };

        msg.set_payload(ApplicationPayload::new("", encrypted_payload.to_vec()).as_content());

        Ok(())
    }

    pub(crate) fn is_mls_up(&self) -> Result<bool, SessionError> {
        Ok(self.mls_up)
    }
}

#[derive(Debug)]
pub(crate) struct MlsModeratorState<'a, P, V>
where
    P: 'a + TokenProvider + Send + Sync + Clone + 'static,
    V: 'a + Verifier + Send + Sync + Clone + 'static,
{
    /// mls state in common between moderator and
    pub(crate) common: MlsState<'a, P, V>,

    /// map of the participants (with real ids) with package keys
    /// used to remove participants from the channel
    pub(crate) participants: HashMap<Name, MlsIdentity>,

    /// message id of the next msl message to send
    pub(crate) next_msg_id: u32,
}

impl<'a, P, V> MlsModeratorState<'a, P, V>
where
    P: 'a + TokenProvider + Send + Sync + Clone + 'static,
    V: 'a + Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(mls: MlsState<'a, P, V>) -> Self {
        MlsModeratorState {
            common: mls,
            participants: HashMap::new(),
            next_msg_id: 0,
        }
    }

    pub(crate) async fn init_moderator(&mut self) -> Result<(), SessionError> {
        self.common
            .mls
            .create_group()
            .await
            .map(|_| ())
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    pub(crate) async fn add_participant(
        &mut self,
        msg: &Message,
    ) -> Result<(CommitMsg, WelcomeMsg), SessionError> {
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_join_reply_payload();

        match self.common.mls.add_member(payload.key_package()).await {
            Ok(ret) => {
                // add participant to the list
                self.participants
                    .insert(msg.get_source(), ret.member_identity);

                Ok((ret.commit_message, ret.welcome_message))
            }
            Err(e) => {
                error!(%e, "error adding new endpoint");
                Err(SessionError::AddParticipant(e.to_string()))
            }
        }
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
                return Err(SessionError::RemoveParticipant(
                    "participant does not exists".to_owned(),
                ));
            }
        };
        let ret = self
            .common
            .mls
            .remove_member(id)
            .await
            .map_err(|e| SessionError::RemoveParticipant(e.to_string()))?;

        // remove the participant from the list
        self.participants.remove(&name);

        Ok(ret)
    }

    pub(crate) async fn process_proposal_message(
        &mut self,
        proposal: &ProposalMsg,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .process_proposal(proposal, true)
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    pub(crate) async fn process_local_pending_proposal(
        &mut self,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .process_local_pending_proposal()
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    pub(crate) fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }

    pub(crate) fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.common.is_mls_up()
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MlsProposalMessagePayload {
    pub(crate) source_name: Name,
    pub(crate) mls_msg: Vec<u8>,
}

impl MlsProposalMessagePayload {
    pub(crate) fn new(source_name: Name, mls_msg: Vec<u8>) -> Self {
        MlsProposalMessagePayload {
            source_name,
            mls_msg,
        }
    }
}

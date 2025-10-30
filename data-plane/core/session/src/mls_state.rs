// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    sync::Arc,
};

// Third-party crates
use parking_lot::Mutex;
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{MlsPayload, ProtoMessage as Message, ProtoSessionMessageType};

// Local crate
use crate::SessionError;
use slim_datapath::messages::Name;
use slim_mls::mls::{CommitMsg, KeyPackageMsg, Mls, MlsIdentity, ProposalMsg, WelcomeMsg};

#[derive(Debug)]
pub struct MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state for the channel of this endpoint
    /// the mls state should be created and initiated in the app
    /// so that it can be shared with the channel and the interceptors
    pub(crate) mls: Arc<Mutex<Mls<P, V>>>,

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
    pub(crate) fn new(mls: Arc<Mutex<Mls<P, V>>>) -> Result<Self, SessionError> {
        mls.lock()
            .initialize()
            .map_err(|e| SessionError::MLSInit(e.to_string()))?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
            mls_up: false,
        })
    }

    pub(crate) fn generate_key_package(&mut self) -> Result<KeyPackageMsg, SessionError> {
        self.mls
            .lock()
            .generate_key_package()
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    pub(crate) fn process_welcome_message(&mut self, msg: &Message) -> Result<(), SessionError> {
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
        let mls_payload = payload.mls.ok_or_else(|| {
            SessionError::WelcomeMessage("missing mls payload in welcome message".to_string())
        })?;
        self.last_mls_msg_id = mls_payload.commit_id;
        let welcome = mls_payload.mls_content;

        self.group = self
            .mls
            .lock()
            .process_welcome(&welcome)
            .map_err(|e| SessionError::WelcomeMessage(e.to_string()))?;

        self.mls_up = true;

        Ok(())
    }

    pub(crate) fn process_control_message(
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
                    self.process_proposal_message(msg, local_name)?;
                }
                ProtoSessionMessageType::GroupAdd => {
                    let payload = msg
                        .get_payload()
                        .unwrap()
                        .as_command_payload()
                        .as_group_add_payload();
                    let mls_payload = payload.mls.ok_or_else(|| {
                        SessionError::Processing("missing mls payload in add message".to_string())
                    })?;
                    self.process_commit_message(msg, mls_payload)?;
                }
                ProtoSessionMessageType::GroupRemove => {
                    let payload = msg
                        .get_payload()
                        .unwrap()
                        .as_command_payload()
                        .as_group_add_payload();
                    let mls_payload = payload.mls.ok_or_else(|| {
                        SessionError::Processing(
                            "missing mls payload in remove message".to_string(),
                        )
                    })?;

                    self.process_commit_message(msg, mls_payload)?;
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

    fn process_commit_message(
        &mut self,
        _msg: Message,
        mls_payload: MlsPayload,
    ) -> Result<(), SessionError> {
        trace!("processing stored commit {}", mls_payload.commit_id);

        // process the commit message
        self.mls
            .lock()
            .process_commit(&mls_payload.mls_content)
            .map_err(|e| SessionError::CommitMessage(e.to_string()))
    }

    fn process_proposal_message(
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
            .lock()
            .process_proposal(&payload.mls_proposal, false)
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

        let commit_id = match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::GroupAdd => {
                msg.get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_group_add_payload()
                    .mls
                    .ok_or_else(|| SessionError::MLSIdMessage("missing mls payload".to_string()))?
                    .commit_id
            }
            ProtoSessionMessageType::GroupRemove => {
                msg.get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_group_remove_payload()
                    .mls
                    .ok_or_else(|| SessionError::MLSIdMessage("missing mls payload".to_string()))?
                    .commit_id
            }
            _ => {
                return Err(SessionError::MLSIdMessage(
                    "unexpected message type".to_string(),
                ));
            }
        };

        if commit_id <= self.last_mls_msg_id {
            debug!(
                "Message with id {} already processed, drop it. last message id {}",
                commit_id, self.last_mls_msg_id
            );
            return Ok(false);
        }

        // store commit in hash map
        match self.stored_commits_proposals.entry(commit_id) {
            Entry::Occupied(_) => {
                debug!("Message with id {} already exists, drop it", commit_id);
                Ok(false)
            }
            Entry::Vacant(entry) => {
                entry.insert(msg);
                Ok(true)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct MlsModeratorState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state in common between moderator and
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

    pub(crate) fn init_moderator(&mut self) -> Result<(), SessionError> {
        self.common
            .mls
            .lock()
            .create_group()
            .map(|_| ())
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    pub(crate) fn add_participant(
        &mut self,
        msg: &Message,
    ) -> Result<(CommitMsg, WelcomeMsg), SessionError> {
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_join_reply_payload();

        match self.common.mls.lock().add_member(payload.key_package()) {
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

    pub(crate) fn remove_participant(&mut self, msg: &Message) -> Result<CommitMsg, SessionError> {
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
            .lock()
            .remove_member(id)
            .map_err(|e| SessionError::RemoveParticipant(e.to_string()))?;

        // remove the participant from the list
        self.participants.remove(&name);

        Ok(ret)
    }

    #[allow(dead_code)]
    pub(crate) fn process_proposal_message(
        &mut self,
        proposal: &ProposalMsg,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .lock()
            .process_proposal(proposal, true)
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    #[allow(dead_code)]
    pub(crate) fn process_local_pending_proposal(&mut self) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .lock()
            .process_local_pending_proposal()
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    pub(crate) fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }
}

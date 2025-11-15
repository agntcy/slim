// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    sync::Arc,
};

// Third-party crates
use tokio::sync::Mutex;
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
}

impl<P, V> MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) async fn new(mls: Arc<Mutex<Mls<P, V>>>) -> Result<Self, SessionError> {
        mls.lock()
            .await
            .initialize()
            .await
            .map_err(|e| SessionError::MLSInit(e.to_string()))?;

        Ok(MlsState {
            mls,
            group: vec![],
            last_mls_msg_id: 0,
            stored_commits_proposals: BTreeMap::new(),
        })
    }

    pub(crate) async fn generate_key_package(&mut self) -> Result<KeyPackageMsg, SessionError> {
        self.mls
            .lock()
            .await
            .generate_key_package()
            .await
            .map_err(|e| SessionError::MLSInit(e.to_string()))
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

        let payload = msg.extract_group_welcome().map_err(|e| {
            SessionError::WelcomeMessage(format!("failed to extract welcome payload: {}", e))
        })?;
        let mls_payload = payload.mls.as_ref().ok_or_else(|| {
            SessionError::WelcomeMessage("missing mls payload in welcome message".to_string())
        })?;
        self.last_mls_msg_id = mls_payload.commit_id;
        let welcome = &mls_payload.mls_content;

        self.group = self
            .mls
            .lock()
            .await
            .process_welcome(welcome)
            .await
            .map_err(|e| SessionError::WelcomeMessage(e.to_string()))?;

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
                ProtoSessionMessageType::GroupAdd => {
                    let payload = msg.extract_group_add().map_err(|e| {
                        SessionError::Processing(format!(
                            "failed to extract group add payload: {}",
                            e
                        ))
                    })?;
                    let mls_payload = payload.mls.as_ref().ok_or_else(|| {
                        SessionError::Processing("missing mls payload in add message".to_string())
                    })?;
                    self.process_commit_message(mls_payload).await?;
                }
                ProtoSessionMessageType::GroupRemove => {
                    let payload = msg.extract_group_remove().map_err(|e| {
                        SessionError::Processing(format!(
                            "failed to extract group remove payload: {}",
                            e
                        ))
                    })?;
                    let mls_payload = payload.mls.as_ref().ok_or_else(|| {
                        SessionError::Processing(
                            "missing mls payload in remove message".to_string(),
                        )
                    })?;

                    self.process_commit_message(mls_payload).await?;
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

    async fn process_commit_message(
        &mut self,
        mls_payload: &MlsPayload,
    ) -> Result<(), SessionError> {
        trace!("processing stored commit {}", mls_payload.commit_id);

        // process the commit message
        self.mls
            .lock()
            .await
            .process_commit(&mls_payload.mls_content)
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))
    }

    async fn process_proposal_message(
        &mut self,
        proposal: Message,
        local_name: &Name,
    ) -> Result<(), SessionError> {
        trace!("processing stored proposal {}", proposal.get_id());

        let payload = proposal.extract_group_proposal().map_err(|e| {
            SessionError::Processing(format!("failed to extract group proposal payload: {}", e))
        })?;

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
            .await
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
            debug!("welcome message not received yet, drop mls message");
            return Ok(false);
        }

        let command_payload = msg.extract_command_payload().map_err(|e| {
            SessionError::MLSIdMessage(format!("failed to extract command payload: {}", e))
        })?;

        let commit_id = match msg.get_session_header().session_message_type() {
            ProtoSessionMessageType::GroupAdd => {
                command_payload
                    .as_group_add_payload()?
                    .mls
                    .as_ref()
                    .ok_or_else(|| SessionError::MLSIdMessage("missing mls payload".to_string()))?
                    .commit_id
            }
            ProtoSessionMessageType::GroupRemove => {
                command_payload
                    .as_group_remove_payload()?
                    .mls
                    .as_ref()
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
        self.common
            .mls
            .lock()
            .await
            .create_group()
            .await
            .map(|_| ())
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    pub(crate) async fn add_participant(
        &mut self,
        msg: &Message,
    ) -> Result<(CommitMsg, WelcomeMsg), SessionError> {
        let payload = msg.extract_join_reply().map_err(|e| {
            SessionError::AddParticipant(format!("failed to extract join reply payload: {}", e))
        })?;

        match self
            .common
            .mls
            .lock()
            .await
            .add_member(payload.key_package())
            .await
        {
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
            .lock()
            .await
            .remove_member(id)
            .await
            .map_err(|e| SessionError::RemoveParticipant(e.to_string()))?;

        // remove the participant from the list
        self.participants.remove(&name);

        Ok(ret)
    }

    #[allow(dead_code)]
    pub(crate) async fn process_proposal_message(
        &mut self,
        proposal: &ProposalMsg,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .lock()
            .await
            .process_proposal(proposal, true)
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    #[allow(dead_code)]
    pub(crate) async fn process_local_pending_proposal(
        &mut self,
    ) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .lock()
            .await
            .process_local_pending_proposal()
            .await
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    pub(crate) fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }
}

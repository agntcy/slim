// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{BTreeMap, HashMap, btree_map::Entry};

// Third-party crates
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{MlsPayload, ProtoMessage as Message, ProtoSessionMessageType};

// Local crate
use crate::SessionError;
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

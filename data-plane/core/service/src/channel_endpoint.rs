// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use bincode::{Decode, Encode};
use parking_lot::Mutex;
use tracing::{debug, error, info, trace};

use crate::{
    errors::SessionError,
    interceptor_mls::{METADATA_MLS_ENABLED, METADATA_MLS_INIT_COMMIT_ID},
    session::{Id, SessionTransmitter},
};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType, SessionHeader,
        SlimHeader,
    },
    messages::{Agent, AgentType, utils::SlimHeaderFlags},
};
use slim_mls::mls::{CommitMsg, KeyPackageMsg, Mls, MlsIdentity, ProposalMsg, WelcomeMsg};

struct RequestTimerObserver<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// message to send in case of timeout
    message: Message,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

#[async_trait]
impl<T> crate::timer::TimerObserver for RequestTimerObserver<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        trace!("timeout number {} for request {}", timeouts, timer_id);

        if self
            .tx
            .send_to_slim(Ok(self.message.clone()))
            .await
            .is_err()
        {
            error!("error sending invite message");
        }
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        error!(?self.message, "unable to send message, stop retrying");
        self.tx
            .send_to_app(Err(SessionError::Processing(
                "timer failed on channel endpoint. Stop sending messages".to_string(),
            )))
            .await
            .expect("error notifying app");
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!(%timer_id, "timer for rtx cancelled");
        // nothing to do
    }
}

trait OnMessageReceived {
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError>;
}

pub(crate) trait MlsEndpoint {
    /// check whether MLS is up
    fn is_mls_up(&self) -> Result<bool, SessionError>;

    /// rotate MLS keys
    async fn update_mls_keys(&mut self) -> Result<(), SessionError>;
}

impl<P, V, T> MlsEndpoint for ChannelEndpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    fn is_mls_up(&self) -> Result<bool, SessionError> {
        match self {
            ChannelEndpoint::ChannelParticipant(cp) => cp.is_mls_up(),
            ChannelEndpoint::ChannelModerator(cm) => cm.is_mls_up(),
        }
    }

    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        match self {
            ChannelEndpoint::ChannelParticipant(cp) => cp.update_mls_keys().await,
            ChannelEndpoint::ChannelModerator(cm) => cm.update_mls_keys().await,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ChannelEndpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    ChannelParticipant(ChannelParticipant<P, V, T>),
    ChannelModerator(ChannelModerator<P, V, T>),
}

impl<P, V, T> ChannelEndpoint<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        match self {
            ChannelEndpoint::ChannelParticipant(cp) => cp.on_message(msg).await,
            ChannelEndpoint::ChannelModerator(cm) => cm.on_message(msg).await,
        }
    }
}

#[derive(Debug)]
pub(crate) struct MlsState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state for the channel of this endpoint
    /// the mls state should be created and initiated in the app
    /// so that it can be shared with the channel and the interceptors
    mls: Arc<Mutex<Mls<P, V>>>,

    /// used only if Some(mls)
    group: Vec<u8>,

    /// last mls message id
    last_mls_msg_id: u32,

    /// map of stored commits and proposals
    stored_commits_proposals: BTreeMap<u32, Message>,

    /// track if MLS is UP. For moderator this is true as soon as at least one participant
    /// has sent back an ack after the welcome message, while for participant
    /// this is true as soon as the welcome message is received and correctly processed
    mls_up: bool,
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

    fn generate_key_package(&mut self) -> Result<KeyPackageMsg, SessionError> {
        self.mls
            .lock()
            .generate_key_package()
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    fn process_welcome_message(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.last_mls_msg_id != 0 {
            debug!("welcome message already received, drop");
            // we already got a welcome message, ignore this one
            return Ok(());
        }

        self.last_mls_msg_id = msg
            .get_metadata(METADATA_MLS_INIT_COMMIT_ID)
            .ok_or(SessionError::WelcomeMessage(
                "received welcome message without commit id, drop it".to_string(),
            ))?
            .parse::<u32>()
            .map_err(|_| {
                SessionError::WelcomeMessage(
                    "received welcome message with invalid commit id, drop it".to_string(),
                )
            })?;

        let welcome = &msg
            .get_payload()
            .ok_or(SessionError::WelcomeMessage(
                "missing payload in MLS welcome, cannot join the group".to_string(),
            ))?
            .blob;

        self.group = self
            .mls
            .lock()
            .process_welcome(welcome)
            .map_err(|e| SessionError::WelcomeMessage(e.to_string()))?;

        self.mls_up = true;

        Ok(())
    }

    fn process_control_message(
        &mut self,
        msg: Message,
        local_name: &Agent,
    ) -> Result<(), SessionError> {
        self.is_valid_msg_id(msg)?;

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
                ProtoSessionMessageType::ChannelMlsProposal => {
                    self.process_proposal_message(msg, local_name)?;
                }
                ProtoSessionMessageType::ChannelMlsCommit => {
                    self.process_commit_message(msg)?;
                }
                _ => {
                    error!("unknown control message type, drop it");
                    return Err(SessionError::Processing(
                        "unknown control message type".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    fn process_commit_message(&mut self, commit: Message) -> Result<(), SessionError> {
        trace!("processing stored commit {}", commit.get_id());

        // get the payload
        let commit = &commit
            .get_payload()
            .ok_or(SessionError::CommitMessage(
                "missing payload in MLS commit, cannot process the commit".to_string(),
            ))?
            .blob;

        // process the commit message
        self.mls
            .lock()
            .process_commit(commit)
            .map_err(|e| SessionError::CommitMessage(e.to_string()))
    }

    fn process_proposal_message(
        &mut self,
        proposal: Message,
        local_name: &Agent,
    ) -> Result<(), SessionError> {
        trace!("processing stored proposal {}", proposal.get_id());

        let content = proposal
            .get_payload()
            .map_or_else(
                || {
                    error!("missing payload in a Mls Proposal, ignore the message");
                    Err(SessionError::Processing(
                        "missing payload in a Mls Proposal".to_string(),
                    ))
                },
                |content| -> Result<(MlsProposalMessagePayload, usize), SessionError> {
                    bincode::decode_from_slice(&content.blob, bincode::config::standard())
                        .map_err(|e| SessionError::ParseProposalMessage(e.to_string()))
                },
            )?
            .0;

        if content.source_name == *local_name {
            // drop the message as we are the original source
            debug!("known proposal, drop the message");
            return Ok(());
        }

        self.mls
            .lock()
            .process_proposal(&content.mls_msg, false)
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(())
    }

    fn is_valid_msg_id(&mut self, msg: Message) -> Result<(), SessionError> {
        // the first message to be received should be a welcome message
        // this message will init the last_mls_msg_id. so if last_mls_msg_id = 0
        // drop the commits
        if self.last_mls_msg_id == 0 {
            error!("welcome message not received yet, drop mls message");
            return Err(SessionError::MLSIdMessage(
                "welcome message not received yet, drop mls message".to_string(),
            ));
        }

        if msg.get_id() < self.last_mls_msg_id {
            debug!(
                "message with id {} already processed, drop it",
                msg.get_id()
            );
            return Ok(());
        }

        // store commit in hash map
        match self.stored_commits_proposals.entry(msg.get_id()) {
            Entry::Occupied(_) => {
                debug!("message with id {} already exists, drop it", msg.get_id());
                Err(SessionError::CommitMessage(
                    "commit message already exists, drop it".to_string(),
                ))
            }
            Entry::Vacant(entry) => {
                entry.insert(msg);
                Ok(())
            }
        }
    }

    fn on_mls_ack(&mut self) -> Result<(), SessionError> {
        // this is called by the moderator when the participant
        // sends back an ack after the welcome message
        self.mls_up = true;

        Ok(())
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        Ok(self.mls_up)
    }
}

#[derive(Debug)]
pub(crate) struct MlsModeratorState<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// mls state in common between moderator and
    common: MlsState<P, V>,

    /// map of the participants with package keys
    /// used to remove participants from the channel
    participants: HashMap<Agent, MlsIdentity>,

    /// message id of the next msl message to send
    next_msg_id: u32,
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

    fn init_moderator(&mut self) -> Result<(), SessionError> {
        self.common
            .mls
            .lock()
            .create_group()
            .map(|_| ())
            .map_err(|e| SessionError::MLSInit(e.to_string()))
    }

    fn add_participant(&mut self, msg: &Message) -> Result<(CommitMsg, WelcomeMsg), SessionError> {
        let payload = &msg
            .get_payload()
            .ok_or(SessionError::AddParticipant(
                "key package is missing. the end point cannot be added to the channel".to_string(),
            ))?
            .blob;

        match self.common.mls.lock().add_member(payload) {
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

    fn remove_participant(&mut self, msg: &Message) -> Result<CommitMsg, SessionError> {
        debug!("remove participant from the MLS group");
        let name = msg.get_name_as_agent();
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

    fn process_proposal_message(
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

    fn process_local_pending_proposal(&mut self) -> Result<CommitMsg, SessionError> {
        let commit = self
            .common
            .mls
            .lock()
            .process_local_pending_proposal()
            .map_err(|e| SessionError::CommitMessage(e.to_string()))?;

        Ok(commit)
    }

    fn get_next_mls_mgs_id(&mut self) -> u32 {
        self.next_msg_id += 1;
        self.next_msg_id
    }

    fn on_mls_ack(&mut self) -> Result<(), SessionError> {
        self.common.on_mls_ack()
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.common.is_mls_up()
    }
}

#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct JoinMessagePayload {
    channel_name: AgentType,
    channel_id: Option<u64>,
    moderator_name: Agent,
}

impl JoinMessagePayload {
    fn new(channel_name: AgentType, channel_id: Option<u64>, moderator_name: Agent) -> Self {
        JoinMessagePayload {
            channel_name,
            channel_id,
            moderator_name,
        }
    }
}

#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct MlsProposalMessagePayload {
    source_name: Agent,
    mls_msg: Vec<u8>,
}

impl MlsProposalMessagePayload {
    fn new(source_name: Agent, mls_msg: Vec<u8>) -> Self {
        MlsProposalMessagePayload {
            source_name,
            mls_msg,
        }
    }
}

#[derive(Debug)]
struct Endpoint<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// endpoint name
    name: Agent,

    /// channel name
    channel_name: AgentType,

    /// Optional channel id, when the channel is a single endpoint
    /// (e.g. a pipe)
    channel_id: Option<u64>,

    /// id of the current session
    session_id: Id,

    /// Session Type associated to this endpoint
    session_type: ProtoSessionType,

    /// connection id to the next hop SLIM
    conn: Option<u64>,

    /// true is the endpoint is already subscribed to the channel
    subscribed: bool,

    /// number or maximum retries before give up with a control message
    max_retries: u32,

    /// interval between retries
    retries_interval: Duration,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

impl<T> Endpoint<T>
where
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        tx: T,
    ) -> Self {
        Endpoint {
            name,
            channel_name,
            channel_id,
            session_id,
            session_type,
            conn: None,
            subscribed: false,
            max_retries,
            retries_interval,
            tx,
        }
    }

    fn create_channel_message(
        &self,
        destination: &AgentType,
        destination_id: Option<u64>,
        broadcast: bool,
        request_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Vec<u8>,
    ) -> Message {
        let flags = if broadcast {
            Some(SlimHeaderFlags::new(10, None, None, None, None))
        } else {
            None
        };

        let slim_header = Some(SlimHeader::new(
            &self.name,
            destination,
            destination_id,
            flags,
        ));

        let session_header = Some(SessionHeader::new(
            self.session_type.into(),
            request_type.into(),
            self.session_id,
            message_id,
        ));

        Message::new_publish_with_headers(slim_header, session_header, "", payload)
    }

    async fn join(&mut self) -> Result<(), SessionError> {
        // subscribe only once to the channel
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        // subscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let sub = Message::new_subscribe(&self.name, &self.channel_name, self.channel_id, header);

        self.send(sub).await?;

        // set route for the channel
        self.set_route(&self.channel_name, self.channel_id).await
    }

    async fn set_route(
        &self,
        route_name: &AgentType,
        route_id: Option<u64>,
    ) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_subscribe(
            &self.name,
            route_name,
            route_id,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn delete_route(
        &self,
        route_name: &AgentType,
        route_id: Option<u64>,
    ) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_unsubscribe(
            &self.name,
            route_name,
            route_id,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn leave(&self) -> Result<(), SessionError> {
        // unsubscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let unsub =
            Message::new_unsubscribe(&self.name, &self.channel_name, self.channel_id, header);

        self.send(unsub).await?;

        // remove route for the channel
        self.delete_route(&self.channel_name, None).await
    }

    async fn send(&self, msg: Message) -> Result<(), SessionError> {
        self.tx.send_to_slim(Ok(msg)).await
    }
}

pub fn handle_channel_discovery_message(
    message: &Message,
    source: &Agent,
    session_id: Id,
    session_type: ProtoSessionType,
) -> Message {
    let destination = message.get_source();
    let msg_id = message.get_id();

    let slim_header = Some(SlimHeader::new(
        source,
        destination.agent_type(),
        destination.agent_id_option(),
        Some(SlimHeaderFlags::default().with_forward_to(message.get_incoming_conn())),
    ));

    let session_header = Some(SessionHeader::new(
        session_type.into(),
        ProtoSessionMessageType::ChannelDiscoveryReply.into(),
        session_id,
        msg_id,
    ));

    debug!("Received discovery request, reply to the msg source");

    Message::new_publish_with_headers(slim_header, session_header, "", vec![])
}

#[derive(Debug)]
pub struct ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Agent>,

    /// timer used for retransmission of mls proposal messages
    timer: Option<crate::timer::Timer>,

    /// endpoint
    endpoint: Endpoint<T>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,
}

impl<P, V, T> ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let endpoint = Endpoint::new(
            name,
            channel_name,
            channel_id,
            session_id,
            session_type,
            max_retries,
            retries_interval,
            tx,
        );
        ChannelParticipant {
            moderator_name: None,
            timer: None,
            endpoint,
            mls_state: mls,
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the payload
        let names = msg
            .get_payload()
            .map_or_else(
                || {
                    error!("missing payload in a Join Channel request, ignore the message");
                    Err(SessionError::Processing(
                        "missing payload in a Join Channel request".to_string(),
                    ))
                },
                |content| -> Result<(JoinMessagePayload, usize), SessionError> {
                    bincode::decode_from_slice(&content.blob, bincode::config::standard())
                        .map_err(|e| SessionError::JoinChannelPayload(e.to_string()))
                },
            )?
            .0;

        // set local state according to the info in the message
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.endpoint.session_id = msg.get_session_header().get_session_id();
        self.endpoint.channel_name = names.channel_name.clone();

        // set route in order to be able to send packets to the moderator
        self.endpoint
            .set_route(
                names.moderator_name.agent_type(),
                names.moderator_name.agent_id_option(),
            )
            .await?;

        // If names.moderator_name and names.channel_name are the same, skip the join
        self.endpoint.subscribed = names
            .channel_id
            .is_some_and(|id| names.moderator_name == Agent::new(names.channel_name, id));

        // set the moderator name after the set route
        self.moderator_name = Some(names.moderator_name);

        // send reply to the moderator
        let src = msg.get_source();
        let payload: Vec<u8> = if msg.contains_metadata(METADATA_MLS_ENABLED) {
            // if mls we need to provide the key package
            self.mls_state
                .as_mut()
                .ok_or(SessionError::NoMls)?
                .generate_key_package()?
        } else {
            // without MLS we can set the state for the channel
            // otherwise the endpoint needs to receive a
            // welcome message first
            self.endpoint.join().await?;
            vec![]
        };

        // reply to the request
        let reply = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelJoinReply,
            msg.get_id(),
            payload,
        );

        self.endpoint.send(reply).await
    }

    async fn on_mls_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        self.mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_welcome_message(&msg)?;

        debug!("Welcome message correctly processed, MLS state initialized");

        // set route for the channel name
        self.endpoint.join().await?;

        // send an ack back to the moderator
        let src = msg.get_source();
        let ack = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelMlsAck,
            msg.get_id(),
            vec![],
        );

        self.endpoint.send(ack).await
    }

    async fn on_mls_control_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_source = msg.get_source();
        let msg_id = msg.get_id();

        // process the control message
        self.mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_control_message(msg, &self.endpoint.name)?;

        debug!("Control message correctly processed, MLS state updated");

        // send an ack back to the moderator
        let ack = self.endpoint.create_channel_message(
            msg_source.agent_type(),
            msg_source.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelMlsAck,
            msg_id,
            vec![],
        );

        self.endpoint.send(ack).await
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // leave the channel
        self.endpoint.leave().await?;

        // reply to the request
        let src = msg.get_source();
        let reply = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelLeaveReply,
            msg.get_id(),
            vec![],
        );

        self.endpoint.send(reply).await?;

        match &self.moderator_name {
            Some(m) => {
                self.endpoint
                    .delete_route(m.agent_type(), m.agent_id_option())
                    .await?
            }
            None => {
                error!("moderator name is not set, cannot remove the route");
            }
        };

        Ok(())
    }

    async fn on_mls_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        // this is the ack for the proposal message (the only MLS ack that can
        // be received by a participant). Stop the timer and wait for the commit
        let msg_id = msg.get_id();

        match self.timer {
            Some(ref mut t) => {
                if t.get_id() != msg_id {
                    debug!("received unexpected ack, drop it");
                    return Err(SessionError::TimerNotFound("wrong timer id".to_string()));
                }
                // stop the timer
                t.stop();
            }
            None => {
                debug!("received unexpected ack, drop it");
                return Err(SessionError::TimerNotFound("timer not set".to_string()));
            }
        }

        info!("participant remove the timer for MLS proposal");
        // reset the timer and return
        self.timer = None;
        Ok(())
    }
}

impl<P, V, T> MlsEndpoint for ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        if self.mls_state.is_none() || self.moderator_name.is_none() {
            return Err(SessionError::NoMls);
        }

        if self.timer.is_some() {
            // there is already another key change pending so drop this one
            return Err(SessionError::KeyRotationPending);
        }

        debug!("Update mls keys");
        let mls = self.mls_state.as_mut().unwrap();
        let proposal_msg;
        {
            let mut lock = mls.mls.lock();
            proposal_msg = lock
                .create_rotation_proposal()
                .map_err(|e| SessionError::NewProposalMessage(e.to_string()))?;
        }
        let dest = self.moderator_name.as_ref().unwrap();

        let content = MlsProposalMessagePayload::new(self.endpoint.name.clone(), proposal_msg);
        let payload: Vec<u8> =
            bincode::encode_to_vec(&content, bincode::config::standard()).unwrap();
        // get msg id
        let proposal_id = rand::random::<u32>();
        let proposal = self.endpoint.create_channel_message(
            dest.agent_type(),
            dest.agent_id_option(),
            true,
            ProtoSessionMessageType::ChannelMlsProposal,
            proposal_id,
            payload,
        );

        debug!("Send MLS Proposal Message to the moderator (participant key update)");
        self.endpoint.send(proposal.clone()).await?;

        // create a timer for the proposal
        let observer = Arc::new(RequestTimerObserver {
            message: proposal,
            tx: self.endpoint.tx.clone(),
        });

        let timer = crate::timer::Timer::new(
            proposal_id,
            crate::timer::TimerType::Constant,
            self.endpoint.retries_interval,
            None,
            Some(self.endpoint.max_retries),
        );

        timer.start(observer);

        self.timer = Some(timer);
        Ok(())
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.mls_state
            .as_ref()
            .ok_or(SessionError::NoMls)?
            .is_mls_up()
    }
}

impl<P, V, T> OnMessageReceived for ChannelParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().session_message_type();
        match msg_type {
            ProtoSessionMessageType::ChannelDiscoveryRequest => {
                error!(
                    "Received discovery request message, this should not happen. drop the message"
                );

                Err(SessionError::Processing(
                    "Received discovery request message, this should not happen".to_string(),
                ))
            }
            ProtoSessionMessageType::ChannelJoinRequest => {
                debug!("Received join request message");
                self.on_join_request(msg).await
            }
            ProtoSessionMessageType::ChannelMlsWelcome => {
                debug!("Received mls welcome message");
                self.on_mls_welcome(msg).await
            }
            ProtoSessionMessageType::ChannelMlsCommit => {
                debug!("Received mls commit message");
                self.on_mls_control_message(msg).await
            }
            ProtoSessionMessageType::ChannelMlsProposal => {
                debug!("Received mls proposal message");
                self.on_mls_control_message(msg).await
            }
            ProtoSessionMessageType::ChannelLeaveRequest => {
                debug!("Received leave request message");
                self.on_leave_request(msg).await
            }
            ProtoSessionMessageType::ChannelMlsAck => {
                debug!("Received mls ack message");
                self.on_mls_ack(msg).await
            }
            _ => {
                debug!("Received message of type {:?}, drop it", msg_type);

                Err(SessionError::Processing(format!(
                    "Received message of type {:?}, drop it",
                    msg_type
                )))
            }
        }
    }
}

#[derive(Debug)]
/// structure to store timers for pending requests
struct ChannelTimer {
    /// the timer itself
    timer: crate::timer::Timer,

    /// number of expected acks before stop the timer
    /// this is used for broadcast messages
    expected_acks: u32,

    /// message to process once the timer is deleted
    /// because all the acks are received and so the
    /// request succeeded (e.g. used for leave request msg
    /// or to send the commit after proposal broadcast)
    to_process: Option<Message>,
}

#[derive(Debug)]
pub struct ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    endpoint: Endpoint<T>,

    /// list of pending requests and related timers
    pending_requests: HashMap<u32, ChannelTimer>,

    /// channel name as payload to add to the invite messages
    invite_payload: Vec<u8>,

    /// mls state
    mls_state: Option<MlsModeratorState<P, V>>,
}

#[allow(clippy::too_many_arguments)]
impl<P, V, T> ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: Agent,
        channel_name: AgentType,
        channel_id: Option<u64>,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
    ) -> Self {
        let p = JoinMessagePayload::new(channel_name.clone(), channel_id, name.clone());
        let invite_payload: Vec<u8> = bincode::encode_to_vec(p, bincode::config::standard())
            .expect("unable to parse channel name as payload");

        let mls_state = mls.map(MlsModeratorState::new);

        let endpoint = Endpoint::new(
            name,
            channel_name,
            channel_id,
            session_id,
            session_type,
            max_retries,
            retries_interval,
            tx,
        );
        ChannelModerator {
            endpoint,
            pending_requests: HashMap::new(),
            invite_payload,
            mls_state,
        }
    }

    pub async fn join(&mut self) -> Result<(), SessionError> {
        if !self.endpoint.subscribed {
            // join the channel
            self.endpoint.join().await?;

            // create mls group if needed
            if let Some(mls) = self.mls_state.as_mut() {
                mls.init_moderator()?;
            }
        }

        Ok(())
    }

    async fn forward(&mut self, msg: Message) -> Result<(), SessionError> {
        // forward message received from the app and set a timer
        let msg_id = msg.get_id();
        self.endpoint.send(msg.clone()).await?;
        // create a timer for this request
        self.create_timer(msg_id, 1, msg, None);

        Ok(())
    }

    fn create_timer(
        &mut self,
        key: u32,
        pending_messages: u32,
        msg: Message,
        to_process: Option<Message>,
    ) {
        let observer = Arc::new(RequestTimerObserver {
            message: msg,
            tx: self.endpoint.tx.clone(),
        });

        let timer = crate::timer::Timer::new(
            key,
            crate::timer::TimerType::Constant,
            self.endpoint.retries_interval,
            None,
            Some(self.endpoint.max_retries),
        );
        timer.start(observer);

        let t = ChannelTimer {
            timer,
            expected_acks: pending_messages,
            to_process,
        };

        self.pending_requests.insert(key, t);
    }

    async fn delete_timer(&mut self, key: u32) -> Result<bool, SessionError> {
        let to_process;
        match self.pending_requests.get_mut(&key) {
            Some(timer) => {
                if timer.expected_acks > 0 {
                    timer.expected_acks -= 1;
                }
                if timer.expected_acks == 0 {
                    timer.timer.stop();
                    to_process = timer.to_process.clone();
                    self.pending_requests.remove(&key);
                } else {
                    return Ok(false);
                }
            }
            None => {
                return Err(SessionError::TimerNotFound(key.to_string()));
            }
        }

        debug!("got all the acks, remove timer");

        if to_process.is_some() {
            match to_process
                .as_ref()
                .unwrap()
                .get_session_header()
                .session_message_type()
            {
                ProtoSessionMessageType::ChannelLeaveRequest => {
                    debug!("forward channel leave request after timer cancellation");
                    self.forward(to_process.unwrap()).await?;
                }
                ProtoSessionMessageType::ChannelMlsProposal => {
                    debug!("create commit message for mls proposal after timer cancellation");
                    // check the payload of the proposal message
                    let content = &to_process
                        .as_ref()
                        .unwrap()
                        .get_payload()
                        .map_or_else(
                            || {
                                error!("missing payload in a Mls Proposal, ignore the message");
                                Err(SessionError::Processing(
                                    "missing payload in a Mls Proposal".to_string(),
                                ))
                            },
                            |content| -> Result<(MlsProposalMessagePayload, usize), SessionError> {
                                bincode::decode_from_slice(
                                    &content.blob,
                                    bincode::config::standard(),
                                )
                                .map_err(|e| SessionError::ParseProposalMessage(e.to_string()))
                            },
                        )?
                        .0;

                    let commit_payload = if content.source_name == self.endpoint.name {
                        // this proposal was originated by the moderator itself
                        // apply it and send the commit
                        self.mls_state
                            .as_mut()
                            .unwrap()
                            .process_local_pending_proposal()?
                    } else {
                        // the proposal comes from a participant
                        // process the content and send the commit
                        self.mls_state
                            .as_mut()
                            .unwrap()
                            .process_proposal_message(&content.mls_msg)?
                    };

                    // broadcast the commit
                    let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

                    let commit = self.endpoint.create_channel_message(
                        &self.endpoint.channel_name,
                        self.endpoint.channel_id,
                        true,
                        ProtoSessionMessageType::ChannelMlsCommit,
                        commit_id,
                        commit_payload,
                    );

                    // send commit message if needed
                    let len = self.mls_state.as_ref().unwrap().participants.len();

                    debug!("Send MLS Commit Message to the channel (commit for proposal)");
                    self.endpoint.send(commit.clone()).await?;
                    self.create_timer(commit_id, len.try_into().unwrap(), commit, None);
                }
                _ => { /*nothing to do at the moment*/ }
            }
        }

        Ok(true)
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the id of the message
        let recv_msg_id = msg.get_id();

        // If recv_msg_id is not in the pending requests, this will fail with an error
        self.delete_timer(recv_msg_id).await?;

        // set the local state and join the channel
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.join().await?;

        // an endpoint replied to the discovery message
        // send a join message
        let src = msg.get_slim_header().get_source();
        let new_msg_id = rand::random::<u32>();

        // this message cannot be received but it is created here
        let mut join = self.endpoint.create_channel_message(
            src.agent_type(),
            src.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelJoinRequest,
            new_msg_id,
            self.invite_payload.clone(),
        );

        if self.mls_state.is_some() {
            join.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());
            debug!("Reply with the join request, MLS is enabled");
        } else {
            debug!("Reply with the join request, MLS is disabled");
        }

        // add a new timer for the join message
        self.create_timer(new_msg_id, 1, join.clone(), None);

        // send the message
        self.endpoint.send(join).await
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let src = msg.get_slim_header().get_source();
        let msg_id = msg.get_id();

        // cancel timer, there only one message pending here
        let ret = self.delete_timer(msg_id).await?;
        debug_assert!(ret, "timer for join reply should be removed");

        // send MLS messages if needed
        if self.mls_state.is_some() {
            let (commit_payload, welcome_payload) =
                self.mls_state.as_mut().unwrap().add_participant(&msg)?;

            // send the commit message to the channel
            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
            let welcome_id = rand::random::<u32>();

            let commit = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                None,
                true,
                ProtoSessionMessageType::ChannelMlsCommit,
                commit_id,
                commit_payload,
            );
            let mut welcome = self.endpoint.create_channel_message(
                src.agent_type(),
                src.agent_id_option(),
                false,
                ProtoSessionMessageType::ChannelMlsWelcome,
                welcome_id,
                welcome_payload,
            );
            welcome.insert_metadata(
                METADATA_MLS_INIT_COMMIT_ID.to_string(),
                commit_id.to_string(),
            );

            // send welcome message
            debug!("Send MLS Welcome Message to the new participant");
            self.endpoint.send(welcome.clone()).await?;
            self.create_timer(welcome_id, 1, welcome, None);

            // send commit message if needed
            let len = self.mls_state.as_ref().unwrap().participants.len();
            if len > 1 {
                debug!("Send MLS Commit Message to the channel (new group member)");
                self.endpoint.send(commit.clone()).await?;
                self.create_timer(commit_id, (len - 1).try_into().unwrap(), commit, None);
            }
        };

        Ok(())
    }

    async fn on_msl_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        let recv_msg_id = msg.get_id();
        let _ = self.delete_timer(recv_msg_id).await?;

        // notify mls state that an ack was received
        self.mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .on_mls_ack()
    }

    async fn on_mls_proposal(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the payload
        let source = msg.get_source();
        let msg_id = msg.get_id();
        let payload = &msg
            .get_payload()
            .ok_or(SessionError::CommitMessage(
                "missing payload in MLS proposal, cannot process it".to_string(),
            ))?
            .blob;

        // ack the MLS proposal
        debug!("received proposal from a participant, send ack");
        let ack = self.endpoint.create_channel_message(
            source.agent_type(),
            source.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelMlsAck,
            msg_id,
            vec![],
        );

        self.endpoint.send(ack).await?;

        // if the sender is the only participant in the group we can apply the proposal
        // locally and send a commit. otherwise the proposal must be known by all the
        // members of the group and so we have to broadcast the proposal first and send
        // the commit when all the acks are received
        let len = self.mls_state.as_ref().unwrap().participants.len();

        if len == 1 {
            // we have a single participant in the group. apply the proposal and send the commit
            let content: MlsProposalMessagePayload =
                bincode::decode_from_slice(payload, bincode::config::standard())
                    .map_err(|e| SessionError::ParseProposalMessage(e.to_string()))?
                    .0;

            debug!("process received proposal and send commit (single participant)");
            let commit_payload = self
                .mls_state
                .as_mut()
                .unwrap()
                .process_proposal_message(&content.mls_msg)?;

            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

            let commit = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                self.endpoint.channel_id,
                true,
                ProtoSessionMessageType::ChannelMlsCommit,
                commit_id,
                commit_payload,
            );

            debug!(
                "Send MLS Commit Message to the channel (commit received proposal - single participant)"
            );
            self.endpoint.send(commit.clone()).await?;
            self.create_timer(commit_id, len.try_into().unwrap(), commit, None);
        } else {
            // broadcast the proposal on the channel
            let broadcast_msg_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
            let broadcast_msg = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                self.endpoint.channel_id,
                true,
                ProtoSessionMessageType::ChannelMlsProposal,
                broadcast_msg_id,
                payload.to_vec(),
            );

            // send the proposal to all the participants and set the timers
            debug!("Send MLS Proposal Message to the channel (key rotation)");
            self.endpoint.send(broadcast_msg.clone()).await?;
            self.create_timer(
                broadcast_msg_id,
                len.try_into().unwrap(),
                broadcast_msg.clone(),
                Some(broadcast_msg),
            );
        }

        Ok(())
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // If MLS is on send the MLS commit and wait for all the
        // acks before send the leave request. If MLS is of forward
        // the message
        match self.mls_state.as_mut() {
            Some(state) => {
                let commit_payload = state.remove_participant(&msg)?;

                let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

                let commit = self.endpoint.create_channel_message(
                    &self.endpoint.channel_name,
                    self.endpoint.channel_id,
                    true,
                    ProtoSessionMessageType::ChannelMlsCommit,
                    commit_id,
                    commit_payload,
                );

                // send commit message if needed
                debug!("Send MLS Commit Message to the channel (remove group member)");
                self.endpoint.send(commit.clone()).await?;

                // wait for len + 1 acks because the participant list does not contains
                // the removed participant anymore
                let len = self.mls_state.as_ref().unwrap().participants.len() + 1;

                // the leave request will be forwarded after all acks are received
                self.create_timer(commit_id, (len).try_into().unwrap(), commit, Some(msg));

                Ok(())
            }
            None => {
                // just send the leave request
                self.forward(msg).await
            }
        }
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_id = msg.get_id();

        // cancel timer
        let ret = self.delete_timer(msg_id).await?;
        debug_assert!(ret, "timer for leave reply should be removed");
        Ok(())
    }
}

impl<P, V, T> MlsEndpoint for ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        if self.mls_state.is_none() {
            return Err(SessionError::NoMls);
        }

        let mls = &self.mls_state.as_mut().unwrap().common;
        let proposal_msg;
        {
            let mut lock = mls.mls.lock();
            proposal_msg = lock
                .create_rotation_proposal()
                .map_err(|e| SessionError::NewProposalMessage(e.to_string()))?;
        }

        let content = MlsProposalMessagePayload::new(self.endpoint.name.clone(), proposal_msg);
        let payload: Vec<u8> =
            bincode::encode_to_vec(&content, bincode::config::standard()).unwrap();
        let proposal_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
        let proposal = self.endpoint.create_channel_message(
            &self.endpoint.channel_name,
            self.endpoint.channel_id,
            true,
            ProtoSessionMessageType::ChannelMlsProposal,
            proposal_id,
            payload,
        );

        debug!("Send MLS Proposal Message to the channel (moderator key update)");
        let len = self.mls_state.as_ref().unwrap().participants.len();
        self.endpoint.send(proposal.clone()).await?;
        self.create_timer(
            proposal_id,
            len.try_into().unwrap(),
            proposal.clone(),
            Some(proposal),
        );

        Ok(())
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.mls_state
            .as_ref()
            .ok_or(SessionError::NoMls)?
            .is_mls_up()
    }
}

impl<P, V, T> OnMessageReceived for ChannelModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().session_message_type();
        match msg_type {
            ProtoSessionMessageType::ChannelDiscoveryRequest => {
                debug!("Invite new participant to the channel, send discovery message");
                // discovery message coming from the application
                self.forward(msg).await
            }
            ProtoSessionMessageType::ChannelDiscoveryReply => {
                debug!("Received discovery reply message");
                self.on_discovery_reply(msg).await
            }
            ProtoSessionMessageType::ChannelJoinReply => {
                debug!("Received join reply message");
                self.on_join_reply(msg).await
            }
            ProtoSessionMessageType::ChannelMlsAck => {
                debug!("Received mls ack message");
                self.on_msl_ack(msg).await
            }
            ProtoSessionMessageType::ChannelMlsProposal => {
                debug!("Received mls proposal message");
                self.on_mls_proposal(msg).await
            }
            ProtoSessionMessageType::ChannelLeaveRequest => {
                // leave message coming from the application
                debug!("Received leave request message");
                self.on_leave_request(msg).await
            }
            ProtoSessionMessageType::ChannelLeaveReply => {
                debug!("Received leave reply message");
                self.on_leave_reply(msg).await
            }
            _ => Err(SessionError::Processing(format!(
                "received unexpected packet type: {:?}",
                msg_type
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutils::MockTransmitter;

    use super::*;
    use slim_auth::simple::SimpleGroup;
    use tracing_test::traced_test;

    use slim_datapath::messages::AgentType;

    const SESSION_ID: u32 = 10;

    #[tokio::test]
    #[traced_test]
    async fn test_full_join_and_leave() {
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let (moderator_tx, mut moderator_rx) = tokio::sync::mpsc::channel(50);
        let (participant_tx, mut participant_rx) = tokio::sync::mpsc::channel(50);

        let moderator_tx = MockTransmitter {
            tx_app: tx_app.clone(),
            tx_slim: moderator_tx,
        };

        let participant_tx = MockTransmitter {
            tx_app: tx_app.clone(),
            tx_slim: participant_tx,
        };

        let moderator = Agent::from_strings("org", "default", "moderator", 12345);
        let participant = Agent::from_strings("org", "default", "participant", 5120);
        let channel_name = AgentType::from_strings("channel", "channel", "channel");
        let conn = 1;

        let moderator_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            moderator.clone(),
            SimpleGroup::new("moderator", "group"),
            SimpleGroup::new("moderator", "group"),
            std::path::PathBuf::from("/tmp/test_moderator_mls"),
        ))))
        .unwrap();

        let participant_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            participant.clone(),
            SimpleGroup::new("participant", "group"),
            SimpleGroup::new("participant", "group"),
            std::path::PathBuf::from("/tmp/test_participant_mls"),
        ))))
        .unwrap();

        let mut cm = ChannelModerator::new(
            moderator.clone(),
            channel_name.clone(),
            None,
            SESSION_ID,
            ProtoSessionType::SessionUnknown,
            3,
            Duration::from_millis(100),
            Some(moderator_mls),
            moderator_tx,
        );
        let mut cp = ChannelParticipant::new(
            participant.clone(),
            channel_name.clone(),
            None,
            SESSION_ID,
            ProtoSessionType::SessionUnknown,
            3,
            Duration::from_millis(100),
            Some(participant_mls),
            participant_tx,
        );

        // create a discovery request
        let flags = SlimHeaderFlags::default().with_incoming_conn(conn);

        let slim_header = Some(SlimHeader::new(
            &moderator,
            participant.agent_type(),
            None,
            Some(flags),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::SessionUnknown.into(),
            ProtoSessionMessageType::ChannelDiscoveryRequest.into(),
            SESSION_ID,
            rand::random::<u32>(),
        ));
        let payload: Vec<u8> =
            bincode::encode_to_vec(&moderator, bincode::config::standard()).unwrap();
        let request = Message::new_publish_with_headers(slim_header, session_header, "", payload);

        // receive the request at the session layer
        cm.on_message(request.clone()).await.unwrap();

        // the request is forwarded to slim
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(request, msg);

        // this message is handled by the session layer itself
        // so we can create a reply and send it back to the moderator
        let destination = msg.get_source();
        let msg_id = msg.get_id();
        let session_id = msg.get_session_header().get_session_id();

        let slim_header = Some(SlimHeader::new(
            &participant,
            destination.agent_type(),
            destination.agent_id_option(),
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::SessionUnknown.into(),
            ProtoSessionMessageType::ChannelDiscoveryReply.into(),
            session_id,
            msg_id,
        ));

        let mut msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        // message reception on moderator side
        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await.unwrap();

        // the first message is the subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a request to compare with the output of on_message
        let jp = JoinMessagePayload {
            channel_name: channel_name.clone(),
            channel_id: None,
            moderator_name: moderator.clone(),
        };

        let payload: Vec<u8> = bincode::encode_to_vec(&jp, bincode::config::standard()).unwrap();
        let mut request = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelJoinRequest,
            0,
            payload,
        );

        request.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());

        let mut msg = moderator_rx.recv().await.unwrap().unwrap();
        let msg_id = msg.get_id();
        request.set_message_id(msg_id);
        assert_eq!(msg, request);

        msg.set_incoming_conn(Some(conn));
        let msg_id = msg.get_id();
        cp.on_message(msg).await.unwrap();

        // the first message is the set route for moderator name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(
            &participant,
            moderator.agent_type(),
            moderator.agent_id_option(),
            header,
        );
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelJoinReply,
            msg_id,
            vec![],
        );
        let mut msg = participant_rx.recv().await.unwrap().unwrap();

        // the payload of the message contains the keypackage and it change all the times
        // so we can compare only the header
        assert_eq!(msg.get_slim_header(), reply.get_slim_header());
        assert_eq!(msg.get_session_header(), reply.get_session_header());

        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await.unwrap();

        // create a reply to compare with the output of on_message
        let mut reply = cm.endpoint.create_channel_message(
            participant.agent_type(),
            participant.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelMlsWelcome,
            0,
            vec![],
        );

        // this should be the MLS welcome message, we can comprare only
        // the headers like in the previous case
        let mut msg = moderator_rx.recv().await.unwrap().unwrap();
        reply.set_message_id(msg.get_id());
        assert_eq!(msg.get_slim_header(), reply.get_slim_header());
        assert_eq!(msg.get_session_header(), reply.get_session_header());

        // receive the message on the participant side
        msg.set_incoming_conn(Some(conn));
        let msg_id = msg.get_id();
        cp.on_message(msg).await.unwrap();

        // the first message generated is a subscription for the channel name
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&participant, &channel_name, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // the third is the ack
        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            moderator.agent_type(),
            moderator.agent_id_option(),
            false,
            ProtoSessionMessageType::ChannelMlsAck,
            msg_id,
            vec![],
        );

        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);
    }
}

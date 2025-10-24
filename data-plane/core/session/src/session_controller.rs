// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

// Third-party crates
use async_trait::async_trait;
use bincode::{Decode, Encode};
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, Content, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType, SessionHeader, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    Id, SessionError, Transmitter,
    common::SessionMessage,
    interceptor_mls::{METADATA_MLS_ENABLED, METADATA_MLS_INIT_COMMIT_ID},
    mls_state::{MlsEndpoint, MlsModeratorState, MlsProposalMessagePayload, MlsState},
    moderator_task::{
        AddParticipant, AddParticipantMls, ModeratorTask, RemoveParticipant, RemoveParticipantMls,
        TaskUpdate, UpdateParticipantMls,
    },
    traits::SessionComponentLifecycle,
};

const CHANNEL_CREATION: &str = "CHANNEL_CREATION";
const CHANNEL_SUBSCRIPTION: &str = "CHANNEL_SUBSCRIPTION";

struct RequestTimerObserver<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// message to send in case of timeout
    message: Message,

    /// transmitter to send messages to the local SLIM instance and to the application
    tx: T,
}

#[async_trait]
impl<T> crate::timer::TimerObserver for RequestTimerObserver<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
        debug!("Timeout number {} for request {}", timeouts, timer_id);

        if self
            .tx
            .send_to_slim(Ok(self.message.clone()))
            .await
            .is_err()
        {
            error!("Error sending invite message");
        }
    }

    async fn on_failure(&self, _timer_id: u32, _timeouts: u32) {
        error!(?self.message, "unable to send message, stop retrying");
        if self
            .tx
            .send_to_app(Err(SessionError::Processing(
                "timer failed on channel endpoint. Stop sending messages".to_string(),
            )))
            .await
            .is_err()
        {
            error!("Error notifying the application");
        }
    }

    async fn on_stop(&self, timer_id: u32) {
        trace!(%timer_id, "timer for rtx cancelled");
        // nothing to do
    }
}

trait OnMessageReceived {
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError>;
}

impl<P, V, T> MlsEndpoint for SessionController<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn is_mls_up(&self) -> Result<bool, SessionError> {
        match self {
            SessionController::SessionParticipant(cp) => cp.is_mls_up(),
            SessionController::SessionModerator(cm) => cm.is_mls_up(),
        }
    }

    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        match self {
            SessionController::SessionParticipant(cp) => cp.update_mls_keys().await,
            SessionController::SessionModerator(cm) => cm.update_mls_keys().await,
        }
    }
}

#[derive(Debug)]
pub(crate) enum SessionController<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    SessionParticipant(SessionParticipant<P, V, T>),
    SessionModerator(SessionModerator<P, V, T>),
}

impl<P, V, T> SessionController<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        match self {
            SessionController::SessionParticipant(cp) => cp.on_message(msg).await,
            SessionController::SessionModerator(cm) => cm.on_message(msg).await,
        }
    }

    pub fn close(&mut self) {
        match self {
            SessionController::SessionParticipant(cp) => cp.close(),
            SessionController::SessionModerator(cm) => cm.close(),
        }
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct JoinMessagePayload {
    channel_name: Name,
    moderator_name: Name,
}

#[derive(Debug)]
struct SessionControllerState<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// endpoint name
    name: Name,

    /// channel name
    channel_name: Name,

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

    /// immutable session-level metadata provided at session creation (used in join request)
    session_metadata: HashMap<String, String>,
}

impl<T> SessionControllerState<T>
where
    T: Transmitter + Send + Sync + Clone + 'static,
{
    const MAX_FANOUT: u32 = 256;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Name,
        channel_name: Name,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        tx: T,
        session_metadata: HashMap<String, String>,
    ) -> Self {
        SessionControllerState {
            name,
            channel_name,
            session_id,
            session_type,
            conn: None,
            subscribed: false,
            max_retries,
            retries_interval,
            tx,
            session_metadata,
        }
    }

    fn create_channel_message(
        &self,
        destination: &Name,
        broadcast: bool,
        request_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Option<Content>,
    ) -> Message {
        let flags = if broadcast {
            Some(SlimHeaderFlags::new(
                Self::MAX_FANOUT,
                None,
                None,
                None,
                None,
            ))
        } else {
            None
        };

        let slim_header = Some(SlimHeader::new(
            &self.name,
            destination,
            "", // put empty identity it will be updated by the identity interceptor
            flags,
        ));

        // no need to specify the source and the destination here. these messages
        // will never be seen by the application
        let session_header = Some(SessionHeader::new(
            self.session_type.into(),
            request_type.into(),
            self.session_id,
            message_id,
        ));

        Message::new_publish_with_headers(slim_header, session_header, payload)
    }

    // creation is set to true is this is the first join to the channel
    // done by the moderator node. False in all the other cases
    async fn join(&mut self, creation: bool) -> Result<(), SessionError> {
        // subscribe only once to the channel
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        // subscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let mut sub = Message::new_subscribe(&self.name, &self.channel_name, None, header);

        // add in the metadata to indication that the
        // subscription is associated to a channel
        sub.insert_metadata(CHANNEL_SUBSCRIPTION.to_string(), "true".to_string());
        if creation {
            sub.insert_metadata(CHANNEL_CREATION.to_string(), "true".to_string());
        }

        self.send(sub).await?;

        // set route for the channel
        self.set_route(&self.channel_name).await
    }

    async fn set_route(&self, route_name: &Name) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_subscribe(
            &self.name,
            route_name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn delete_route(&self, route_name: &Name) -> Result<(), SessionError> {
        // send a message with subscription from
        let msg = Message::new_unsubscribe(
            &self.name,
            route_name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send(msg).await
    }

    async fn leave(&self) -> Result<(), SessionError> {
        // unsubscribe for the channel
        let header = Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap()));
        let mut unsub = Message::new_unsubscribe(&self.name, &self.channel_name, None, header);

        // add in the metadata to indication that the
        // subscription is associated to a channel
        unsub.insert_metadata(CHANNEL_SUBSCRIPTION.to_string(), "true".to_string());

        self.send(unsub).await?;

        // remove route for the channel
        self.delete_route(&self.channel_name).await
    }

    async fn send(&self, msg: Message) -> Result<(), SessionError> {
        self.tx.send_to_slim(Ok(msg)).await
    }
}

pub fn handle_channel_discovery_message(
    message: &Message,
    app_name: &Name,
    session_id: Id,
    session_type: ProtoSessionType,
) -> Message {
    let destination = message.get_source();

    // the destination of the discovery message may be different from the name of
    // application itself. This can happen if the application subscribes to multiple
    // service names. So we can reply using as a source the destination name of
    // the discovery message but setting the application id

    let mut source = message.get_dst();
    source.set_id(app_name.id());
    let msg_id = message.get_id();

    let slim_header = Some(SlimHeader::new(
        &source,
        &destination,
        "", // the identity will be added by the identity interceptor
        Some(SlimHeaderFlags::default().with_forward_to(message.get_incoming_conn())),
    ));

    // no need to specify the source and the destination here. these messages
    // will never be seen by the application
    let session_header = Some(SessionHeader::new(
        session_type.into(),
        ProtoSessionMessageType::DiscoveryReply.into(),
        session_id,
        msg_id,
    ));

    debug!("Received discovery request, reply to the msg source");

    let payload = Some(CommandPayload::new_discovery_reply_payload().as_content());
    Message::new_publish_with_headers(slim_header, session_header, payload)
}

#[derive(Debug)]
pub struct SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// timer used for retransmission of mls proposal messages
    timer: Option<crate::timer::Timer>,

    /// endpoint
    endpoint: SessionControllerState<T>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,
}

impl<P, V, T> SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Name,
        channel_name: Name,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
        session_metadata: HashMap<String, String>,
    ) -> Self {
        let endpoint = SessionControllerState::new(
            name,
            channel_name,
            session_id,
            session_type,
            max_retries,
            retries_interval,
            tx,
            session_metadata,
        );

        SessionParticipant {
            moderator_name: None,
            timer: None,
            endpoint,
            mls_state: mls,
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the payload
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_join_request_payload();
        let source = msg.get_source();

        // set local state according to the info in the message
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.endpoint.session_id = msg.get_session_header().get_session_id();
        self.endpoint.channel_name = Name::from(payload.channel.as_ref().ok_or_else(|| {
            SessionError::Processing("missing source in proposal payload".to_string())
        })?);

        // set route in order to be able to send packets to the moderator
        self.endpoint.set_route(&source).await?;

        // If names.moderator_name and names.channel_name are the same, skip the join
        if self.endpoint.session_type == ProtoSessionType::PointToPoint {
            self.endpoint.subscribed = true;
        }

        // set the moderator name after the set route
        self.moderator_name = Some(source);

        // send reply to the moderator
        let src = msg.get_source();
        let payload = if msg.contains_metadata(METADATA_MLS_ENABLED) {
            // if mls we need to provide the key package
            let key = self
                .mls_state
                .as_mut()
                .ok_or(SessionError::NoMls)?
                .generate_key_package()?;
            Some(key)
        } else {
            // without MLS we can set the state for the channel
            // otherwise the endpoint needs to receive a
            // welcome message first
            self.endpoint.join(false).await?;
            None
        };

        let content = Some(CommandPayload::new_join_reply_payload(payload).as_content());

        // reply to the request
        let reply = self.endpoint.create_channel_message(
            &src,
            false,
            ProtoSessionMessageType::JoinReply,
            msg.get_id(),
            content,
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
        self.endpoint.join(false).await?;

        // send an ack back to the moderator
        let src = msg.get_source();
        let ack = self.endpoint.create_channel_message(
            &src,
            false,
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            Some(CommandPayload::new_group_ack_payload().as_content()),
        );

        self.endpoint.send(ack).await
    }

    async fn on_mls_control_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_source = msg.get_source();
        let msg_id = msg.get_id();

        // process the control message
        let ret = self
            .mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_control_message(msg, &self.endpoint.name)?;

        if !ret {
            // message already processed, drop it
            debug!("Message with id {} already processed, drop it", msg_id);
            return Ok(());
        }

        debug!("Control message correctly processed, MLS state updated");

        // send an ack back to the moderator
        let ack = self.endpoint.create_channel_message(
            &msg_source,
            false,
            ProtoSessionMessageType::GroupAck,
            msg_id,
            Some(CommandPayload::new_group_ack_payload().as_content()),
        );

        self.endpoint.send(ack).await
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // leave the channel
        self.endpoint.leave().await?;

        // reply to the request
        let src = msg.get_source();
        let reply = self.endpoint.create_channel_message(
            &src,
            false,
            ProtoSessionMessageType::LeaveReply,
            msg.get_id(),
            Some(CommandPayload::new_leave_reply_payload().as_content()),
        );

        self.endpoint.send(reply).await?;

        match &self.moderator_name {
            Some(m) => self.endpoint.delete_route(m).await?,
            None => {
                error!("moderator name is not set, cannot remove the route");
            }
        };

        if let Some(t) = &mut self.timer {
            t.stop();
        }

        Ok(())
    }

    async fn on_mls_ack_or_nack(&mut self, msg: Message) -> Result<(), SessionError> {
        // this is the ack for the proposal message (the only MLS ack that can
        // be received by a participant). Stop the timer and wait for the commit
        let msg_id = msg.get_id();

        match self.timer {
            Some(ref mut t) => {
                if t.get_id() != msg_id {
                    debug!("Received unexpected ack, drop it");
                    return Err(SessionError::TimerNotFound("wrong timer id".to_string()));
                }
                // stop the timer
                t.stop();
            }
            None => {
                debug!("Received unexpected ack, drop it");
                return Err(SessionError::TimerNotFound("timer not set".to_string()));
            }
        }

        debug!("Got a reply for MLS proposal form the moderator, remove the timer");
        // reset the timer
        self.timer = None;

        // check the payload of the msg. if is not empty the moderator
        // rejected the proposal so we need to send a new one.
        if msg.get_session_message_type() == ProtoSessionMessageType::GroupAck {
            // all good the moderator is processing the update
            debug!("Proposal message was accepted by the moderator");
        } else {
            debug!("Proposal message was rejected by the moderator, send it again");
            self.update_mls_keys().await?;
        }

        Ok(())
    }
}

impl<P, V, T> MlsEndpoint for SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
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

        let payload = Some(
            CommandPayload::new_group_proposal_payload(
                Some(self.endpoint.name.clone()),
                proposal_msg,
            )
            .as_content(),
        );

        // get msg id
        let proposal_id = rand::random::<u32>();
        let proposal = self.endpoint.create_channel_message(
            dest,
            true,
            ProtoSessionMessageType::GroupProposal,
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

impl<P, V, T> SessionComponentLifecycle for SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        debug!("closing channel for session {}", self.endpoint.session_id);
        if let Some(t) = &mut self.timer {
            t.stop();
        }
    }
}

impl<P, V, T> OnMessageReceived for SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().session_message_type();
        match msg_type {
            ProtoSessionMessageType::DiscoveryRequest => {
                error!(
                    "Received discovery request message, this should not happen. drop the message"
                );

                Err(SessionError::Processing(
                    "Received discovery request message, this should not happen".to_string(),
                ))
            }
            ProtoSessionMessageType::JoinRequest => {
                debug!("Received join request message");
                self.on_join_request(msg).await
            }
            ProtoSessionMessageType::GroupWelcome => {
                debug!("Received mls welcome message");
                self.on_mls_welcome(msg).await
            }
            ProtoSessionMessageType::GroupUpdate => {
                debug!("Received mls commit message");
                self.on_mls_control_message(msg).await
            }
            ProtoSessionMessageType::GroupProposal => {
                debug!("Received mls proposal message");
                self.on_mls_control_message(msg).await
            }
            ProtoSessionMessageType::LeaveRequest => {
                debug!("Received leave request message");
                self.on_leave_request(msg).await
            }
            ProtoSessionMessageType::GroupAck => {
                debug!("Received mls ack message");
                self.on_mls_ack_or_nack(msg).await
            }
            ProtoSessionMessageType::GroupNack => {
                debug!("Received mls mack message");
                self.on_mls_ack_or_nack(msg).await
            }
            _ => {
                error!("Received message of type {:?}, drop it", msg_type);

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
pub struct SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    endpoint: SessionControllerState<T>,

    /// list of pending task to execute
    tasks_todo: VecDeque<Message>,

    /// the current task executed by the moderator
    /// if it is None the moderator can accept a new task
    current_task: Option<ModeratorTask>,

    /// list of pending requests and related timers
    pending_requests: HashMap<u32, ChannelTimer>,

    /// mls state
    mls_state: Option<MlsModeratorState<P, V>>,

    /// map of the participant in the channel
    /// map from name to u64. The name is the
    /// generic name provided by the app/controller on
    /// invite/remove participant. The val contains the
    /// id of the actual participant found after the
    /// discovery
    group_list: HashMap<Name, u64>,

    /// channel to send delete message to the session layer
    tx_session: Option<tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>>,

    /// set to true on delete_all
    closing: bool,
}

#[allow(clippy::too_many_arguments)]
impl<P, V, T> SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub fn new(
        name: Name,
        channel_name: Name,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        mls: Option<MlsState<P, V>>,
        tx: T,
        tx_session: Option<tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>>,
        session_metadata: HashMap<String, String>,
    ) -> Self {
        let mls_state = mls.map(MlsModeratorState::new);

        let endpoint = SessionControllerState::new(
            name,
            channel_name,
            session_id,
            session_type,
            max_retries,
            retries_interval,
            tx,
            session_metadata,
        );

        SessionModerator {
            endpoint,
            tasks_todo: vec![].into(),
            current_task: None,
            pending_requests: HashMap::new(),
            mls_state,
            group_list: HashMap::new(),
            tx_session,
            closing: false,
        }
    }

    pub async fn join(&mut self) -> Result<(), SessionError> {
        if !self.endpoint.subscribed {
            // join the channel
            self.endpoint.join(true).await?;

            // create mls group if needed
            if let Some(mls) = self.mls_state.as_mut() {
                mls.init_moderator()?;
            }
        }

        Ok(())
    }

    async fn parse_discovery_request(&mut self, mut msg: Message) -> Result<Message, SessionError> {
        // check if there is a destination name in the payload. If yes recreate the message
        // with the right destination and send it out
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_discovery_request_payload();
        match payload.destination {
            Some(dst_name) => {
                // set the connection id if not done yet
                self.endpoint.conn = Some(msg.get_incoming_conn());

                // set the route to forward the messages correctly
                let dst = Name::from(&dst_name);
                self.endpoint.set_route(&dst).await?;

                // create a new empty payload and change the message destination
                let p = CommandPayload::new_discovery_request_payload(None).as_content();
                msg.get_slim_header_mut().set_source(&self.endpoint.name);
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(p);
                Ok(msg)
            }
            None => {
                // simply forward the message
                Ok(msg)
            }
        }
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

        debug!("Got all the acks, remove timer");

        if let Some(msg) = to_process {
            match msg.get_session_header().session_message_type() {
                ProtoSessionMessageType::LeaveRequest => {
                    debug!("Forward channel leave request after timer cancellation");
                    let msg_id = msg.get_id();
                    self.forward(msg).await?;

                    // advance current task state and start leave phase
                    self.current_task.as_mut().unwrap().leave_start(msg_id)?;
                }
                ProtoSessionMessageType::GroupProposal => {
                    debug!("Create commit message for mls proposal after timer cancellation");
                    // check the payload of the proposal message
                    let payload = msg
                        .get_payload()
                        .unwrap()
                        .as_command_payload()
                        .as_group_proposal_payload();

                    let original_source = Name::from(payload.source.as_ref().ok_or_else(|| {
                        SessionError::Processing("missing source in proposal payload".to_string())
                    })?);

                    let commit = if original_source == self.endpoint.name {
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
                            .process_proposal_message(&payload.mls_proposal)?
                    };

                    // broadcast the commit
                    let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

                    let mut participants_vec = vec![];
                    for (n, id) in &self.group_list {
                        let name = n.clone().with_id(*id);
                        participants_vec.push(name);
                    }

                    let payload = Some(
                        CommandPayload::new_group_update_payload(participants_vec, Some(commit))
                            .as_content(),
                    );

                    let commit = self.endpoint.create_channel_message(
                        &self.endpoint.channel_name,
                        true,
                        ProtoSessionMessageType::GroupUpdate,
                        commit_id,
                        payload,
                    );

                    // send commit message if needed
                    let len = self.mls_state.as_ref().unwrap().participants.len();

                    debug!("Send MLS Commit Message to the channel (commit for proposal)");
                    self.endpoint.send(commit.clone()).await?;
                    self.create_timer(commit_id, len.try_into().unwrap(), commit, None);

                    // advance current task state and start commit phase
                    self.current_task
                        .as_mut()
                        .unwrap()
                        .commit_start(commit_id)?;
                }
                _ => { /*nothing to do at the moment*/ }
            }
        }

        debug!(%key, "Timer cancelled, all messages acked");
        Ok(true)
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // get the id of the message
        let recv_msg_id = msg.get_id();

        // If recv_msg_id is not in the pending requests, this will fail with an error
        self.delete_timer(recv_msg_id).await?;

        // evolve the current task state
        // the discovery phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_complete(recv_msg_id)?;

        // set the local state and join the channel
        self.endpoint.conn = Some(msg.get_incoming_conn());
        self.join().await?;

        // an endpoint replied to the discovery message
        // send a join message
        let src = msg.get_slim_header().get_source();
        let new_msg_id = rand::random::<u32>();

        let payload = Some(
            CommandPayload::new_join_request_payload(
                true,
                true,
                self.mls_state.is_some(),
                Some(self.endpoint.max_retries),
                Some(self.endpoint.retries_interval),
                Some(self.endpoint.channel_name.clone()),
            )
            .as_content(),
        );

        // this message cannot be received but it is created here
        let mut join = self.endpoint.create_channel_message(
            &src,
            false,
            ProtoSessionMessageType::JoinRequest,
            new_msg_id,
            payload,
        );

        if self.mls_state.is_some() {
            join.insert_metadata(METADATA_MLS_ENABLED.to_string(), "true".to_owned());
            debug!("Reply with the join request, MLS is enabled");
        } else {
            debug!("Reply with the join request, MLS is disabled");
        }

        // add immutable session metadata (do not override existing keys)
        if !self.endpoint.session_metadata.is_empty() {
            for (k, v) in self.endpoint.session_metadata.iter() {
                if !join.contains_metadata(k) {
                    join.insert_metadata(k.clone(), v.clone());
                }
            }
        }

        // add a new timer for the join message
        self.create_timer(new_msg_id, 1, join.clone(), None);

        // evolve the current task state
        // start the join phase
        self.current_task.as_mut().unwrap().join_start(new_msg_id)?;

        // send the message
        self.endpoint.send(join).await
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let src = msg.get_slim_header().get_source();
        let msg_id = msg.get_id();

        // cancel timer, there only one message pending here
        let ret = self.delete_timer(msg_id).await?;
        debug_assert!(ret, "timer for join reply should be removed");

        // evolve the current task state
        // the join phase is completed
        self.current_task.as_mut().unwrap().join_complete(msg_id)?;

        // at this point the participant is part of the group so we can add it to
        // the list, if msl is on the interaction will continue and the participant
        // will be added to the MLS group as well later on
        let mut new_participant_name = src.clone();
        let new_participant_id = new_participant_name.id();
        new_participant_name.reset_id();
        self.group_list
            .insert(new_participant_name, new_participant_id);

        // send MLS messages if needed
        if self.mls_state.is_some() {
            let (commit_payload, welcome_payload) =
                self.mls_state.as_mut().unwrap().add_participant(&msg)?;

            // send the commit message to the channel
            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
            let welcome_id = rand::random::<u32>();

            // Create participants list for commit message
            let mut participants_vec = vec![];
            for (n, id) in &self.group_list {
                let name = n.clone().with_id(*id);
                participants_vec.push(name);
            }

            let commit = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                true,
                ProtoSessionMessageType::GroupUpdate,
                commit_id,
                Some(
                    CommandPayload::new_group_update_payload(
                        participants_vec.clone(),
                        Some(commit_payload),
                    )
                    .as_content(),
                ),
            );
            let mut welcome = self.endpoint.create_channel_message(
                &src,
                false,
                ProtoSessionMessageType::GroupWelcome,
                welcome_id,
                Some(
                    CommandPayload::new_group_welcome_payload(
                        participants_vec,
                        Some(commit_id),
                        Some(welcome_payload),
                    )
                    .as_content(),
                ),
            );
            welcome.insert_metadata(
                METADATA_MLS_INIT_COMMIT_ID.to_string(),
                commit_id.to_string(),
            );

            // send welcome message
            debug!("Send MLS Welcome Message to the new participant");
            self.endpoint.send(welcome.clone()).await?;
            self.create_timer(welcome_id, 1, welcome, None);

            // evolve the current task state
            // welcome start
            self.current_task
                .as_mut()
                .unwrap()
                .welcome_start(welcome_id)?;

            // send commit message if needed
            let len = self.mls_state.as_ref().unwrap().participants.len();
            if len > 1 {
                debug!("Send MLS Commit Message to the channel (new group member)");
                self.endpoint.send(commit.clone()).await?;
                self.create_timer(commit_id, (len - 1).try_into().unwrap(), commit, None);

                // evolve the current task state
                // commit start
                self.current_task
                    .as_mut()
                    .unwrap()
                    .commit_start(commit_id)?;
            } else {
                // no commit message will be sent so update the task state to consider the commit as received
                // the timer id is not important here, it just need to be consistent
                self.current_task.as_mut().unwrap().commit_start(0)?;
                self.current_task.as_mut().unwrap().mls_phase_completed(0)?;
            }
        } else {
            // MLS is disable so the current task should be completed
            self.task_done().await?;
        }

        Ok(())
    }

    async fn on_msl_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        let recv_msg_id = msg.get_id();
        if self.delete_timer(recv_msg_id).await? {
            // one mls phase was completed so update the current task state
            self.current_task
                .as_mut()
                .unwrap()
                .mls_phase_completed(recv_msg_id)?;

            // check if the task is done. if yes we can set mls_up to
            // true because at least one MLS task was done
            if self.current_task.as_mut().unwrap().task_complete() {
                self.mls_state
                    .as_mut()
                    .ok_or(SessionError::NoMls)?
                    .common
                    .mls_up = true;
            }

            // check if the current task is completed
            self.task_done().await?;
        }

        Ok(())
    }

    async fn ack_msl_proposal(&mut self, msg: &Message) -> Result<(), SessionError> {
        // get the payload
        let source = msg.get_source();
        let msg_id = msg.get_id();

        let msg = if self.current_task.is_some() {
            // moderator is busy, send nack
            debug!("Received proposal from a participant, send nack");
            self.endpoint.create_channel_message(
                &source,
                false,
                ProtoSessionMessageType::GroupNack,
                msg_id,
                Some(CommandPayload::new_group_nack_payload().as_content()),
            )
        } else {
            // send an empty ack
            debug!("Received proposal from a participant, send ack");
            self.endpoint.create_channel_message(
                &source,
                false,
                ProtoSessionMessageType::GroupAck,
                msg_id,
                Some(CommandPayload::new_group_ack_payload().as_content()),
            )
        };

        // reply to the the MLS proposal
        self.endpoint.send(msg).await
    }

    async fn on_mls_proposal(&mut self, msg: Message) -> Result<(), SessionError> {
        // we need to send the ack back to the participant
        // if the moderator is no busy the message can be processed
        // immediately otherwise we need to ask to participant to send
        // a new proposal because the proposal as related to mls epochs
        // and a proposal from an old epoch cannot be processed.
        let payload = &msg
            .get_payload()
            .ok_or(SessionError::CommitMessage(
                "missing payload in MLS proposal, cannot process it".to_string(),
            ))?
            .as_command_payload()
            .as_group_proposal_payload();

        self.ack_msl_proposal(&msg).await?;

        // check if the moderator is busy or if we can process the packet
        if self.current_task.is_some() {
            debug!("Moderator is busy. drop the proposal");
            return Ok(());
        }

        // now the moderator is busy
        self.current_task = Some(ModeratorTask::UpdateParticipantMls(
            UpdateParticipantMls::default(),
        ));

        // if the sender is the only participant in the group we can apply the proposal
        // locally and send a commit. otherwise the proposal must be known by all the
        // members of the group and so we have to broadcast the proposal first and send
        // the commit when all the acks are received
        let len = self.mls_state.as_ref().unwrap().participants.len();

        if len == 1 {
            debug!("Only one participant in the group. send the commit");

            let commit_payload = self
                .mls_state
                .as_mut()
                .unwrap()
                .process_proposal_message(&payload.mls_proposal)?;

            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

            // Create participants list for commit message
            let mut participants_vec = vec![];
            for (n, id) in &self.group_list {
                let name = n.clone().with_id(*id);
                participants_vec.push(name);
            }

            let commit = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                true,
                ProtoSessionMessageType::GroupUpdate,
                commit_id,
                Some(
                    CommandPayload::new_group_update_payload(
                        participants_vec,
                        Some(commit_payload),
                    )
                    .as_content(),
                ),
            );

            debug!(
                "Send MLS Commit Message to the channel (commit received proposal - single participant)"
            );
            self.endpoint.send(commit.clone()).await?;
            self.create_timer(commit_id, len.try_into().unwrap(), commit, None);

            // in the current task mark the proposal phase as done because it will not be executed
            // and start the commit phase waiting for the ack
            self.current_task.as_mut().unwrap().proposal_start(0)?;
            self.current_task.as_mut().unwrap().mls_phase_completed(0)?;

            self.current_task
                .as_mut()
                .unwrap()
                .commit_start(commit_id)?;
        } else {
            // broadcast the proposal on the channel
            let broadcast_msg_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
            let broadcast_msg = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                true,
                ProtoSessionMessageType::GroupProposal,
                broadcast_msg_id,
                msg.get_payload().cloned(),
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

            // advance the current task with the proposal start
            self.current_task
                .as_mut()
                .unwrap()
                .proposal_start(broadcast_msg_id)?;
        }

        Ok(())
    }

    async fn delete_all(&mut self, _msg: Message) -> Result<(), SessionError> {
        debug!("receive a close channel message, send signals to all participants");
        // create tasks to remove each participant from the group
        // even if mls is enable we just send the leave message
        // in any case the group will be deleted so there is no need to
        // update the mls state, this will speed up the process
        self.closing = true;
        // remove mls state
        self.mls_state = None;
        // clear all pending tasks
        self.tasks_todo.clear();

        for (p, _id) in self.group_list.iter() {
            let leave = self.endpoint.create_channel_message(
                p,
                false,
                ProtoSessionMessageType::LeaveRequest,
                rand::random::<u32>(),
                Some(CommandPayload::new_leave_request_payload(Some(p.clone())).as_content()),
            );
            // append the task to the list
            self.tasks_todo.push_back(leave);
        }

        // try to pickup a task
        match self.tasks_todo.pop_front() {
            Some(m) => {
                self.current_task = Some(ModeratorTask::RemoveParticipant(
                    RemoveParticipant::default(),
                ));
                return self.on_leave_request(m).await;
            }
            None => {
                // we can notify the session layer and close the channel
                if let Some(tx_session) = &self.tx_session {
                    debug!("Signal session layer to close the session, all tasks are done");
                    tx_session
                        .send(Ok(SessionMessage::DeleteSession {
                            session_id: self.endpoint.session_id,
                        }))
                        .await
                        .map_err(|e| {
                            SessionError::Processing(format!("failed to send delete session: {e}"))
                        })?;
                }
                Ok(())
            }
        }
    }

    async fn on_leave_request(&mut self, mut msg: Message) -> Result<(), SessionError> {
        // we need to adjust the message
        // if coming from the controller we need to modify source and destination
        // if coming from the app we need to add the participant id to the destination
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_leave_request_payload();
        let leave_message = match payload.destination {
            Some(dst_name) => {
                // Handle case where destination is provided
                let dst = Name::from(&dst_name);
                let id = *self
                    .group_list
                    .get(&dst)
                    .ok_or(SessionError::RemoveParticipant(
                        "participant not found".to_string(),
                    ))?;

                let dst = dst.with_id(id);

                let new_payload = CommandPayload::new_leave_request_payload(None).as_content();
                msg.get_slim_header_mut().set_source(&self.endpoint.name);
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(new_payload);

                msg
            }
            None => {
                // Handle case where no destination is provided, use message destination
                let dst = msg.get_dst();
                let id = *self
                    .group_list
                    .get(&dst)
                    .ok_or(SessionError::RemoveParticipant(
                        "participant not found".to_string(),
                    ))?;

                msg.get_slim_header_mut().set_destination(&dst.with_id(id));
                msg
            }
        };

        // If MLS is on, send the MLS commit and wait for all the
        // acks before send the leave request. If MLS is off forward
        // the message
        match self.mls_state.as_mut() {
            Some(state) => {
                let commit_payload = state.remove_participant(&leave_message)?;

                let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

                // Create participants list for commit message
                let mut participants_vec = vec![];
                for (n, id) in &self.group_list {
                    let name = n.clone().with_id(*id);
                    participants_vec.push(name);
                }

                let commit = self.endpoint.create_channel_message(
                    &self.endpoint.channel_name,
                    true,
                    ProtoSessionMessageType::GroupUpdate,
                    commit_id,
                    Some(
                        CommandPayload::new_group_update_payload(
                            participants_vec,
                            Some(commit_payload),
                        )
                        .as_content(),
                    ),
                );

                // send commit message if needed
                debug!("Send MLS Commit Message to the channel (remove group member)");
                self.endpoint.send(commit.clone()).await?;

                // wait for len + 1 acks because the participant list does not contains
                // the removed participant anymore
                let len = self.mls_state.as_ref().unwrap().participants.len() + 1;

                // the leave request will be forwarded after all acks are received
                self.create_timer(
                    commit_id,
                    (len).try_into().unwrap(),
                    commit,
                    Some(leave_message),
                );
                self.current_task
                    .as_mut()
                    .unwrap()
                    .commit_start(commit_id)?;

                Ok(())
            }
            None => {
                // just send the leave request
                let msg_id = leave_message.get_id();
                self.forward(leave_message).await?;
                self.current_task.as_mut().unwrap().leave_start(msg_id)
            }
        }
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_id = msg.get_id();

        // remove the participant from the group list
        let mut src = msg.get_source();
        src.reset_id();
        self.group_list.remove(&src);

        // cancel timer
        if self.delete_timer(msg_id).await? {
            // with the leave reply reception we conclude a participant remove
            // update the task and try to pickup a new task
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
            self.task_done().await
        } else {
            debug!("Timer for leave reply {:?} was not removed", msg_id);
            Ok(())
        }
    }

    async fn task_done(&mut self) -> Result<(), SessionError> {
        if !self.current_task.as_ref().unwrap().task_complete() {
            // the task is not completed so just return
            // and continue with the process
            debug!("Current task is NOT completed");
            return Ok(());
        }

        // here the moderator is not busy anymore
        self.current_task = None;

        self.pop_task().await
    }

    async fn pop_task(&mut self) -> Result<(), SessionError> {
        if self.current_task.is_some() {
            // moderator is busy, nothing to do
            return Ok(());
        }

        // check if there is a pending task to process
        let msg = match self.tasks_todo.pop_front() {
            Some(m) => m,
            None => {
                // nothing else to do
                debug!("No tasks left to perform");

                // check if we need to close the session
                if self.closing
                    && let Some(tx_session) = &self.tx_session
                {
                    debug!("Signal session layer to close the session, all tasks are done");
                    tx_session
                        .send(Ok(SessionMessage::DeleteSession {
                            session_id: self.endpoint.session_id,
                        }))
                        .await
                        .map_err(|e| {
                            SessionError::Processing(format!("failed to send delete session: {e}"))
                        })?;
                }
                return Ok(());
            }
        };

        debug!("Process a new task from the todo list");
        let msg_type = msg.get_session_header().session_message_type();
        match msg_type {
            ProtoSessionMessageType::DiscoveryRequest => {
                // now the moderator is busy
                self.current_task = if self.mls_state.is_some() {
                    Some(ModeratorTask::AddParticipantMls(
                        AddParticipantMls::default(),
                    ))
                } else {
                    Some(ModeratorTask::AddParticipant(AddParticipant::default()))
                };

                debug!("Start a new inivte task, send discovery message");
                let msg_id = msg.get_id();
                // discovery message coming from the application
                let to_forward = self.parse_discovery_request(msg).await?;
                self.forward(to_forward).await?;

                // register the discovery start in the current task
                self.current_task.as_mut().unwrap().discovery_start(msg_id)
            }
            ProtoSessionMessageType::GroupProposal => {
                // only the moderator itself can schedule a proposal task
                debug!("Start a new local key update task");
                self.update_mls_keys().await
            }
            ProtoSessionMessageType::LeaveRequest => {
                // if the metadata contains the key "DELETE_GROUP" remove all the participants
                // and close the session when all task are completed
                if msg.contains_metadata("DELETE_GROUP") {
                    return self.delete_all(msg).await;
                }

                debug!("Start a new channel leave task");
                // now the moderator is busy
                self.current_task = if self.mls_state.is_some() {
                    Some(ModeratorTask::RemoveParticipantMls(
                        RemoveParticipantMls::default(),
                    ))
                } else {
                    Some(ModeratorTask::RemoveParticipant(
                        RemoveParticipant::default(),
                    ))
                };
                self.on_leave_request(msg).await
            }
            _ => {
                error!("unexpected message in the list of tasks to do, drop it");
                Err(SessionError::ModeratorTask(
                    "unexpected new task".to_string(),
                ))
            }
        }
    }
}

impl<P, V, T> MlsEndpoint for SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        debug!("Update local mls keys");

        if self.current_task.is_some() {
            debug!("Another task is running, schedule update for later");
            // if busy postpone the task and add it to the todo list
            // at this point we cannot create a real proposal so create
            // a fake one with empty payload and push it to the todo list
            let payload =
                Some(CommandPayload::new_group_proposal_payload(None, vec![]).as_content());
            let empty_msg = self.endpoint.create_channel_message(
                &self.endpoint.channel_name,
                true,
                ProtoSessionMessageType::GroupProposal,
                rand::random::<u32>(),
                payload,
            );

            self.tasks_todo.push_back(empty_msg);
            return Ok(());
        }

        // now the moderator is busy
        self.current_task = Some(ModeratorTask::UpdateParticipantMls(
            UpdateParticipantMls::default(),
        ));

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
            true,
            ProtoSessionMessageType::GroupProposal,
            proposal_id,
            Some(
                CommandPayload::new_group_proposal_payload(
                    Some(self.endpoint.name.clone()),
                    payload,
                )
                .as_content(),
            ),
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

        // advance current task with proposal start
        self.current_task
            .as_mut()
            .unwrap()
            .proposal_start(proposal_id)
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.mls_state
            .as_ref()
            .ok_or(SessionError::NoMls)?
            .is_mls_up()
    }
}

impl<P, V, T> SessionComponentLifecycle for SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        self.tasks_todo.clear();
        self.current_task = None;

        for (_, mut t) in self.pending_requests.drain() {
            t.timer.stop()
        }
    }
}

impl<P, V, T> OnMessageReceived for SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_type = msg.get_session_header().session_message_type();
        match msg_type {
            ProtoSessionMessageType::DiscoveryRequest => {
                // the channel discovery starts a new participant invite.
                // process the request only if not busy
                if self.current_task.is_some() {
                    debug!(
                        "Moderator is busy. Add invite participant task to the list and process it later"
                    );
                    // if busy postpone the task and add it to the todo list
                    self.tasks_todo.push_back(msg);
                    return Ok(());
                }
                // now the moderator is busy
                self.current_task = if self.mls_state.is_some() {
                    debug!("Create AddParticipantMls task");
                    Some(ModeratorTask::AddParticipantMls(
                        AddParticipantMls::default(),
                    ))
                } else {
                    debug!("Create AddParticipant task");
                    Some(ModeratorTask::AddParticipant(AddParticipant::default()))
                };

                let msg_id = msg.get_id();
                // discovery message coming from the application
                let to_forward = self.parse_discovery_request(msg).await?;
                self.forward(to_forward).await?;

                // register the discovery start in the current task
                self.current_task.as_mut().unwrap().discovery_start(msg_id)
            }
            ProtoSessionMessageType::DiscoveryReply => {
                // this is part of an invite, process the packet
                debug!("Received discovery reply message");
                self.on_discovery_reply(msg).await
            }
            ProtoSessionMessageType::JoinReply => {
                // this is part of an invite, process the packet
                debug!("Received join reply message");
                self.on_join_reply(msg).await
            }
            ProtoSessionMessageType::GroupAck => {
                // this is part of an mls exchange, process the packet
                debug!("Received mls ack message");
                self.on_msl_ack(msg).await
            }
            ProtoSessionMessageType::GroupProposal => {
                debug!("Received mls proposal message");
                self.on_mls_proposal(msg).await
            }
            ProtoSessionMessageType::LeaveRequest => {
                debug!("received leave request message");
                // leave message coming from the app or the controller
                // this message starts a new participant removal.
                // process the request only if not busy
                if self.current_task.is_some() {
                    // if busy postpone the task and add it to the todo list
                    debug!(
                        "Moderator is busy. Add  leave request task to the list and process it later"
                    );
                    self.tasks_todo.push_back(msg);
                    return Ok(());
                }

                // if the metadata contains the key "DELETE_GROUP" remove all the participants
                // and close the session when all task are completed
                if msg.contains_metadata("DELETE_GROUP") {
                    return self.delete_all(msg).await;
                }

                // now the moderator is busy
                self.current_task = if self.mls_state.is_some() {
                    Some(ModeratorTask::RemoveParticipantMls(
                        RemoveParticipantMls::default(),
                    ))
                } else {
                    Some(ModeratorTask::RemoveParticipant(
                        RemoveParticipant::default(),
                    ))
                };

                debug!("Received leave request message on moderator");
                self.on_leave_request(msg).await
            }
            ProtoSessionMessageType::LeaveReply => {
                // this is part of a remove, process the packet
                debug!("Received leave reply message on moderator");
                self.on_leave_reply(msg).await
            }
            ProtoSessionMessageType::JoinRequest => {
                // packet coming from the controller
                // this message created a new multicast session on the local app
                // setting the application as moderator
                // all the necessary is already set we can simply drop the packet
                debug!("Received channel join request from the controller.");
                Ok(())
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
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use parking_lot::Mutex;
    use slim_auth::shared_secret::SharedSecret;
    use slim_auth::testutils::TEST_VALID_SECRET;
    use slim_mls::mls::Mls;
    use tracing_test::traced_test;

    use slim_datapath::messages::Name;

    const SESSION_ID: u32 = 10;

    #[tokio::test]
    #[traced_test]
    async fn test_full_join_and_leave() {
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let (moderator_tx, mut moderator_rx) = tokio::sync::mpsc::channel(50);
        let (participant_tx, mut participant_rx) = tokio::sync::mpsc::channel(50);

        let moderator_tx = SessionTransmitter::new(moderator_tx, tx_app.clone());
        let participant_tx = SessionTransmitter::new(participant_tx, tx_app.clone());

        let moderator = Name::from_strings(["org", "default", "moderator"]).with_id(12345);
        let participant = Name::from_strings(["org", "default", "participant"]).with_id(5120);
        let moderator_id = &moderator.to_string();
        let channel_name = Name::from_strings(["channel", "channel", "channel"]);
        let conn = 1;

        let moderator_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            moderator.clone(),
            SharedSecret::new("moderator", TEST_VALID_SECRET),
            SharedSecret::new("moderator", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_moderator_mls"),
        ))))
        .unwrap();

        let participant_mls = MlsState::new(Arc::new(Mutex::new(Mls::new(
            participant.clone(),
            SharedSecret::new("participant", TEST_VALID_SECRET),
            SharedSecret::new("participant", TEST_VALID_SECRET),
            std::path::PathBuf::from("/tmp/test_participant_mls"),
        ))))
        .unwrap();

        let mut cm = SessionModerator::new(
            moderator.clone(),
            channel_name.clone(),
            SESSION_ID,
            ProtoSessionType::Unspecified,
            3,
            Duration::from_millis(100),
            Some(moderator_mls),
            moderator_tx,
            None,
            HashMap::new(),
        );
        let mut cp = SessionParticipant::new(
            participant.clone(),
            channel_name.clone(),
            SESSION_ID,
            ProtoSessionType::Unspecified,
            3,
            Duration::from_millis(100),
            Some(participant_mls),
            participant_tx,
            HashMap::new(),
        );

        // create a discovery request
        let flags = SlimHeaderFlags::default().with_incoming_conn(conn);

        let slim_header = Some(SlimHeader::new(
            &moderator,
            &participant,
            moderator_id,
            Some(flags),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::Unspecified.into(),
            ProtoSessionMessageType::DiscoveryRequest.into(),
            SESSION_ID,
            rand::random::<u32>(),
        ));

        let payload = Some(CommandPayload::new_discovery_request_payload(None).as_content());
        let request = Message::new_publish_with_headers(slim_header, session_header, payload);

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
            &destination,
            moderator_id,
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::Unspecified.into(),
            ProtoSessionMessageType::DiscoveryReply.into(),
            session_id,
            msg_id,
        ));

        let payload = Some(CommandPayload::new_discovery_reply_payload().as_content());
        let mut msg = Message::new_publish_with_headers(slim_header, session_header, payload);

        // message reception on moderator side
        msg.set_incoming_conn(Some(conn));
        cm.on_message(msg).await.unwrap();

        // the first message is the subscription for the channel name
        // this is also the channel creation
        let header = Some(SlimHeaderFlags::default().with_forward_to(conn));
        let mut sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        sub.insert_metadata(CHANNEL_SUBSCRIPTION.to_string(), "true".to_string());
        sub.insert_metadata(CHANNEL_CREATION.to_string(), "true".to_string());
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // then we have the set route for the channel name
        let header = Some(SlimHeaderFlags::default().with_recv_from(conn));
        let sub = Message::new_subscribe(&moderator, &channel_name, None, header);
        let msg = moderator_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        // create a request to compare with the output of on_message
        let payload = Some(
            CommandPayload::new_join_request_payload(
                true,
                true,
                true,
                Some(3),
                Some(Duration::from_millis(100)),
                Some(channel_name.clone()),
            )
            .as_content(),
        );

        let mut request = cm.endpoint.create_channel_message(
            &participant,
            false,
            ProtoSessionMessageType::JoinRequest,
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
        let sub = Message::new_subscribe(&participant, &moderator, None, header);
        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, sub);

        let payload = Some(CommandPayload::new_join_reply_payload(None).as_content());
        // create a reply to compare with the output of on_message
        let reply = cp.endpoint.create_channel_message(
            &moderator,
            false,
            ProtoSessionMessageType::JoinReply,
            msg_id,
            payload,
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
            &participant,
            false,
            ProtoSessionMessageType::GroupWelcome,
            0,
            Some(
                CommandPayload::new_group_welcome_payload(
                    vec![moderator.clone()],
                    Some(1),
                    Some(vec![]),
                )
                .as_content(),
            ),
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
        let mut sub = Message::new_subscribe(&participant, &channel_name, None, header);
        sub.insert_metadata(CHANNEL_SUBSCRIPTION.to_string(), "true".to_string());
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
            &moderator,
            false,
            ProtoSessionMessageType::GroupAck,
            msg_id,
            Some(CommandPayload::new_group_ack_payload().as_content()),
        );

        let msg = participant_rx.recv().await.unwrap().unwrap();
        assert_eq!(msg, reply);
    }
}

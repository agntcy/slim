// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{
    collections::{HashMap, HashSet, VecDeque},
    marker::PhantomData,
    sync::Arc,
    time::Duration,
};

// Third-party crates
use async_trait::async_trait;
use bincode::{Decode, Encode};
use tokio::{net::unix::pipe::Receiver, sync::mpsc};
use tokio_util::future::FutureExt;
use tracing::{debug, error, trace};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, Content,
        MessageType::{self, Subscribe},
        ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType, SessionHeader,
        SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    Id, MessageDirection, Session, SessionError, Transmitter,
    common::SessionMessage,
    controller_sender::ControllerSender,
    interceptor_mls::{METADATA_MLS_ENABLED, METADATA_MLS_INIT_COMMIT_ID},
    mls_state::{MlsEndpoint, MlsModeratorState, MlsProposalMessagePayload, MlsState},
    moderator_task::{
        AddParticipant, AddParticipantMls, ModeratorTask, RemoveParticipant, RemoveParticipantMls,
        TaskUpdate, UpdateParticipantMls,
    },
    timer,
    timer_factory::TimerSettings,
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

impl<P, V> MlsEndpoint for SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
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

pub(crate) enum SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    SessionParticipant(SessionParticipant<P, V>),
    SessionModerator(SessionModerator<P, V>),
}

impl<P, V> SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
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

/*struct SessionControllerState
{
    /// endpoint name
    name: Name, // session.source()

    /// channel name
    channel_name: Name, // session.dst() ??

    /// id of the current session
    session_id: Id, // session.id()

    /// Session Type associated to this endpoint
    session_type: ProtoSessionType, //session.kind()

    /// connection id to the next hop SLIM
    conn: Option<u64>, // missing!

    /// true is the endpoint is already subscribed to the channel
    subscribed: bool,

    /// number or maximum retries before give up with a control message
    max_retries: u32, // session.config

    /// interval between retries
    retries_interval: Duration,

    /// transmitter to send messages to the local SLIM instance and to the application
    //tx: T, // session.common.tx
    // self.session.unwrap().tx_ref().app_tx.send(value)

    /// immutable session-level metadata provided at session creation (used in join request)
    session_metadata: HashMap<String, String>, // session.config.metadata()
}

impl SessionControllerState {
    const MAX_FANOUT: u32 = 256;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Name,
        channel_name: Name,
        session_id: Id,
        session_type: ProtoSessionType,
        max_retries: u32,
        retries_interval: Duration,
        //tx: T,
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
            //tx,
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
        todo!()
        //self.tx.send_to_slim(Ok(msg)).await
    }
}*/

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

pub struct SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// timer used for retransmission of mls proposal messages
    timer: Option<crate::timer::Timer>,

    /// endpoint
    //endpoint: SessionControllerState,

    /// list of participants
    group_list: HashSet<Name>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// the session itself
    session: Arc<Session<P, V>>,
}

impl<P, V> SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        //name: Name,
        //channel_name: Name,
        //session_id: Id,
        //session_type: ProtoSessionType,
        //max_retries: u32,
        //retries_interval: Duration,
        //mls: Option<MlsState<P, V>>,
        //: Option<MlsState<P, V>>,
        session: Arc<Session<P, V>>,
    ) -> Self {
        /*let endpoint = SessionControllerState::new(
            name,
            channel_name,
            session_id,
            session_type,
            max_retries,
            retries_interval,
            //tx
            session_metadata,
        );*/

        let mls_state = session
            .mls()
            .map(|mls| MlsState::new(mls).expect("failed to create MLS state"));

        SessionParticipant {
            moderator_name: None,
            timer: None,
            group_list: HashSet::new(),
            mls_state,
            session,
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*// get the payload
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

        self.endpoint.send(reply).await*/
    }

    async fn on_mls_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*self.mls_state
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

        self.endpoint.send(ack).await*/
    }

    async fn on_mls_control_message(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*let msg_source = msg.get_source();
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

        self.endpoint.send(ack).await*/
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*// leave the channel
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

        Ok(())*/
    }

    async fn on_mls_ack_or_nack(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*// this is the ack for the proposal message (the only MLS ack that can
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

        Ok(())*/
    }
}

impl<P, V> MlsEndpoint for SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        todo!()
        /*if self.mls_state.is_none() || self.moderator_name.is_none() {
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
        // TODO, fix this!
        /*let observer = Arc::new(RequestTimerObserver {
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

        self.timer = Some(timer);*/
        Ok(())*/
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.mls_state
            .as_ref()
            .ok_or(SessionError::NoMls)?
            .is_mls_up()
    }
}

impl<P, V> SessionComponentLifecycle for SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        todo!()
        /*debug!("closing channel for session {}", self.endpoint.session_id);
        if let Some(t) = &mut self.timer {
            t.stop();
        }*/
    }
}

impl<P, V> OnMessageReceived for SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
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

pub struct SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    tx: tokio::sync::mpsc::Sender<SessionMessage>,
    _phantom: PhantomData<(P, V, T)>,
}

impl<P, V, T> SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(
        session: Arc<Session<P, V>>,
        transmitter: T,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let timer_settings = TimerSettings {
            duration: Duration::from_secs(1),
            max_duration: None,
            max_retries: Some(10),
            timer_type: timer::TimerType::Constant,
        };

        let (tx, rx) = mpsc::channel(128);

        let sender = ControllerSender::new(timer_settings, transmitter.clone(), tx.clone());

        // Create the processor
        let processor = SessionModeratorProcessor::new(session, sender, rx, tx_to_session_layer);

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        SessionModerator {
            tx,
            _phantom: PhantomData,
        }
    }

    pub async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        let msg = SessionMessage::OnMessage { message, direction };

        self.tx
            .send(msg)
            .await
            .map_err(|e| SessionError::SlimTransmission(e.to_string()))
    }
}

pub struct SessionModeratorProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// the session itself
    session: Arc<Session<P, V>>,

    /// connection from where the moderator
    /// is receiving messages
    conn: Option<u64>,

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

    /// sender for command messages
    sender: ControllerSender<T>,

    /// TODO: add arc a session sender and session receiver

    /// channel used to recevied messages from the session layer
    /// and timeouts from timers
    rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,

    /// channel used to communincate with the session layer.
    /// used to send closing signal at the moment
    tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// a postponed message is a leave message that we need to
    /// send after the receptions of all the acks for the group
    /// update message
    postponed_message: Option<Message>,

    /// subscribed is set to true if the moderator already subscribed
    /// for the channel
    subscribed: bool,

    /// set to true on delete_all
    closing: bool,
}

impl<P, V, T> SessionModeratorProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    const MAX_FANOUT: u32 = 256;

    /// session: pointer to the session
    /// sender: controller sender for timers
    /// rx_channel: messages coming from the session layer
    /// tx_channel: messages to the session layer,
    pub fn new(
        session: Arc<Session<P, V>>,
        sender: ControllerSender<T>,
        rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let mls_state = session.mls().map(|mls| {
            let mls_state = MlsState::new(mls).expect("failed to create MLS state");
            MlsModeratorState::new(mls_state)
        });

        SessionModeratorProcessor {
            session,
            conn: None,
            tasks_todo: vec![].into(),
            current_task: None,
            pending_requests: HashMap::new(),
            mls_state,
            group_list: HashMap::new(),
            sender,
            rx_from_session_layer,
            tx_to_session_layer,
            postponed_message: None,
            subscribed: false,
            closing: false,
        }
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                next = self.rx_from_session_layer.recv() => {
                    match next {
                        Some(message) => match message {
                            SessionMessage::OnMessage { message, direction: _ } => {
                                if self.is_command_message(message.get_session_message_type()) {
                                    self.process_control_message(message).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::SetPointToPointConfig { config } => todo!(),
                            SessionMessage::TimerTimeout { message_id, message_type, name: _, timeouts: _ } => {
                                if self.is_command_message(message_type) {
                                    self.sender.on_timer_timeout(message_id).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name: _, timeouts: _ } => {
                                if self.is_command_message(message_type) {
                                    self.sender.on_timer_failure(message_id).await;
                                    // the current task failed:
                                    // 1. create the right error message
                                    let error_message = match self.current_task.as_ref().unwrap() {
                                        ModeratorTask::AddParticipantMls(_) => {
                                            "failed to add a participant to the group"
                                        }
                                        ModeratorTask::RemoveParticipant(_) => {
                                            "failed to remove a participant from the group"
                                        }
                                        ModeratorTask::RemoveParticipantMls(_) => {
                                            "failed to remove a participant from the group"
                                        }
                                        ModeratorTask::UpdateParticipantMls(_) => {
                                            "failed to update state of the participant"
                                        }
                                    };

                                    // 2. notify the application
                                    self.session
                                        .tx_ref()
                                        .app_tx
                                        .send(Err(SessionError::ModeratorTask(error_message.to_string())))
                                        .await
                                        .map_err(|e| SessionError::Processing(format!("failed to notify application: {}", e)));

                                    // 3. delete current task and pick a new one
                                    self.current_task = None;
                                    if let Err(e) = self.pop_task().await {
                                        error!("Failed to pop next task: {:?}", e);
                                    }
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::DeleteSession { session_id } => todo!(),
                            SessionMessage::AddEndpoint { endpoint } => todo!(),
                            SessionMessage::RemoveEndpoint { endpoint } => todo!(),
                            SessionMessage::Drain { grace_period_ms } => todo!(),
                        }
                        None => {
                            debug!("session controller close channel {}", self.session.id());
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn process_control_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::DiscoveryRequest => self.on_discovery_request(message).await,
            ProtoSessionMessageType::DiscoveryReply => self.on_discovery_reply(message).await,
            ProtoSessionMessageType::JoinRequest => {
                // this message should arrive only from the control plane
                // the effect of it is to create the session itself with
                // the right settings. Here we can simply return
                Ok(())
            }
            ProtoSessionMessageType::JoinReply => self.on_join_reply(message).await,
            ProtoSessionMessageType::LeaveRequest => {
                // if the metadata contains the key "DELETE_GROUP" remove all the participants
                // and close the session when all task are completed
                if message.contains_metadata("DELETE_GROUP") {
                    return self.delete_all(message).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::GroupProposal => todo!(),
            ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupNack => {
                debug!(
                    "Unexpected control message type {:?}",
                    message.get_session_message_type()
                );
                Ok(())
            }
            _ => {
                debug!(
                    "Unexpected message type {:?}",
                    message.get_session_message_type()
                );
                Ok(())
            }
        }
    }

    /// message processing functions
    async fn on_discovery_request(&mut self, mut msg: Message) -> Result<(), SessionError> {
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
        debug!("Create AddParticipantMls task");
        self.current_task = Some(ModeratorTask::AddParticipantMls(
            AddParticipantMls::default(),
        ));

        // check if there is a destination name in the payload. If yes recreate the message
        // with the right destination and send it out
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_discovery_request_payload();

        let mut discovery = match payload.destination {
            Some(dst_name) => {
                // set the connection id if not done yet
                self.conn = Some(msg.get_incoming_conn());

                // set the route to forward the messages correctly
                let dst = Name::from(&dst_name);
                self.set_route(&dst).await?;

                // create a new empty payload and change the message destination
                let p = CommandPayload::new_discovery_request_payload(None).as_content();
                msg.get_slim_header_mut().set_source(&self.session.source());
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(p);
                msg
            }
            None => {
                // simply forward the message
                msg
            }
        };

        // start the current task
        let id = rand::random::<u32>();
        discovery.get_session_header_mut().set_message_id(id);
        self.current_task.as_mut().unwrap().discovery_start(id);

        // send the message
        self.send_with_timer(discovery).await
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // update sender status to stop timers
        self.sender.on_message(&msg).await?;

        // evolve the current task state
        // the discovery phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_complete(msg.get_id())?;

        // join the channel if needed
        self.join(msg.get_incoming_conn()).await?;

        // an endpoint replied to the discovery message
        // send a join message
        let msg_id = rand::random::<u32>();

        let payload = CommandPayload::new_join_request_payload(
            true,
            true,
            self.mls_state.is_some(),
            self.session.session_config().max_retries(),
            self.session.session_config().timer_duration(),
            Some(self.session.dst().unwrap()),
        )
        .as_content();

        self.send_control_message(
            &msg.get_slim_header().get_source(),
            ProtoSessionMessageType::JoinRequest,
            msg_id,
            payload,
            false,
        )
        .await?;

        // evolve the current task state
        // start the join phase
        self.current_task.as_mut().unwrap().join_start(msg_id)
    }

    async fn on_join_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // stop the timer for the join request
        self.sender.on_message(&msg).await?;

        // evolve the current task state
        // the join phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .join_complete(msg.get_id())?;

        // at this point the participant is part of the group so we can add it to the list
        let mut new_participant_name = msg.get_source().clone();
        let new_participant_id = new_participant_name.id();
        new_participant_name.reset_id();
        self.group_list
            .insert(new_participant_name, new_participant_id);

        // get mls data if MLS is enabled
        let (commit, commit_id, welcome, welcome_id) = if self.mls_state.is_some() {
            let (commit_payload, welcome_payload) =
                self.mls_state.as_mut().unwrap().add_participant(&msg)?;

            // get the id of the committ, the welcome message has a random id
            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
            let welcome_id = rand::random::<u32>();
            (
                Some(commit_payload),
                commit_id,
                Some(welcome_payload),
                welcome_id,
            )
        } else {
            (None, rand::random::<u32>(), None, rand::random::<u32>())
        };

        // Create participants list for the messages to send
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        // send the group update
        if participants_vec.len() > 2 {
            let update_payload =
                CommandPayload::new_group_update_payload(participants_vec.clone(), commit)
                    .as_content();
            self.send_control_message(
                &self.session.dst().unwrap(),
                ProtoSessionMessageType::GroupUpdate,
                commit_id,
                update_payload,
                true,
            )
            .await;
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

        // send welcome message
        let mls_commit_id = if welcome.is_some() {
            Some(welcome_id)
        } else {
            None
        };
        let welcome_payload =
            CommandPayload::new_group_welcome_payload(participants_vec, mls_commit_id, welcome)
                .as_content();
        self.send_control_message(
            &msg.get_slim_header().get_source(),
            ProtoSessionMessageType::GroupWelcome,
            welcome_id,
            welcome_payload,
            false,
        )
        .await;

        // evolve the current task state
        // welcome start
        self.current_task
            .as_mut()
            .unwrap()
            .welcome_start(welcome_id)?;

        Ok(())
    }

    async fn on_leave_request(&mut self, mut msg: Message) -> Result<(), SessionError> {
        if self.current_task.is_some() {
            // if busy postpone the task and add it to the todo list
            debug!("Moderator is busy. Add  leave request task to the list and process it later");
            self.tasks_todo.push_back(msg);
            return Ok(());
        }

        // now the moderator is busy
        self.current_task = Some(ModeratorTask::RemoveParticipantMls(
            RemoveParticipantMls::default(),
        ));

        // adjust the message according to the sender:
        // - if coming from the controller (destination in the payload) we need to modify source and destination
        // - if coming from the app (empty payload) we need to add the participant id to the destination
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
                msg.get_slim_header_mut().set_source(&self.session.source());
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(new_payload);
                msg.set_message_id(rand::random::<u32>());
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
                msg.set_message_id(rand::random::<u32>());
                msg
            }
        };

        // Before send the leave request we may need to send the Group update
        // with the new participant list and the new mls paylaod if needed
        // Create participants list for commit message
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        if participants_vec.len() > 2 {
            // in this case we need to send first the group update and later the leave message
            let (mls_payload, msg_id) = match self.mls_state.as_mut() {
                Some(state) => {
                    let commit_payload = state.remove_participant(&leave_message)?;
                    let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
                    (Some(commit_payload), commit_id)
                }
                None => (None, rand::random::<u32>()),
            };

            let update_payload =
                CommandPayload::new_group_update_payload(participants_vec, mls_payload)
                    .as_content();
            self.send_control_message(
                &self.session.dst().unwrap(),
                ProtoSessionMessageType::GroupUpdate,
                msg_id,
                update_payload,
                true,
            )
            .await?;
            self.current_task.as_mut().unwrap().commit_start(msg_id)?;

            // We need to save the leave message and send it after
            // the reception of all the acks for the group update message
            // see on_group_ack for postponed_message handling
            self.postponed_message = Some(leave_message);
        } else {
            // no commit message will be sent so update the task state to consider the commit as received
            // the timer id is not important here, it just need to be consistent
            self.current_task.as_mut().unwrap().commit_start(0)?;
            self.current_task.as_mut().unwrap().mls_phase_completed(0)?;

            // just send the leave message in this case
            self.sender.on_message(&leave_message).await;

            self.current_task
                .as_mut()
                .unwrap()
                .leave_start(leave_message.get_id());
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

        // Collect the participants first to avoid borrowing conflicts
        let participants: Vec<Name> = self.group_list.keys().cloned().collect();

        for p in participants {
            // here we use only p as destination name, the id will be
            // added later in the on_leave_request message
            let leave = self.create_control_message(
                &p,
                ProtoSessionMessageType::LeaveRequest,
                rand::random::<u32>(),
                CommandPayload::new_leave_request_payload(None).as_content(),
                false,
            );
            // append the task to the list
            self.tasks_todo.push_back(leave);
        }

        // try to pickup the first task
        match self.tasks_todo.pop_front() {
            Some(m) => {
                self.current_task = Some(ModeratorTask::RemoveParticipant(
                    RemoveParticipant::default(),
                ));
                self.on_leave_request(m).await
            }
            None => {
                self.send_close_signal().await;
                Ok(())
            }
        }
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        let msg_id = msg.get_id();

        // remove the participant from the group list
        let mut src = msg.get_source();
        src.reset_id();
        self.group_list.remove(&src);

        // notify the sender and see if we can pick another task
        self.sender.on_message(&msg).await?;
        if !self.sender.is_still_pending(msg_id) {
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
        }

        self.task_done().await
    }

    async fn on_group_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        // notify the sender
        self.sender.on_message(&msg);

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.sender.is_still_pending(msg.get_id()) {
            // we received all the messages related to this timer
            // check if we are done and move on
            self.current_task
                .as_mut()
                .unwrap()
                .mls_phase_completed(msg_id)?;

            // check if the task is complited.
            if self.current_task.as_mut().unwrap().task_complete() {
                // task done. At this point if this was the first MLS
                // action MLS is setup so we can set mls_up to true
                self.mls_state
                    .as_mut()
                    .ok_or(SessionError::NoMls)?
                    .common
                    .mls_up = true;
            } else {
                // if the task is not finished yet we may need to send a leave
                // message that was postponed to send all group update first
                if self.postponed_message.is_some()
                    && matches!(
                        self.current_task,
                        Some(ModeratorTask::RemoveParticipantMls(_))
                    )
                {
                    // send the leave message an progress
                    let leave_message = self.postponed_message.as_ref().unwrap();
                    self.sender.on_message(leave_message).await;
                    self.current_task
                        .as_mut()
                        .unwrap()
                        .leave_start(leave_message.get_id());
                    // rest the postponed message
                    self.postponed_message = None;
                }
            }

            // check if we can progress with another task
            self.task_done().await?;
        }

        Ok(())
    }

    /// task handlig functions
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
                if self.closing {
                    self.send_close_signal().await;
                }

                // return
                return Ok(());
            }
        };

        debug!("Process a new task from the todo list");
        self.process_control_message(msg).await
    }

    /// helper functions
    fn is_command_message(&self, message_type: ProtoSessionMessageType) -> bool {
        match message_type {
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinRequest
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck
            | ProtoSessionMessageType::GroupNack => true,
            _ => false,
        }
    }

    async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
    }

    async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.session
            .tx_ref()
            .slim_tx
            .send(Ok(message))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to send route message: {}", e)))
    }

    async fn set_route(&self, name: &Name) -> Result<(), SessionError> {
        let route = Message::new_subscribe(
            &self.session.source(),
            &name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send_to_slim(route).await
    }

    fn create_control_message(
        &mut self,
        dst: &Name,
        message_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Content,
        broadcast: bool,
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
            &self.session.source(),
            dst,
            "", // put empty identity it will be updated by the identity interceptor
            flags,
        ));

        let session_type = match self.session.kind() {
            crate::SessionType::PointToPoint => ProtoSessionType::PointToPoint,
            crate::SessionType::Multicast => ProtoSessionType::Multicast,
        };

        let session_header = Some(SessionHeader::new(
            session_type.into(),
            message_type.into(),
            self.session.id(),
            message_id,
        ));

        Message::new_publish_with_headers(slim_header, session_header, Some(payload))
    }

    async fn send_control_message(
        &mut self,
        dst: &Name,
        message_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Content,
        broadcast: bool,
    ) -> Result<(), SessionError> {
        let msg = self.create_control_message(dst, message_type, message_id, payload, broadcast);
        self.send_with_timer(msg).await
    }

    async fn join(&mut self, in_conn: u64) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.conn = Some(in_conn);

        let sub = Message::new_subscribe(
            &self.session.source(),
            self.session.dst().as_ref().unwrap(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.conn.unwrap())),
        );

        self.send_to_slim(sub).await?;
        self.set_route(self.session.dst().as_ref().unwrap());

        // create mls group if needed
        if let Some(mls) = self.mls_state.as_mut() {
            mls.init_moderator()?;
        }

        // add ourself to the participants
        let local_name = self.session.source().clone();
        let id = local_name.id();
        self.group_list.insert(local_name, id);

        Ok(())
    }

    async fn send_close_signal(&mut self) {
        // TODO check if the senders/receivers are happy as well
        debug!("Signal session layer to close the session, all tasks are done");
        let res = self
            .tx_to_session_layer
            .send(SessionMessage::DeleteSession {
                session_id: self.session.id(),
            })
            .await;

        if res.is_err() {
            error!("an error occured while signaling session close");
        }
    }

    /*fn create_timer(
        &mut self,
        key: u32,
        pending_messages: u32,
        msg: Message,
        to_process: Option<Message>,
    ) {
        todo!()
        /*let observer = Arc::new(RequestTimerObserver {
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

        self.pending_requests.insert(key, t);*/
    }*/

    /*async fn delete_timer(&mut self, key: u32) -> Result<bool, SessionError> {
        todo!()
        /*let to_process;
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
        Ok(true)*/
    }*/

    async fn ack_msl_proposal(&mut self, msg: &Message) -> Result<(), SessionError> {
        todo!()
        /*// get the payload
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
        self.endpoint.send(msg).await*/
    }

    async fn on_mls_proposal(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
        /*// we need to send the ack back to the participant
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

        Ok(())*/
    }

    async fn delete_all(&mut self, _msg: Message) -> Result<(), SessionError> {
        todo!()
        /*debug!("receive a close channel message, send signals to all participants");
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
        }*/
    }
}

impl<P, V> MlsEndpoint for SessionModerator<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        todo!()
        /*debug!("Update local mls keys");

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
            .proposal_start(proposal_id)*/
    }

    fn is_mls_up(&self) -> Result<bool, SessionError> {
        self.mls_state
            .as_ref()
            .ok_or(SessionError::NoMls)?
            .is_mls_up()
    }
}

impl<P, V> SessionComponentLifecycle for SessionModerator<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        self.tasks_todo.clear();
        self.current_task = None;

        for (_, mut t) in self.pending_requests.drain() {
            t.timer.stop()
        }
    }
}

impl<P, V> OnMessageReceived for SessionModerator<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
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

/*#[cfg(test)]
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
            //moderator_tx,
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
            //participant_tx,
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
}*/

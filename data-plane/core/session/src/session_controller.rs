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
        AddParticipantMls, ModeratorTask, RemoveParticipant, RemoveParticipantMls, TaskUpdate,
        UpdateParticipantMls,
    },
    timer,
    timer_factory::TimerSettings,
    traits::SessionComponentLifecycle,
};

//trait OnMessageReceived {
//    async fn on_message(&mut self, msg: Message) -> Result<(), SessionError>;
//}

impl<P, V, T> MlsEndpoint for SessionController<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn is_mls_up(&self) -> Result<bool, SessionError> {
        todo!()
        // still needed?
        //match self {
        //    SessionController::SessionParticipant(cp) => cp.is_mls_up(),
        //    SessionController::SessionModerator(cm) => cm.is_mls_up(),
        //}
    }

    async fn update_mls_keys(&mut self) -> Result<(), SessionError> {
        todo!()
        // still needed?
        //match self {
        //    SessionController::SessionParticipant(cp) => cp.update_mls_keys().await,
        //    SessionController::SessionModerator(cm) => cm.update_mls_keys().await,
        //}
    }
}

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
    pub async fn on_message(
        &self,
        msg: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match self {
            SessionController::SessionParticipant(cp) => cp.on_message(msg, direction).await,
            SessionController::SessionModerator(cm) => cm.on_message(msg, direction).await,
        }
    }

    pub fn close(&mut self) {
        todo!()
        // still needed?
        //match self {
        //    SessionController::SessionParticipant(cp) => cp.close(),
        //    SessionController::SessionModerator(cm) => cm.close(),
        //}
    }

    pub fn is_initiator(&self) -> bool {
        match self {
            SessionController::SessionParticipant(_) => false,
            SessionController::SessionModerator(_) => true,
        }
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

pub struct SessionControllerCommon<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// connection used to send messages
    conn: Option<u64>,

    /// sender for command messages
    sender: ControllerSender<T>,

    /// channel used to recevied messages from the session layer
    /// and timeouts from timers
    rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,

    /// channel used to communincate with the session layer.
    tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// the session itself
    session: Arc<Session<P, V>>,
}

impl<P, V, T> SessionControllerCommon<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    const MAX_FANOUT: u32 = 256;

    fn new(
        sender: ControllerSender<T>,
        rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
        session: Arc<Session<P, V>>,
    ) -> Self {
        SessionControllerCommon {
            conn: None,
            sender,
            rx_from_session_layer,
            tx_to_session_layer,
            session,
        }
    }

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

    async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.session
            .tx_ref()
            .slim_tx
            .send(Ok(message))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to send route message: {}", e)))
    }

    async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
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

    async fn delete_route(&self, name: &Name) -> Result<(), SessionError> {
        let route = Message::new_unsubscribe(
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
}

pub struct SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    tx: tokio::sync::mpsc::Sender<SessionMessage>,
    _phantom: PhantomData<(P, V, T)>,
}

impl<P, V, T> SessionParticipant<P, V, T>
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
        let processor = SessionParticipantProcessor::new(session, sender, rx, tx_to_session_layer);

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        SessionParticipant {
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

impl<P, V, T> SessionComponentLifecycle for SessionParticipant<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        todo!()
        // probably we don't need this anymore
    }
}

pub struct SessionParticipantProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// list of participants
    group_list: HashSet<Name>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    common: SessionControllerCommon<P, V, T>,
}

impl<P, V, T> SessionParticipantProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
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
        let mls_state = session
            .mls()
            .map(|mls| MlsState::new(mls).expect("failed to create MLS state"));

        SessionParticipantProcessor {
            moderator_name: None,
            group_list: HashSet::new(),
            mls_state,
            common: SessionControllerCommon {
                conn: None,
                sender,
                rx_from_session_layer,
                tx_to_session_layer,
                session,
            },
        }
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                next = self.common.rx_from_session_layer.recv() => {
                    match next {
                        Some(message) => match message {
                            SessionMessage::OnMessage { message, direction: _ } => {
                                if self.common.is_command_message(message.get_session_message_type()) {
                                    self.process_control_message(message).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::SetPointToPointConfig { config } => todo!(),
                            SessionMessage::TimerTimeout { message_id, message_type, name: _, timeouts: _ } => {
                                if self.common.is_command_message(message_type) {
                                    // check it needed
                                    self.common.sender.on_timer_timeout(message_id).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name: _, timeouts: _ } => {
                                if self.common.is_command_message(message_type) {
                                    // check if needed
                                    self.common.sender.on_timer_failure(message_id).await;
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
                            debug!("session controller close channel {}", self.common.session.id());
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn process_control_message(&mut self, message: Message) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::JoinRequest => self.on_join_request(message).await,
            ProtoSessionMessageType::GroupWelcome => self.on_mls_welcome(message).await,
            ProtoSessionMessageType::GroupUpdate => self.on_group_update_message(message).await,
            ProtoSessionMessageType::LeaveRequest => self.on_leave_request(message).await,
            ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck
            | ProtoSessionMessageType::GroupNack => todo!(),
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveReply => {
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

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // set local state with the moderator params
        let source = msg.get_source();
        self.moderator_name = Some(source.clone());
        self.common.conn = Some(msg.get_incoming_conn());

        // set route in order to be able to send packets to the moderator
        self.common.set_route(&source).await?;

        // send reply to the moderator

        let payload = if self.mls_state.is_some() {
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
            self.join().await?;
            None
        };

        let content = CommandPayload::new_join_reply_payload(payload).as_content();

        // reply to the request
        self.common
            .send_control_message(
                &source,
                ProtoSessionMessageType::JoinReply,
                msg.get_id(),
                content,
                false,
            )
            .await
    }

    async fn on_mls_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        self.mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_welcome_message(&msg)?;

        debug!("Welcome message correctly processed, MLS state initialized");

        // set route for the channel name
        self.join().await?;

        // send an ack back to the moderator
        self.common
            .send_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::GroupAck,
                msg.get_id(),
                CommandPayload::new_group_ack_payload().as_content(),
                false,
            )
            .await
    }

    async fn on_group_update_message(&mut self, msg: Message) -> Result<(), SessionError> {
        // process the control message
        let ret = self
            .mls_state
            .as_mut()
            .ok_or(SessionError::NoMls)?
            .process_control_message(msg.clone(), &self.common.session.source())?;

        if !ret {
            // message already processed, drop it
            debug!(
                "Message with id {} already processed, drop it",
                msg.get_id()
            );
            return Ok(());
        }

        debug!("Control message correctly processed, MLS state updated");

        // update participant list
        let list = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_group_update_payload()
            .participant;
        for p in list {
            self.group_list.insert(Name::from(&p));
        }

        // send an ack back to the moderator
        self.common
            .send_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::GroupAck,
                msg.get_id(),
                CommandPayload::new_group_ack_payload().as_content(),
                false,
            )
            .await
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // reply to the sender
        self.common
            .send_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::LeaveReply,
                msg.get_id(),
                CommandPayload::new_leave_reply_payload().as_content(),
                false,
            )
            .await?;

        // leave the channel
        self.leave().await?;

        // notify the session layer that the session can be removed
        self.common
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.session.id(),
            }))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to notify session layer: {}", e)))
    }

    async fn on_mls_ack_or_nack(&mut self, msg: Message) -> Result<(), SessionError> {
        todo!()
    }

    /// helper functions
    async fn join(&self) -> Result<(), SessionError> {
        // we need to setup the network only in case of multicast session
        let session_type = match self.common.session.kind() {
            crate::SessionType::PointToPoint => ProtoSessionType::PointToPoint,
            crate::SessionType::Multicast => ProtoSessionType::Multicast,
        };

        if session_type == ProtoSessionType::PointToPoint {
            // simply return
            return Ok(());
        }

        // set route and subscription for the group name
        self.common
            .set_route(&self.common.session.dst().unwrap())
            .await;
        let sub = Message::new_subscribe(
            &self.common.session.source(),
            self.common.session.dst().as_ref().unwrap(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
        );

        self.common.send_to_slim(sub).await
    }

    async fn leave(&self) -> Result<(), SessionError> {
        // delete route to the remote endpoint
        self.common
            .delete_route(&self.common.session.dst().unwrap())
            .await;

        // we need to setup the network only in case of multicast session
        let session_type = match self.common.session.kind() {
            crate::SessionType::PointToPoint => ProtoSessionType::PointToPoint,
            crate::SessionType::Multicast => ProtoSessionType::Multicast,
        };

        if session_type == ProtoSessionType::PointToPoint {
            // simply return
            return Ok(());
        }

        // set route for the moderator and unsubscribe for the group name
        self.common
            .delete_route(&self.moderator_name.as_ref().unwrap())
            .await;
        let sub = Message::new_unsubscribe(
            &self.common.session.source(),
            self.common.session.dst().as_ref().unwrap(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
        );

        self.common.send_to_slim(sub).await
    }
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
        let source = session.source().clone();
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

impl<P, V, T> SessionComponentLifecycle for SessionModerator<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    fn close(&mut self) {
        todo!()
        // probably we don't need this anymore
    }
}

pub struct SessionModeratorProcessor<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// list of pending task to execute
    tasks_todo: VecDeque<Message>,

    /// the current task executed by the moderator
    /// if it is None the moderator can accept a new task
    current_task: Option<ModeratorTask>,

    /// mls state
    mls_state: Option<MlsModeratorState<P, V>>,

    /// map of the participant in the channel
    /// map from name to u64. The name is the
    /// generic name provided by the app/controller on
    /// invite/remove participant. The val contains the
    /// id of the actual participant found after the
    /// discovery
    group_list: HashMap<Name, u64>,

    /// TODO: add arc a session sender and session receiver
    common: SessionControllerCommon<P, V, T>,

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
            tasks_todo: vec![].into(),
            current_task: None,
            mls_state,
            group_list: HashMap::new(),
            common: SessionControllerCommon {
                conn: None,
                sender,
                rx_from_session_layer,
                tx_to_session_layer,
                session,
            },
            postponed_message: None,
            subscribed: false,
            closing: false,
        }
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                next = self.common.rx_from_session_layer.recv() => {
                    match next {
                        Some(message) => match message {
                            SessionMessage::OnMessage { message, direction: _ } => {
                                if self.common.is_command_message(message.get_session_message_type()) {
                                    self.process_control_message(message).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::SetPointToPointConfig { config } => todo!(),
                            SessionMessage::TimerTimeout { message_id, message_type, name: _, timeouts: _ } => {
                                if self.common.is_command_message(message_type) {
                                    self.common.sender.on_timer_timeout(message_id).await;
                                } else {
                                    todo!()
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name: _, timeouts: _ } => {
                                if self.common.is_command_message(message_type) {
                                    self.common.sender.on_timer_failure(message_id).await;
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
                                    self.common.session
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
                            debug!("session controller close channel {}", self.common.session.id());
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

                // if the message contains a payaload and the name is the same as the
                // local one, call the delete all anyway
                if let Some(n) = message
                    .get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_leave_request_payload()
                    .destination
                    && n == self.common.session.source().into()
                {
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
            | ProtoSessionMessageType::GroupNack => Err(SessionError::Processing(format!(
                "Unexpected control message type {:?}",
                message.get_session_message_type()
            ))),
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message.get_session_message_type()
            ))),
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
                self.common.conn = Some(msg.get_incoming_conn());

                // set the route to forward the messages correctly
                let dst = Name::from(&dst_name);
                self.common.set_route(&dst).await?;

                // create a new empty payload and change the message destination
                let p = CommandPayload::new_discovery_request_payload(None).as_content();
                msg.get_slim_header_mut()
                    .set_source(&self.common.session.source());
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
        self.common.send_with_timer(discovery).await
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        // update sender status to stop timers
        self.common.sender.on_message(&msg).await?;

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
            self.common.session.session_config().max_retries(),
            self.common.session.session_config().timer_duration(),
            Some(self.common.session.dst().unwrap()),
        )
        .as_content();

        self.common
            .send_control_message(
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
        self.common.sender.on_message(&msg).await?;

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
            self.common
                .send_control_message(
                    &self.common.session.dst().unwrap(),
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
        self.common
            .send_control_message(
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
                msg.get_slim_header_mut()
                    .set_source(&self.common.session.source());
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
            self.common
                .send_control_message(
                    &self.common.session.dst().unwrap(),
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
            self.common.sender.on_message(&leave_message).await;

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
            let leave = self.common.create_control_message(
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
        self.common.sender.on_message(&msg).await?;
        if !self.common.sender.is_still_pending(msg_id) {
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
        }

        self.task_done().await
    }

    async fn on_group_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        // notify the sender
        self.common.sender.on_message(&msg);

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.common.sender.is_still_pending(msg.get_id()) {
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
                    self.common.sender.on_message(leave_message).await;
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

    async fn join(&mut self, in_conn: u64) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.common.conn = Some(in_conn);

        let sub = Message::new_subscribe(
            &self.common.session.source(),
            self.common.session.dst().as_ref().unwrap(),
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
        );

        self.common.send_to_slim(sub).await?;
        self.common
            .set_route(self.common.session.dst().as_ref().unwrap());

        // create mls group if needed
        if let Some(mls) = self.mls_state.as_mut() {
            mls.init_moderator()?;
        }

        // add ourself to the participants
        let local_name = self.common.session.source().clone();
        let id = local_name.id();
        self.group_list.insert(local_name, id);

        Ok(())
    }

    async fn send_close_signal(&mut self) {
        // TODO check if the senders/receivers are happy as well
        debug!("Signal session layer to close the session, all tasks are done");

        // delete route for the channel
        self.common
            .delete_route(&self.common.session.dst().unwrap());

        // notify the session layer
        let res = self
            .common
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.session.id(),
            }))
            .await;

        if res.is_err() {
            error!("an error occured while signaling session close");
        }
    }

    async fn ack_msl_proposal(&mut self, _msg: &Message) -> Result<(), SessionError> {
        todo!()
    }

    async fn on_mls_proposal(&mut self, _msg: Message) -> Result<(), SessionError> {
        todo!()
    }
}

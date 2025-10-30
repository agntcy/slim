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
use parking_lot::Mutex;
use slim_mls::mls::Mls;
use tracing::{debug, error};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        ApplicationPayload, CommandPayload, Content, MlsPayload, ProtoMessage as Message,
        ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    MessageDirection, SessionError, Transmitter,
    common::SessionMessage,
    controller_sender::ControllerSender,
    mls_state::{MlsModeratorState, MlsState},
    moderator_task::{AddParticipant, ModeratorTask, RemoveParticipant, TaskUpdate},
    session::Session,
    timer_factory::TimerSettings,
    transmitter::SessionTransmitter,
};

pub(crate) enum SessionControllerImpl<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    SessionParticipant(SessionParticipant<P, V>),
    SessionModerator(SessionModerator<P, V>),
}

pub struct SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// session id
    id: u32,

    /// local name
    source: Name,

    /// group or remote endpoint name
    destination: Name,

    /// session config
    config: SessionConfig,

    /// controller (participant or moderator)
    controller: SessionControllerImpl<P, V>,
}

impl<P, V> SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let session_config = config.clone();

        let controller = if config.initiator {
            SessionControllerImpl::SessionModerator(SessionModerator::new(
                id,
                source.clone(),
                destination.clone(),
                conn,
                config,
                identity_provider,
                identity_verifier,
                storage_path,
                tx,
                tx_to_session_layer,
            ))
        } else {
            SessionControllerImpl::SessionParticipant(SessionParticipant::new(
                id,
                source.clone(),
                destination.clone(),
                conn,
                config,
                identity_provider,
                identity_verifier,
                storage_path,
                tx,
                tx_to_session_layer,
            ))
        };

        SessionController {
            id,
            source,
            destination,
            config: session_config,
            controller,
        }
    }

    /// getters
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn source(&self) -> &Name {
        &self.source
    }

    pub fn dst(&self) -> &Name {
        &self.destination
    }

    pub fn session_type(&self) -> ProtoSessionType {
        self.config.session_type
    }

    pub fn metadata(&self) -> HashMap<String, String> {
        self.config.metadata.clone()
    }

    pub fn session_config(&self) -> SessionConfig {
        self.config.clone()
    }

    pub fn is_initiator(&self) -> bool {
        match self.controller {
            SessionControllerImpl::SessionParticipant(_) => false,
            SessionControllerImpl::SessionModerator(_) => true,
        }
    }

    /// public functions
    pub async fn on_message(
        &self,
        msg: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match &self.controller {
            SessionControllerImpl::SessionParticipant(cp) => cp.on_message(msg, direction).await,
            SessionControllerImpl::SessionModerator(cm) => cm.on_message(msg, direction).await,
        }
    }

    pub fn close(&mut self) {
        todo!()
        // still needed?
    }

    pub async fn publish_message(&self, message: Message) -> Result<(), SessionError> {
        self.on_message(message, MessageDirection::South).await
    }

    /// Publish a message to a specific connection (forward_to)
    pub async fn publish_to(
        &self,
        name: &Name,
        forward_to: u64,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default().with_forward_to(forward_to),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message to a specific app name
    pub async fn publish(
        &self,
        name: &Name,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        self.publish_with_flags(
            name,
            SlimHeaderFlags::default(),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message with specific flags
    pub async fn publish_with_flags(
        &self,
        name: &Name,
        flags: SlimHeaderFlags,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SessionError> {
        let ct = payload_type.unwrap_or_else(|| "msg".to_string());

        let payload = Some(ApplicationPayload::new(&ct, blob).as_content());

        let mut msg = Message::new_publish(self.source(), name, None, Some(flags), payload);
        if let Some(map) = metadata
            && !map.is_empty()
        {
            msg.set_metadata_map(map);
        }

        // southbound=true means towards slim
        self.publish_message(msg).await
    }

    pub async fn invite_participant(&self, destination: &Name) -> Result<(), SessionError> {
        match self.session_type() {
            ProtoSessionType::PointToPoint => Err(SessionError::Processing(
                "cannot invite participant to point-to-point session".into(),
            )),
            ProtoSessionType::Multicast => {
                if !self.is_initiator() {
                    return Err(SessionError::Processing(
                        "cannot remove participant from this session session".into(),
                    ));
                }
                let slim_header = Some(SlimHeader::new(self.source(), destination, "", None));
                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::Multicast.into(),
                    ProtoSessionMessageType::DiscoveryRequest.into(),
                    self.id(),
                    rand::random::<u32>(),
                ));
                let payload =
                    Some(CommandPayload::new_discovery_request_payload(None).as_content());
                let msg = Message::new_publish_with_headers(slim_header, session_header, payload);
                self.publish_message(msg).await
            }
            _ => Err(SessionError::Processing("unexpected session type".into())),
        }
    }

    pub async fn remove_participant(&self, destination: &Name) -> Result<(), SessionError> {
        match self.session_type() {
            ProtoSessionType::PointToPoint => Err(SessionError::Processing(
                "cannot remove participant to point-to-point session".into(),
            )),
            ProtoSessionType::Multicast => {
                if !self.is_initiator() {
                    return Err(SessionError::Processing(
                        "cannot remove participant from this session session".into(),
                    ));
                }
                let slim_header = Some(SlimHeader::new(self.source(), destination, "", None));
                let session_header = Some(SessionHeader::new(
                    ProtoSessionType::Multicast.into(),
                    ProtoSessionMessageType::LeaveRequest.into(),
                    self.id(),
                    rand::random::<u32>(),
                ));
                let payload =
                    Some(CommandPayload::new_discovery_request_payload(None).as_content());
                let msg = Message::new_publish_with_headers(slim_header, session_header, payload);
                self.publish_message(msg).await
            }
            _ => Err(SessionError::Processing("unexpected session type".into())),
        }
    }
}

pub fn handle_channel_discovery_message(
    message: &Message,
    app_name: &Name,
    session_id: u32,
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

#[derive(Default, Clone)]
pub struct SessionConfig {
    /// session type
    pub session_type: ProtoSessionType,

    /// number of retries for each message/rtx
    pub max_retries: Option<u32>,

    /// time between retries
    pub duration: Option<std::time::Duration>,

    /// true is mls is enabled
    pub mls_enabled: bool,

    /// true is the local endpoint is initiator of the session
    pub initiator: bool,

    /// metadata related to the sessions
    pub metadata: HashMap<String, String>,
}

impl SessionConfig {
    pub fn with_session_type(&self, session_type: ProtoSessionType) -> Self {
        Self {
            session_type,
            max_retries: self.max_retries,
            duration: self.duration,
            mls_enabled: self.mls_enabled,
            initiator: self.initiator,
            metadata: self.metadata.clone(),
        }
    }
}

pub struct SessionControllerCommon {
    /// session id
    id: u32,

    ///local name
    source: Name,

    /// remote/group name
    /// in case of remote the name the id may be updated after the discovery phase
    destination: Name,

    /// connection with the remote slim node where to send/recv messages
    conn: Option<u64>,

    /// session configuration
    config: SessionConfig,

    /// sender for command messages
    sender: ControllerSender,

    /// directly sent to the slim/app
    tx: SessionTransmitter,

    /// channel used to received messages from the session layer
    /// and timeouts from timers
    rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,

    /// channel used to communincate with the session layer.
    tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// the session itself
    /// TODO rename it in session
    session: Session,
}

impl SessionControllerCommon {
    const MAX_FANOUT: u32 = 256;

    #[allow(clippy::too_many_arguments)]
    fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        tx: SessionTransmitter,
        tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,
        rx_controller: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        // timers settings for the controller
        let controller_timer_settings =
            TimerSettings::constant(Duration::from_secs(1)).with_max_retries(10);

        // create the controller sender
        let controller_sender = ControllerSender::new(
            controller_timer_settings,
            source.clone(),
            // send messages to slim/app
            tx.clone(),
            // send signal to the controller
            tx_controller.clone(),
        );

        // create the session
        let session = Session::new(
            id,
            config.clone(),
            &source,
            tx.clone(),
            tx_controller.clone(),
        );

        SessionControllerCommon {
            id,
            source,
            destination,
            conn,
            config,
            sender: controller_sender,
            tx,
            rx_from_session_layer: rx_controller,
            tx_to_session_layer,
            session,
        }
    }

    /// internal and helper functions
    fn is_command_message(&self, message_type: ProtoSessionMessageType) -> bool {
        matches!(
            message_type,
            ProtoSessionMessageType::DiscoveryRequest
                | ProtoSessionMessageType::DiscoveryReply
                | ProtoSessionMessageType::JoinRequest
                | ProtoSessionMessageType::JoinReply
                | ProtoSessionMessageType::LeaveRequest
                | ProtoSessionMessageType::LeaveReply
                | ProtoSessionMessageType::GroupAdd
                | ProtoSessionMessageType::GroupRemove
                | ProtoSessionMessageType::GroupWelcome
                | ProtoSessionMessageType::GroupProposal
                | ProtoSessionMessageType::GroupAck
                | ProtoSessionMessageType::GroupNack
        )
    }

    async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.tx.send_to_slim(Ok(message)).await
    }

    async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
    }

    async fn set_route(&self, name: &Name) -> Result<(), SessionError> {
        let route = Message::new_subscribe(
            &self.source,
            name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(self.conn.unwrap())),
        );

        self.send_to_slim(route).await
    }

    async fn delete_route(&self, name: &Name) -> Result<(), SessionError> {
        let route = Message::new_unsubscribe(
            &self.source,
            name,
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
            &self.source,
            dst,
            "", // put empty identity it will be updated by the identity interceptor
            flags,
        ));

        let session_header = Some(SessionHeader::new(
            self.config.session_type.into(),
            message_type.into(),
            self.id,
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

pub struct SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    tx: tokio::sync::mpsc::Sender<SessionMessage>,
    _phantom: PhantomData<(P, V)>,
}

impl<P, V> SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        // tx/rx controller used to receive messages from the session_layer
        let (tx_controller, rx_controller) = tokio::sync::mpsc::channel(128);

        let processor = SessionParticipantProcessor::new(
            id,
            source,
            destination,
            conn,
            config,
            identity_provider,
            identity_verifier,
            storage_path,
            tx,
            tx_controller.clone(),
            rx_controller,
            tx_to_session_layer,
        );

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        SessionParticipant {
            tx: tx_controller,
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

pub struct SessionParticipantProcessor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// list of participants
    group_list: HashSet<Name>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// common session state
    common: SessionControllerCommon,

    subscribed: bool,
}

impl<P, V> SessionParticipantProcessor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,
        rx_controller: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let mls_state = if config.mls_enabled {
            Some(
                MlsState::new(Arc::new(Mutex::new(Mls::new(
                    source.clone(),
                    identity_provider,
                    identity_verifier,
                    storage_path,
                ))))
                .expect("failed to create MLS state"),
            )
        } else {
            None
        };

        let common = SessionControllerCommon::new(
            id,
            source,
            destination,
            conn,
            config,
            tx,
            tx_controller,
            rx_controller,
            tx_to_session_layer,
        );

        SessionParticipantProcessor {
            moderator_name: None,
            group_list: HashSet::new(),
            mls_state,
            common,
            subscribed: false,
        }
    }

    async fn process_loop(mut self) {
        loop {
            tokio::select! {
                next = self.common.rx_from_session_layer.recv() => {
                    match next {
                        Some(message) => match message {
                            SessionMessage::OnMessage { message, direction } => {
                                if self.common.is_command_message(message.get_session_message_type()) {
                                    if let Err(e) = self.process_control_message(message).await {
                                        error!("Error processing control message: {:?}", e);
                                    }
                                } else if let Err(e) = self.common.session.on_message(SessionMessage::OnMessage { message, direction }).await {
                                    error!("Error drnding message to the session: {:?}", e);
                                }
                            }
                            SessionMessage::TimerTimeout { message_id, message_type, name, timeouts } => {
                                if self.common.is_command_message(message_type) {
                                    // check it needed
                                    if let Err(e) = self.common.sender.on_timer_timeout(message_id).await {
                                        error!("Error processing timeout for control message: {:?}", e);
                                    }
                                } else if let Err(e) = self.common.session.on_message(SessionMessage::TimerTimeout { message_id, message_type, name, timeouts }).await {
                                    error!("Error processing timeout in the session: {:?}", e);
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name, timeouts } => {
                                if self.common.is_command_message(message_type) {
                                    // check if needed
                                    self.common.sender.on_timer_failure(message_id).await;
                                } else if let Err(e) = self.common.session.on_message(SessionMessage::TimerFailure { message_id, message_type, name, timeouts }).await {
                                    error!("Error processing timer failure in the session: {:?}", e);
                                }
                            }
                            SessionMessage::DeleteSession { session_id: _ } => todo!(),
                            SessionMessage::Drain { grace_period_ms: _  } => todo!(),
                            _ => {
                                debug!("Unexpected message type");
                            }
                        }
                        None => {
                            debug!("session controller close channel {}", self.common.id);
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
            ProtoSessionMessageType::GroupWelcome => self.on_welcome(message).await,
            ProtoSessionMessageType::GroupAdd => self.on_group_update_message(message, true).await,
            ProtoSessionMessageType::GroupRemove => {
                self.on_group_update_message(message, false).await
            }
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
        debug!("received a join request");
        // set local state with the moderator params
        let source = msg.get_source();
        self.moderator_name = Some(source.clone());
        self.common.conn = Some(msg.get_incoming_conn());

        // set route in order to be able to send packets to the moderator
        self.common.set_route(&source).await?;

        // send reply to the moderator
        let payload = if self.mls_state.is_some() {
            debug!("mls enabled, create the package key");
            // if mls we need to provide the key package
            let key = self.mls_state.as_mut().unwrap().generate_key_package()?;
            Some(key)
        } else {
            // without MLS we can set the state for the channel
            // otherwise the endpoint needs to receive a
            // welcome message first
            self.join().await?;
            None
        };

        let content = CommandPayload::new_join_reply_payload(payload).as_content();

        debug!("send join reply message");
        // reply to the request
        let reply = self.common.create_control_message(
            &source,
            ProtoSessionMessageType::JoinReply,
            msg.get_id(),
            content,
            false,
        );

        self.common.send_to_slim(reply).await
    }

    async fn on_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!("process welcome message");

        if self.mls_state.is_some() {
            self.mls_state
                .as_mut()
                .unwrap()
                .process_welcome_message(&msg)?;
        }

        // set route for the channel name
        self.join().await?;

        // setup the local participant list
        let list = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_welcome_payload()
            .participants;
        for n in list {
            let name = Name::from(&n);
            self.group_list.insert(name.clone());

            if name != self.common.source {
                // notify the local session that a new participant was added to the group
                debug!("add endpoint to the session {}", msg.get_source());
                self.common
                    .session
                    .on_message(SessionMessage::AddEndpoint { endpoint: name })
                    .await?;
            }
        }

        // send an ack back to the moderator
        let ack = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::new_group_ack_payload().as_content(),
            false,
        );

        self.common.send_to_slim(ack).await
    }

    async fn on_group_update_message(
        &mut self,
        msg: Message,
        add: bool,
    ) -> Result<(), SessionError> {
        // process the control message
        if self.mls_state.is_some() {
            debug!("process mls control update");
            let ret = self
                .mls_state
                .as_mut()
                .unwrap()
                .process_control_message(msg.clone(), &self.common.source)?;

            if !ret {
                // message already processed, drop it
                debug!(
                    "Message with id {} already processed, drop it",
                    msg.get_id()
                );
                return Ok(());
            }
        }

        if add {
            let p = msg
                .get_payload()
                .unwrap()
                .as_command_payload()
                .as_group_add_payload();
            if let Some(ref new_participant) = p.new_participant {
                let name = Name::from(new_participant);
                self.group_list.insert(name.clone());

                // notify the local session that a new participant was added to the group
                debug!("add endpoint to the session {}", msg.get_source());
                self.common
                    .session
                    .on_message(SessionMessage::AddEndpoint { endpoint: name })
                    .await?;
            }
        } else {
            let p = msg
                .get_payload()
                .unwrap()
                .as_command_payload()
                .as_group_remove_payload();
            if let Some(ref removed_participant) = p.removed_participant {
                let name = Name::from(removed_participant);
                self.group_list.remove(&name);

                // notify the local session that a new participant was added to the group
                debug!("add endpoint to the session {}", msg.get_source());
                self.common
                    .session
                    .on_message(SessionMessage::RemoveEndpoint { endpoint: name })
                    .await?;
            }
        }

        // send an ack back to the moderator
        let msg = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::new_group_ack_payload().as_content(),
            false,
        );
        self.common.send_to_slim(msg).await
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        // reply to the sender
        let reply = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::LeaveReply,
            msg.get_id(),
            CommandPayload::new_leave_reply_payload().as_content(),
            false,
        );

        self.common.send_to_slim(reply).await?;

        // leave the channel
        self.leave().await?;

        // notify the session layer that the session can be removed
        self.common
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.id,
            }))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to notify session layer: {}", e)))
    }

    #[allow(dead_code)]
    async fn on_mls_ack_or_nack(&mut self, _msg: Message) -> Result<(), SessionError> {
        todo!()
    }

    /// helper functions
    async fn join(&mut self) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        // we need to setup the network only in case of multicast session

        if self.common.config.session_type == ProtoSessionType::PointToPoint {
            // simply return
            return Ok(());
        }

        // set route and subscription for the group name
        self.common.set_route(&self.common.destination).await?;
        let sub = Message::new_subscribe(
            &self.common.source,
            &self.common.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
        );

        self.common.send_to_slim(sub).await
    }

    async fn leave(&self) -> Result<(), SessionError> {
        // delete route to the remote endpoint
        self.common.delete_route(&self.common.destination).await?;

        // we need to setup the network only in case of multicast session
        if self.common.config.session_type == ProtoSessionType::PointToPoint {
            // simply return
            return Ok(());
        }

        // set route for the moderator and unsubscribe for the group name
        self.common
            .delete_route(self.moderator_name.as_ref().unwrap())
            .await?;
        let sub = Message::new_unsubscribe(
            &self.common.source,
            &self.common.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
        );

        self.common.send_to_slim(sub).await
    }
}

impl<P, V> SessionModerator<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        // tx/rx controller used to receive messages from the session_layer
        let (tx_controller, rx_controller) = tokio::sync::mpsc::channel(128);

        let processor = SessionModeratorProcessor::new(
            id,
            source,
            destination,
            conn,
            config,
            identity_provider,
            identity_verifier,
            storage_path,
            tx,
            tx_controller.clone(),
            rx_controller,
            tx_to_session_layer,
        );

        // Start the processor loop
        tokio::spawn(processor.process_loop());

        SessionModerator {
            tx: tx_controller,
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
pub struct SessionModerator<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    tx: tokio::sync::mpsc::Sender<SessionMessage>,
    _phantom: PhantomData<(P, V)>,
}

pub struct SessionModeratorProcessor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
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
    common: SessionControllerCommon,

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

impl<P, V> SessionModeratorProcessor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u32,
        source: Name,
        destination: Name,
        conn: Option<u64>,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,
        rx_controller: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
    ) -> Self {
        let mls_state = if config.mls_enabled {
            Some(MlsModeratorState::new(
                MlsState::new(Arc::new(Mutex::new(Mls::new(
                    source.clone(),
                    identity_provider.clone(),
                    identity_verifier.clone(),
                    storage_path,
                ))))
                .expect("failed to create MLS state"),
            ))
        } else {
            None
        };

        let common = SessionControllerCommon::new(
            id,
            source,
            destination,
            conn,
            config,
            tx,
            tx_controller,
            rx_controller,
            tx_to_session_layer,
        );

        SessionModeratorProcessor {
            tasks_todo: vec![].into(),
            current_task: None,
            mls_state,
            group_list: HashMap::new(),
            common,
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
                            SessionMessage::OnMessage { mut message, direction } => {
                                if self.common.is_command_message(message.get_session_message_type()) {
                                    if let Err(e)= self.process_control_message(message).await {
                                        error!("Error processeing command message: {}", e);
                                    }
                                } else {
                                    // this is a application message. if direction (needs to go to the remote endpoint) and
                                    // the session is p2p, update the destination of the message with the destination in
                                    // the self.common. In this way we always add the right id to the name
                                    if direction == MessageDirection::South && self.common.config.session_type == ProtoSessionType::PointToPoint {
                                        message.get_slim_header_mut().set_destination(&self.common.destination);
                                    }
                                    if let Err(e) = self.common.session.on_message(SessionMessage::OnMessage { message, direction }).await {
                                        error!("Error sending message to the session: {}", e);
                                    }
                                }
                            }
                            SessionMessage::TimerTimeout { message_id, message_type, name, timeouts } => {
                                if self.common.is_command_message(message_type) {
                                    if let Err(e)=  self.common.sender.on_timer_timeout(message_id).await {
                                        error!("Error processeing timeout: {}", e);
                                    }
                                } else if let Err(e)=  self.common.session.on_message(SessionMessage::TimerTimeout { message_id, message_type, name, timeouts }).await {
                                        error!("Error sending timeout to the session: {}", e);
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name, timeouts} => {
                                if self.common.is_command_message(message_type) {
                                    self.common.sender.on_timer_failure(message_id).await;
                                    // the current task failed:
                                    // 1. create the right error message
                                    let error_message = match self.current_task.as_ref().unwrap() {
                                        ModeratorTask::Add(_) => {
                                            "failed to add a participant to the group"
                                        }
                                        ModeratorTask::Remove(_) => {
                                            "failed to remove a participant from the group"
                                        }
                                        ModeratorTask::Update(_) => {
                                            "failed to update state of the participant"
                                        }
                                    };

                                    // 2. notify the application
                                    if let Err(e) = self.common.tx.send_to_app(Err(SessionError::ModeratorTask(error_message.to_string()))).await {
                                        error!("failed to notify application: {}", e);
                                    }

                                    // 3. delete current task and pick a new one
                                    self.current_task = None;
                                    if let Err(e) = self.pop_task().await {
                                        error!("Failed to pop next task: {:?}", e);
                                    }
                                } else if let Err(e) = self.common.session.on_message(SessionMessage::TimerFailure { message_id, message_type, name, timeouts }).await {
                                        error!("failed to sending timer failure to the session: {}", e);
                                }
                            }
                            SessionMessage::DeleteSession { session_id: _ } => todo!(),
                            SessionMessage::Drain { grace_period_ms: _ } => todo!(),
                            _ => {
                                debug!("Unexpected message type");
                            }
                        }
                        None => {
                            debug!("session controller close channel {}", self.common.id);
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
                    && Name::from(&n) == self.common.source
                {
                    return self.delete_all(message).await;
                }

                // otherwise start the leave process
                self.on_leave_request(message).await
            }
            ProtoSessionMessageType::LeaveReply => self.on_leave_reply(message).await,
            ProtoSessionMessageType::GroupAck => self.on_group_ack(message).await,
            ProtoSessionMessageType::GroupProposal => todo!(),
            ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
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
        debug!("received discovery request");
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
        self.current_task = Some(ModeratorTask::Add(AddParticipant::default()));

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
                msg.get_slim_header_mut().set_source(&self.common.source);
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
        self.current_task.as_mut().unwrap().discovery_start(id)?;

        // send the message
        self.common.send_with_timer(discovery).await
    }

    async fn on_discovery_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!("received discovery reply");
        // update sender status to stop timers
        self.common.sender.on_message(&msg).await?;

        // evolve the current task state
        // the discovery phase is completed
        self.current_task
            .as_mut()
            .unwrap()
            .discovery_complete(msg.get_id())?;

        // join the channel if needed
        self.join(msg.get_source(), msg.get_incoming_conn()).await?;

        // set a route to the remote participant
        self.common.set_route(&msg.get_source()).await?;

        // an endpoint replied to the discovery message
        // send a join message
        let msg_id = rand::random::<u32>();

        let channel = if self.common.config.session_type == ProtoSessionType::Multicast {
            Some(self.common.destination.clone())
        } else {
            None
        };

        let payload = CommandPayload::new_join_request_payload(
            self.mls_state.is_some(),
            self.common.config.max_retries,
            self.common.config.duration,
            channel,
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
        debug!("process join reply");
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

        // notify the local session that a new participant was added to the group
        debug!("add endpoint to the session {}", msg.get_source());
        self.common
            .session
            .on_message(SessionMessage::AddEndpoint {
                endpoint: msg.get_source().clone(),
            })
            .await?;

        // get mls data if MLS is enabled
        let (commit, welcome) = if self.mls_state.is_some() {
            let (commit_payload, welcome_payload) =
                self.mls_state.as_mut().unwrap().add_participant(&msg)?;

            // get the id of the commit, the welcome message has a random id
            let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();

            let commit = MlsPayload {
                commit_id,
                mls_content: commit_payload,
            };
            let welcome = MlsPayload {
                commit_id,
                mls_content: welcome_payload,
            };

            (Some(commit), Some(welcome))
        } else {
            (None, None)
        };

        // Create participants list for the messages to send
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        // send the group update
        if participants_vec.len() > 2 {
            debug!("participant len is > 2, send a group update");
            let update_payload = CommandPayload::new_group_add_payload(
                msg.get_source().clone(),
                participants_vec.clone(),
                commit,
            )
            .as_content();
            let add_msg_id = rand::random::<u32>();
            self.common
                .send_control_message(
                    &self.common.destination.clone(),
                    ProtoSessionMessageType::GroupAdd,
                    add_msg_id,
                    update_payload,
                    true,
                )
                .await?;
            self.current_task
                .as_mut()
                .unwrap()
                .commit_start(add_msg_id)?;
        } else {
            // no commit message will be sent so update the task state to consider the commit as received
            // the timer id is not important here, it just need to be consistent
            debug!("cancel the a group update task");
            self.current_task.as_mut().unwrap().commit_start(12345)?;
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(12345)?;
        }

        // send welcome message
        let welcome_msg_id = rand::random::<u32>();
        let welcome_payload =
            CommandPayload::new_group_welcome_payload(participants_vec, welcome).as_content();
        self.common
            .send_control_message(
                &msg.get_slim_header().get_source(),
                ProtoSessionMessageType::GroupWelcome,
                welcome_msg_id,
                welcome_payload,
                false,
            )
            .await?;

        // evolve the current task state
        // welcome start
        self.current_task
            .as_mut()
            .unwrap()
            .welcome_start(welcome_msg_id)?;

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
        self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::default()));

        // adjust the message according to the sender:
        // - if coming from the controller (destination in the payload) we need to modify source and destination
        // - if coming from the app (empty payload) we need to add the participant id to the destination
        let payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()
            .as_leave_request_payload();

        let (leave_message, dst_without_id) = match payload.destination {
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
                msg.get_slim_header_mut().set_source(&self.common.source);
                msg.get_slim_header_mut().set_destination(&dst);
                msg.set_payload(new_payload);
                msg.set_message_id(rand::random::<u32>());
                (msg, Name::from(&dst_name))
            }
            None => {
                // Handle case where no destination is provided, use message destination
                let original_dst = msg.get_dst();
                let dst = msg.get_dst();
                let id = *self
                    .group_list
                    .get(&dst)
                    .ok_or(SessionError::RemoveParticipant(
                        "participant not found".to_string(),
                    ))?;

                msg.get_slim_header_mut().set_destination(&dst.with_id(id));
                msg.set_message_id(rand::random::<u32>());
                (msg, original_dst)
            }
        };

        // remove the participant from the group list and notify the the local session
        debug!(
            "remove endpoint from the session {}",
            leave_message.get_dst()
        );

        self.group_list.remove(&dst_without_id);

        self.common
            .session
            .on_message(SessionMessage::RemoveEndpoint {
                endpoint: leave_message.get_dst(),
            })
            .await?;

        // Before send the leave request we may need to send the Group update
        // with the new participant list and the new mls payload if needed
        // Create participants list for commit message
        let mut participants_vec = vec![];
        for (n, id) in &self.group_list {
            let name = n.clone().with_id(*id);
            participants_vec.push(name);
        }

        if participants_vec.len() > 2 {
            // in this case we need to send first the group update and later the leave message
            let mls_payload = match self.mls_state.as_mut() {
                Some(state) => {
                    let mls_content = state.remove_participant(&leave_message)?;
                    let commit_id = self.mls_state.as_mut().unwrap().get_next_mls_mgs_id();
                    Some(MlsPayload {
                        commit_id,
                        mls_content,
                    })
                }
                None => None,
            };

            let update_payload = CommandPayload::new_group_remove_payload(
                leave_message.get_dst(),
                participants_vec,
                mls_payload,
            )
            .as_content();
            let msg_id = rand::random::<u32>();
            self.common
                .send_control_message(
                    &self.common.destination.clone(),
                    ProtoSessionMessageType::GroupRemove,
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
            self.current_task.as_mut().unwrap().commit_start(12345)?;
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(12345)?;

            // just send the leave message in this case
            self.common.sender.on_message(&leave_message).await?;

            self.current_task
                .as_mut()
                .unwrap()
                .leave_start(leave_message.get_id())?;
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
                self.current_task = Some(ModeratorTask::Remove(RemoveParticipant::default()));
                self.on_leave_request(m).await
            }
            None => {
                self.send_close_signal().await;
                Ok(())
            }
        }
    }

    async fn on_leave_reply(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!("process leave reply");
        let msg_id = msg.get_id();

        // notify the sender and see if we can pick another task
        self.common.sender.on_message(&msg).await?;
        if !self.common.sender.is_still_pending(msg_id) {
            self.current_task.as_mut().unwrap().leave_complete(msg_id)?;
        }

        self.task_done().await
    }

    async fn on_group_ack(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!("process group ack");
        // notify the sender
        self.common.sender.on_message(&msg).await?;

        // check if the timer is done
        let msg_id = msg.get_id();
        if !self.common.sender.is_still_pending(msg_id) {
            debug!(
                "process group ack for message {}. try to close task",
                msg_id
            );
            // we received all the messages related to this timer
            // check if we are done and move on
            self.current_task
                .as_mut()
                .unwrap()
                .update_phase_completed(msg_id)?;

            // check if the task is finished.
            if self.current_task.as_mut().unwrap().task_complete() {
                // task done. At this point if this was the first MLS
                // action MLS is setup so we can set mls_up to true
                if self.mls_state.is_some() {
                    self.mls_state.as_mut().unwrap().common.mls_up = true;
                }
            } else {
                // if the task is not finished yet we may need to send a leave
                // message that was postponed to send all group update first
                if self.postponed_message.is_some()
                    && matches!(self.current_task, Some(ModeratorTask::Remove(_)))
                {
                    // send the leave message an progress
                    let leave_message = self.postponed_message.as_ref().unwrap();
                    self.common.sender.on_message(leave_message).await?;
                    self.current_task
                        .as_mut()
                        .unwrap()
                        .leave_start(leave_message.get_id())?;
                    // rest the postponed message
                    self.postponed_message = None;
                }
            }

            // check if we can progress with another task
            self.task_done().await?;
        } else {
            debug!(
                "timer for message {} is still pending, do not close the task",
                msg_id
            );
        }

        Ok(())
    }

    /// task handling functions
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
        Box::pin(self.process_control_message(msg)).await
    }

    async fn join(&mut self, remote: Name, in_conn: u64) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        self.common.conn = Some(in_conn);

        // if this is a multicast session we need to subscribe for the channel name
        // otherwise we update the destination name with the full name of the remote participant
        if self.common.config.session_type == ProtoSessionType::Multicast {
            // subscribe for the channel
            let sub = Message::new_subscribe(
                &self.common.source,
                &self.common.destination,
                None,
                Some(SlimHeaderFlags::default().with_forward_to(self.common.conn.unwrap())),
            );

            self.common.send_to_slim(sub).await?;

            // set the route for the channel
            self.common.set_route(&self.common.destination).await?;
        } else {
            self.common.destination = remote;
        }

        // create mls group if needed
        if let Some(mls) = self.mls_state.as_mut() {
            mls.init_moderator()?;
        }

        // add ourself to the participants
        let local_name = self.common.source.clone();
        let id = local_name.id();
        self.group_list.insert(local_name, id);

        Ok(())
    }

    async fn send_close_signal(&mut self) {
        // TODO check if the senders/receivers are happy as well
        debug!("Signal session layer to close the session, all tasks are done");

        // delete route for the channel
        if let Err(e) = self.common.delete_route(&self.common.destination).await {
            error!("error with removing route {}", e);
        }

        // notify the session layer
        let res = self
            .common
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.id,
            }))
            .await;

        if res.is_err() {
            error!("an error occurred while signaling session close");
        }
    }

    #[allow(dead_code)]
    async fn ack_msl_proposal(&mut self, _msg: &Message) -> Result<(), SessionError> {
        todo!()
    }

    #[allow(dead_code)]
    async fn on_mls_proposal(&mut self, _msg: Message) -> Result<(), SessionError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use crate::transmitter::SessionTransmitter;

    use super::*;
    use slim_auth::shared_secret::SharedSecret;
    use std::time::Duration;
    use tokio::time::timeout;
    use tracing_test::traced_test;

    const SHARED_SECRET: &str = "kjandjansdiasb8udaijdniasdaindasndasndasndasndasndasndasndas";

    #[tokio::test]
    #[traced_test]
    async fn test_end_to_end_p2p() {
        let session_id = 10;
        let moderator_name = Name::from_strings(["org", "ns", "moderator"]).with_id(1);
        let participant_name = Name::from_strings(["org", "ns", "participant"]);
        let participant_name_id = Name::from_strings(["org", "ns", "participant"]).with_id(1);
        let storage_path_moderator = std::path::PathBuf::from("/tmp/test_invite_moderator");
        let storage_path_participant = std::path::PathBuf::from("/tmp/test_invite_participant");

        // create a SessionModerator
        let (tx_slim_moderator, mut rx_slim_moderator) = tokio::sync::mpsc::channel(10);
        let (tx_app_moderator, _rx_app_moderator) = tokio::sync::mpsc::channel(10);
        let (tx_session_layer_moderator, _rx_session_layer_moderator) =
            tokio::sync::mpsc::channel(10);

        let tx_moderator =
            SessionTransmitter::new(tx_slim_moderator.clone(), tx_app_moderator.clone());

        let moderator_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            duration: Some(Duration::from_millis(200)),
            mls_enabled: true,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };

        let moderator = SessionModerator::<SharedSecret, SharedSecret>::new(
            session_id,
            moderator_name.clone(),
            participant_name.clone(),
            None,
            moderator_config,
            SharedSecret::new("moderator", SHARED_SECRET),
            SharedSecret::new("moderator", SHARED_SECRET),
            storage_path_moderator.clone(),
            tx_moderator.clone(),
            tx_session_layer_moderator,
        );

        // create a SessionParticipant
        let (tx_slim_participant, mut rx_slim_participant) = tokio::sync::mpsc::channel(10);
        let (tx_app_participant, mut rx_app_participant) = tokio::sync::mpsc::channel(10);
        let (tx_session_layer_participant, _rx_session_layer_participant) =
            tokio::sync::mpsc::channel(10);

        let tx_participant =
            SessionTransmitter::new(tx_slim_participant.clone(), tx_app_participant.clone());

        let participant_config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
            max_retries: Some(5),
            duration: Some(Duration::from_millis(200)),
            mls_enabled: true,
            initiator: false,
            metadata: std::collections::HashMap::new(),
        };

        let participant = SessionParticipant::<SharedSecret, SharedSecret>::new(
            session_id,
            participant_name_id.clone(),
            moderator_name.clone(),
            None,
            participant_config,
            SharedSecret::new("participant", SHARED_SECRET),
            SharedSecret::new("participant", SHARED_SECRET),
            storage_path_participant.clone(),
            tx_participant.clone(),
            tx_session_layer_participant,
        );

        // create a discovery request and send it on the moderator on message (direction south)
        let slim_header = Some(SlimHeader::new(
            &moderator_name,
            &participant_name,
            "",
            None,
        ));
        let session_header = Some(SessionHeader::new(
            slim_datapath::api::ProtoSessionType::PointToPoint.into(),
            slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest.into(),
            session_id,
            1, // this id will be changed by the session controller
        ));
        let payload = Some(CommandPayload::new_discovery_request_payload(None).as_content());
        let discovery_request =
            Message::new_publish_with_headers(slim_header, session_header, payload);

        moderator
            .on_message(discovery_request.clone(), MessageDirection::South)
            .await
            .expect("error sending discovery request");

        // check that the request is received by slim on the moderator
        let received_discovery_request =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv())
                .await
                .expect("timeout waiting for discovery request on moderator slim channel")
                .expect("channel closed")
                .expect("error in discovery request");

        assert_eq!(
            received_discovery_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::DiscoveryRequest
        );

        let discovery_msg_id = received_discovery_request.get_id();

        // create a discovery reply and call the on message on the moderator with the reply (direction north)
        let slim_header = Some(SlimHeader::new(
            &participant_name_id,
            &moderator_name,
            "",
            Some(SlimHeaderFlags::default().with_forward_to(1)),
        ));
        let session_header = Some(SessionHeader::new(
            slim_datapath::api::ProtoSessionType::PointToPoint.into(),
            slim_datapath::api::ProtoSessionMessageType::DiscoveryReply.into(),
            session_id,
            discovery_msg_id,
        ));
        let payload = Some(CommandPayload::new_discovery_reply_payload().as_content());
        let mut discovery_reply =
            Message::new_publish_with_headers(slim_header, session_header, payload);
        discovery_reply
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message(discovery_reply, MessageDirection::North)
            .await
            .expect("error processing discovery reply on moderator");

        // check that we get a route for the remote endpoint on slim
        let route = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        // check that the route message type is a subscription, the destination name is remote and the flag recv_from is set to 1
        assert!(route.is_subscribe(), "route should be a subscribe message");
        assert_eq!(route.get_dst(), participant_name_id);
        assert_eq!(route.get_slim_header().get_recv_from(), Some(1));

        // check that a join request is received by slim
        let join_request = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        assert_eq!(
            join_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::JoinRequest
        );
        assert_eq!(join_request.get_dst(), participant_name_id);

        // call the on message on the participant side with the join request (direction north)
        let mut join_request_to_participant = join_request.clone();
        join_request_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message(join_request_to_participant, MessageDirection::North)
            .await
            .expect("error processing join request on participant");

        // check that a route for the moderator is generated
        let route = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for route on moderator slim channel")
            .expect("channel closed")
            .expect("error in route");

        assert!(route.is_subscribe(), "route should be a subscribe message");
        assert_eq!(route.get_dst(), moderator_name);
        assert_eq!(route.get_slim_header().get_recv_from(), Some(1));

        // check that a join reply is received by slim on the participant
        let join_reply = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for join reply on participant slim channel")
            .expect("channel closed")
            .expect("error in join reply");

        assert_eq!(
            join_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::JoinReply
        );
        assert_eq!(join_reply.get_dst(), moderator_name);

        // call the on message on the moderator with the reply (direction north)
        let mut join_reply_to_moderator = join_reply.clone();
        join_reply_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message(join_reply_to_moderator, MessageDirection::North)
            .await
            .expect("error processing join reply on moderator");

        // check that a welcome message is received by slim on the moderator
        let welcome_message = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for welcome message on moderator slim channel")
            .expect("channel closed")
            .expect("error in welcome message");

        assert_eq!(
            welcome_message.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::GroupWelcome
        );
        assert_eq!(welcome_message.get_dst(), participant_name_id);

        // call the on message on the participant side with the welcome message (direction north)
        let mut welcome_to_participant = welcome_message.clone();
        welcome_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message(welcome_to_participant, MessageDirection::North)
            .await
            .expect("error processing welcome message on participant");

        // check that an ack group is received by slim on the participant
        let ack_group = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for ack group on participant slim channel")
            .expect("channel closed")
            .expect("error in ack group");

        assert_eq!(
            ack_group.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::GroupAck
        );
        assert_eq!(ack_group.get_dst(), moderator_name);

        // call the on message on the moderator with the ack (direction north)
        let mut ack_to_moderator = ack_group.clone();
        ack_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message(ack_to_moderator, MessageDirection::North)
            .await
            .expect("error processing ack on moderator");

        // no other message should be sent
        let no_more_moderator = timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;
        assert!(
            no_more_moderator.is_err(),
            "Expected no more messages on moderator slim channel"
        );

        let no_more_participant =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant.is_err(),
            "Expected no more messages on participant slim channel"
        );

        // create an application message using the participant name
        let app_data = b"Hello from moderator to participant".to_vec();
        let slim_header = Some(SlimHeader::new(
            &moderator_name,
            &participant_name,
            "",
            None,
        ));
        let session_header = Some(SessionHeader::new(
            slim_datapath::api::ProtoSessionType::PointToPoint.into(),
            slim_datapath::api::ProtoSessionMessageType::Msg.into(),
            session_id,
            1,
        ));
        let payload = Some(ApplicationPayload::new("test-app-data", app_data.clone()).as_content());
        let app_message = Message::new_publish_with_headers(slim_header, session_header, payload);

        // call on message on the moderator (direction south)
        moderator
            .on_message(app_message.clone(), MessageDirection::South)
            .await
            .expect("error sending application message from moderator");

        // check that message is received from slim with destination equal to participant name id
        let app_msg_to_slim = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for application message on moderator slim channel")
            .expect("channel closed")
            .expect("error in application message");

        assert_eq!(app_msg_to_slim.get_dst(), participant_name_id);
        assert!(
            app_msg_to_slim.is_publish(),
            "message should be a publish message"
        );

        let app_msg_id = app_msg_to_slim.get_id();

        // call the on message on the participant (direction north)
        let mut app_msg_to_participant = app_msg_to_slim.clone();
        app_msg_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message(app_msg_to_participant, MessageDirection::North)
            .await
            .expect("error processing application message on participant");

        // check that the message is received by the application
        let app_msg_received = timeout(Duration::from_millis(100), rx_app_participant.recv())
            .await
            .expect("timeout waiting for application message on participant app channel")
            .expect("channel closed")
            .expect("error in application message to app");

        assert_eq!(app_msg_received.get_source(), moderator_name);
        assert!(
            app_msg_received.is_publish(),
            "message should be a publish message"
        );
        let content = app_msg_received
            .get_payload()
            .unwrap()
            .as_application_payload()
            .blob
            .clone();
        assert_eq!(content, app_data);

        // check that an ack is sent to slim
        let ack_msg = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for ack on participant slim channel")
            .expect("channel closed")
            .expect("error in ack");

        assert_eq!(
            ack_msg.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::MsgAck,
            "message should be an ack"
        );
        assert_eq!(ack_msg.get_dst(), moderator_name);
        assert_eq!(ack_msg.get_id(), app_msg_id);

        // call the on message with the ack on the moderator
        let mut ack_to_moderator = ack_msg.clone();
        ack_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message(ack_to_moderator, MessageDirection::North)
            .await
            .expect("error processing ack on moderator");

        // check that no other message is generated
        let no_more_moderator_after_ack =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;
        assert!(
            no_more_moderator_after_ack.is_err(),
            "Expected no more messages on moderator slim channel after ack"
        );

        let no_more_participant_after_ack =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant_after_ack.is_err(),
            "Expected no more messages on participant slim channel after ack"
        );

        // create a leave request and send to moderator on message (direction south)
        let slim_header = Some(SlimHeader::new(
            &moderator_name,
            &participant_name,
            "",
            None,
        ));
        let session_header = Some(SessionHeader::new(
            slim_datapath::api::ProtoSessionType::PointToPoint.into(),
            slim_datapath::api::ProtoSessionMessageType::LeaveRequest.into(),
            session_id,
            rand::random::<u32>(),
        ));
        let payload = Some(CommandPayload::new_leave_request_payload(None).as_content());
        let leave_request = Message::new_publish_with_headers(slim_header, session_header, payload);

        moderator
            .on_message(leave_request.clone(), MessageDirection::South)
            .await
            .expect("error sending leave request");

        // check that the request is received by slim on the moderator
        let received_leave_request = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for leave request on moderator slim channel")
            .expect("channel closed")
            .expect("error in leave request");

        assert_eq!(
            received_leave_request.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::LeaveRequest
        );
        assert_eq!(received_leave_request.get_dst(), participant_name_id);

        // send the request to the participant (direction north)
        let mut leave_request_to_participant = received_leave_request.clone();
        leave_request_to_participant
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        participant
            .on_message(leave_request_to_participant, MessageDirection::North)
            .await
            .expect("error processing leave request on participant");

        // get the leave reply on the participant slim
        let leave_reply = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for leave reply on participant slim channel")
            .expect("channel closed")
            .expect("error in leave reply");

        assert_eq!(
            leave_reply.get_session_message_type(),
            slim_datapath::api::ProtoSessionMessageType::LeaveReply
        );
        assert_eq!(leave_reply.get_dst(), moderator_name);

        // get the delete route on the participant slim
        let delete_route = timeout(Duration::from_millis(100), rx_slim_participant.recv())
            .await
            .expect("timeout waiting for delete route on participant slim channel")
            .expect("channel closed")
            .expect("error in delete route");

        assert!(
            delete_route.is_unsubscribe(),
            "delete route should be an unsubscribe message"
        );
        assert_eq!(delete_route.get_dst(), moderator_name);

        // send the leave reply to the moderator on message (direction north)
        let mut leave_reply_to_moderator = leave_reply.clone();
        leave_reply_to_moderator
            .get_slim_header_mut()
            .set_incoming_conn(Some(1));

        moderator
            .on_message(leave_reply_to_moderator, MessageDirection::North)
            .await
            .expect("error processing leave reply on moderator");

        // check that no other messages are generated by the moderator
        let no_more_moderator_final =
            timeout(Duration::from_millis(100), rx_slim_moderator.recv()).await;
        assert!(
            no_more_moderator_final.is_err(),
            "Expected no more messages on moderator slim channel after leave"
        );

        let no_more_participant_final =
            timeout(Duration::from_millis(100), rx_slim_participant.recv()).await;
        assert!(
            no_more_participant_final.is_err(),
            "Expected no more messages on participant slim channel after leave"
        );
    }
}

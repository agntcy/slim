// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::{collections::HashMap, time::Duration};

// Third-party crates
use tokio_util::sync::CancellationToken;
use tracing::debug;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        ApplicationPayload, CommandPayload, Content, ProtoMessage as Message,
        ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader,
    },
    messages::{Name, utils::SlimHeaderFlags},
};

// Local crate
use crate::{
    MessageDirection, SessionError, Transmitter,
    common::SessionMessage,
    controller_sender::ControllerSender,
    session::Session,
    session_builder::{ForController, SessionBuilder},
    session_config::SessionConfig,
    session_moderator::SessionModerator,
    session_participant::SessionParticipant,
    timer_factory::TimerSettings,
    transmitter::SessionTransmitter,
};

// Third-party crates for trait
use tracing::error;

/// Trait for session processors (moderator and participant) to handle control messages
pub(crate) trait SessionProcessor: Send {
    /// Process a control message specific to the processor type
    async fn process_control_message(&mut self, message: Message) -> Result<(), SessionError>;

    /// Get immutable reference to common state
    fn common(&self) -> &SessionControllerCommon;

    /// Get mutable reference to common state
    fn common_mut(&mut self) -> &mut SessionControllerCommon;

    /// Hook for custom handling before processing application messages
    /// Returns the potentially modified message
    fn on_before_app_message(&mut self, message: Message, _direction: MessageDirection) -> Message {
        message
    }

    /// Hook for custom handling of timer failures for control messages
    /// Returns true if the default handling should be skipped
    async fn on_control_timer_failure(&mut self, _message_id: u32) -> bool {
        false
    }
}

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
    pub(crate) id: u32,

    /// local name
    pub(crate) source: Name,

    /// group or remote endpoint name
    pub(crate) destination: Name,

    /// session config
    pub(crate) config: SessionConfig,

    /// controller (participant or moderator)
    pub(crate) controller: SessionControllerImpl<P, V>,

    /// use in drop implementation to close immediately
    /// the session processor loop
    pub(crate) cancellation_token: CancellationToken,
}

impl<P, V> SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Returns a new SessionBuilder for constructing a SessionController
    pub fn builder() -> SessionBuilder<P, V, ForController> {
        SessionBuilder::for_controller()
    }

    /// Internal constructor for the builder to use
    pub(crate) fn from_parts(
        id: u32,
        source: Name,
        destination: Name,
        config: SessionConfig,
        controller: SessionControllerImpl<P, V>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            id,
            source,
            destination,
            config,
            controller,
            cancellation_token,
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

        let slim_header = Some(SlimHeader::new(self.source(), name, "", Some(flags)));
        let session_header = Some(SessionHeader::new(
            self.session_type().into(),
            ProtoSessionMessageType::Msg.into(),
            self.id(),
            rand::random::<u32>(), // this will be changed by the session itself
        ));

        let mut msg = Message::new_publish_with_headers(slim_header, session_header, payload);
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
                let payload = Some(CommandPayload::new_leave_request_payload(None).as_content());
                let msg = Message::new_publish_with_headers(slim_header, session_header, payload);
                self.publish_message(msg).await
            }
            _ => Err(SessionError::Processing("unexpected session type".into())),
        }
    }
}

impl<P, V> Drop for SessionController<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        self.cancellation_token.cancel();
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

pub struct SessionControllerCommon {
    /// session id
    pub(crate) id: u32,

    ///local name
    pub(crate) source: Name,

    /// remote/group name
    /// in case of remote the name the id may be updated after the discovery phase
    pub(crate) destination: Name,

    /// session configuration
    pub(crate) config: SessionConfig,

    /// sender for command messages
    pub(crate) sender: ControllerSender,

    /// directly sent to the slim/app
    pub(crate) tx: SessionTransmitter,

    /// channel used to received messages from the session layer
    /// and timeouts from timers
    pub(crate) rx_from_session_layer: tokio::sync::mpsc::Receiver<SessionMessage>,

    /// channel used to communincate with the session layer.
    pub(crate) tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// the session itself
    pub(crate) session: Session,

    /// cancellation token to exit from the processor loop
    /// in case of session drop
    pub(crate) cancellation_token: CancellationToken,
}

impl SessionControllerCommon {
    const MAX_FANOUT: u32 = 256;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u32,
        source: Name,
        destination: Name,
        config: SessionConfig,
        tx: SessionTransmitter,
        tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,
        rx_controller: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
        cancellation_token: CancellationToken,
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
            config,
            sender: controller_sender,
            tx,
            rx_from_session_layer: rx_controller,
            tx_to_session_layer,
            session,
            cancellation_token,
        }
    }

    /// internal and helper functions
    pub(crate) fn is_command_message(&self, message_type: ProtoSessionMessageType) -> bool {
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

    pub(crate) async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.tx.send_to_slim(Ok(message)).await
    }

    pub(crate) async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
    }

    pub(crate) async fn set_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::new_subscribe(
            &self.source,
            name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );

        self.send_to_slim(route).await
    }

    pub(crate) async fn delete_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::new_unsubscribe(
            &self.source,
            name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );

        self.send_to_slim(route).await
    }

    pub(crate) fn create_control_message(
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

    pub(crate) async fn send_control_message(
        &mut self,
        dst: &Name,
        message_type: ProtoSessionMessageType,
        message_id: u32,
        payload: Content,
        metadata: Option<HashMap<String, String>>,
        broadcast: bool,
    ) -> Result<(), SessionError> {
        let mut msg =
            self.create_control_message(dst, message_type, message_id, payload, broadcast);
        if let Some(m) = metadata {
            msg.set_metadata_map(m);
        }
        self.send_with_timer(msg).await
    }

    /// Common event loop for session processors
    pub(crate) async fn run_processor_loop<P>(mut processor: P)
    where
        P: SessionProcessor + 'static,
    {
        let cancellation_token = processor.common().cancellation_token.clone();
        loop {
            let common = processor.common_mut();
            tokio::select! {
                next = common.rx_from_session_layer.recv() => {
                    match next {
                        Some(message) => match message {
                            SessionMessage::OnMessage { message, direction } => {
                                let is_command = processor.common().is_command_message(message.get_session_message_type());
                                let dir = direction.clone(); // Clone direction to avoid move issues
                                if is_command {
                                    if let Err(e) = processor.process_control_message(message).await {
                                        error!("Error processing control message: {:?}", e);
                                    }
                                } else {
                                    // Allow processor to modify message before sending
                                    let modified_message = processor.on_before_app_message(message, direction);
                                    let common = processor.common_mut();
                                    if let Err(e) = common.session.on_message(SessionMessage::OnMessage { message: modified_message, direction: dir }).await {
                                        error!("Error sending message to the session: {:?}", e);
                                    }
                                }
                            }
                            SessionMessage::TimerTimeout { message_id, message_type, name, timeouts } => {
                                let is_command = processor.common().is_command_message(message_type);
                                if is_command {
                                    let common = processor.common_mut();
                                    if let Err(e) = common.sender.on_timer_timeout(message_id).await {
                                        error!("Error processing timeout for control message: {:?}", e);
                                    }
                                } else {
                                    let common = processor.common_mut();
                                    if let Err(e) = common.session.on_message(SessionMessage::TimerTimeout { message_id, message_type, name, timeouts }).await {
                                        error!("Error processing timeout in the session: {:?}", e);
                                    }
                                }
                            }
                            SessionMessage::TimerFailure { message_id, message_type, name, timeouts } => {
                                let is_command = processor.common().is_command_message(message_type);
                                if is_command {
                                    {
                                        let common = processor.common_mut();
                                        common.sender.on_timer_failure(message_id).await;
                                    }

                                    // Allow processor to do custom handling
                                    let _ = processor.on_control_timer_failure(message_id).await;
                                } else {
                                    let common = processor.common_mut();
                                    if let Err(e) = common.session.on_message(SessionMessage::TimerFailure { message_id, message_type, name, timeouts }).await {
                                        error!("Error processing timer failure in the session: {:?}", e);
                                    }
                                }
                            }
                            SessionMessage::StartDrain { grace_period_ms: _ } => {
                                debug!("StartDrain message received (not implemented)");
                            }
                            _ => {
                                debug!("Unexpected message type");
                            }
                        }
                        None => {
                            let session_id = processor.common().id;
                            debug!("session controller close channel {}", session_id);
                            break;
                        }
                    }
                }
                _ = cancellation_token.cancelled() => {
                    debug!("cancellation token signaled on session processor, close it");
                    let common = processor.common_mut();
                    common.session.close();
                    common.sender.close();
                    break;
                }
            }
        }
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
        let cancellation_token = CancellationToken::new();

        // create a SessionModerator
        let (tx_slim_moderator, mut rx_slim_moderator) = tokio::sync::mpsc::channel(10);
        let (tx_app_moderator, _rx_app_moderator) = tokio::sync::mpsc::unbounded_channel();
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

        let moderator = SessionBuilder::for_moderator()
            .with_id(session_id)
            .with_source(moderator_name.clone())
            .with_destination(participant_name.clone())
            .with_config(moderator_config)
            .with_identity_provider(SharedSecret::new("moderator", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("moderator", SHARED_SECRET))
            .with_storage_path(storage_path_moderator.clone())
            .with_tx(tx_moderator.clone())
            .with_tx_to_session_layer(tx_session_layer_moderator)
            .with_cancellation_token(cancellation_token.clone())
            .ready()
            .expect("failed to validate builder")
            .build()
            .await;

        // create a SessionParticipant
        let (tx_slim_participant, mut rx_slim_participant) = tokio::sync::mpsc::channel(10);
        let (tx_app_participant, mut rx_app_participant) = tokio::sync::mpsc::unbounded_channel();
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

        let participant = SessionBuilder::for_participant()
            .with_id(session_id)
            .with_source(participant_name_id.clone())
            .with_destination(moderator_name.clone())
            .with_config(participant_config)
            .with_identity_provider(SharedSecret::new("participant", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("participant", SHARED_SECRET))
            .with_storage_path(storage_path_participant.clone())
            .with_tx(tx_participant.clone())
            .with_tx_to_session_layer(tx_session_layer_participant)
            .with_cancellation_token(cancellation_token.clone())
            .ready()
            .expect("failed to validate builder")
            .build()
            .await;

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
            .unwrap()
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

        // expect a remove route for the participant name
        let delete_route = timeout(Duration::from_millis(100), rx_slim_moderator.recv())
            .await
            .expect("timeout waiting for delete route on participant slim channel")
            .expect("channel closed")
            .expect("error in delete route");

        assert!(
            delete_route.is_unsubscribe(),
            "delete route should be an unsubscribe message"
        );
        assert_eq!(delete_route.get_dst(), participant_name_id);

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

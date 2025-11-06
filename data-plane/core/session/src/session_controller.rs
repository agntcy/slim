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
    session_builder::{ForController, SessionBuilder},
    session_config::SessionConfig,
    session_settings::SessionSettings,
    timer_factory::TimerSettings,
    traits::MessageHandler,
};

pub struct SessionController {
    /// session id
    pub(crate) id: u32,

    /// local name
    pub(crate) source: Name,

    /// group or remote endpoint name
    pub(crate) destination: Name,

    /// session config
    pub(crate) config: SessionConfig,

    /// channel to send messages to the processing loop
    tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,

    /// use in drop implementation to close immediately
    /// the session processor loop
    pub(crate) cancellation_token: CancellationToken,
}

impl SessionController {
    /// Returns a new SessionBuilder for constructing a SessionController
    pub fn builder<P, V>() -> SessionBuilder<P, V, ForController>
    where
        P: TokenProvider + Send + Sync + Clone + 'static,
        V: Verifier + Send + Sync + Clone + 'static,
    {
        SessionBuilder::for_controller()
    }

    /// Internal constructor for the builder to use
    pub(crate) fn from_parts<I>(
        id: u32,
        source: Name,
        destination: Name,
        config: SessionConfig,
        tx: tokio::sync::mpsc::Sender<SessionMessage>,
        rx: tokio::sync::mpsc::Receiver<SessionMessage>,
        inner: I,
    ) -> Self
    where
        I: MessageHandler + Send + Sync + 'static,
    {
        // Spawn the processing loop
        let cancellation_token = CancellationToken::new();
        tokio::spawn(Self::processing_loop(inner, rx, cancellation_token.clone()));

        Self {
            id,
            source,
            destination,
            config,
            tx_controller: tx,
            cancellation_token,
        }
    }

    /// Internal processing loop that handles messages with mutable access
    async fn processing_loop(
        mut inner: impl MessageHandler + 'static,
        mut rx: tokio::sync::mpsc::Receiver<SessionMessage>,
        cancellation_token: CancellationToken,
    ) {
        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    debug!("Processing loop cancelled");
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(session_message) => {
                            if let Err(e) = inner.on_message(session_message).await {
                                tracing::error!(error=%e, "Error processing message in session");
                            }
                        }
                        None => {
                            debug!("Controller channel closed, exiting processing loop");
                            break;
                        }
                    }
                }
            }
        }

        // Perform shutdown
        if let Err(e) = inner.on_shutdown().await {
            tracing::error!(error=%e, "Error during shutdown of session");
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
        self.config.initiator
    }

    /// Send a message to the controller for processing
    pub async fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        self.tx_controller
            .send(SessionMessage::OnMessage { message, direction })
            .await
            .map_err(|e| {
                SessionError::Processing(format!("Failed to send message to controller: {}", e))
            })
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

    /// Creates a discovery request message with minimum required information
    fn create_discovery_request(&self, destination: &Name) -> Message {
        Message::new_discovery_request(self.source(), destination, self.session_type(), self.id())
    }

    pub async fn invite_participant(&self, destination: &Name) -> Result<(), SessionError> {
        match self.session_type() {
            ProtoSessionType::PointToPoint => Err(SessionError::Processing(
                "cannot invite participant to point-to-point session".into(),
            )),
            ProtoSessionType::Multicast => {
                if !self.is_initiator() {
                    return Err(SessionError::Processing(
                        "cannot invite participant to this session session".into(),
                    ));
                }
                let msg = self.create_discovery_request(destination);
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

impl Drop for SessionController {
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

pub(crate) struct SessionControllerCommon<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// common session fields
    pub(crate) settings: SessionSettings<P, V>,

    /// sender for command messages
    pub(crate) sender: ControllerSender,
}

impl<P, V> SessionControllerCommon<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    pub(crate) fn new(settings: SessionSettings<P, V>) -> Self {
        // timers settings for the controller
        let controller_timer_settings =
            TimerSettings::constant(Duration::from_secs(1)).with_max_retries(10);

        // create the controller sender
        let controller_sender = ControllerSender::new(
            controller_timer_settings,
            settings.source.clone(),
            // send messages to slim/app
            settings.tx.clone(),
            // send signal to the controller
            settings.tx_session.clone(),
        );

        SessionControllerCommon {
            settings,
            sender: controller_sender,
        }
    }

    /// internal and helper functions
    pub(crate) async fn send_to_slim(&self, message: Message) -> Result<(), SessionError> {
        self.settings.tx.send_to_slim(Ok(message)).await
    }

    pub(crate) async fn send_with_timer(&mut self, message: Message) -> Result<(), SessionError> {
        self.sender.on_message(&message).await
    }

    pub(crate) async fn set_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::new_subscribe(
            &self.settings.source,
            name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );

        self.send_to_slim(route).await
    }

    pub(crate) async fn delete_route(&self, name: &Name, conn: u64) -> Result<(), SessionError> {
        let route = Message::new_unsubscribe(
            &self.settings.source,
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
        Message::new_control_message(
            &self.settings.source,
            dst,
            self.settings.config.session_type,
            message_type,
            self.settings.id,
            message_id,
            payload,
            broadcast,
        )
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

        let moderator = SessionController::builder()
            .with_id(session_id)
            .with_source(moderator_name.clone())
            .with_destination(participant_name.clone())
            .with_config(moderator_config)
            .with_identity_provider(SharedSecret::new("moderator", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("moderator", SHARED_SECRET))
            .with_storage_path(storage_path_moderator.clone())
            .with_tx(tx_moderator.clone())
            .with_tx_to_session_layer(tx_session_layer_moderator)
            .ready()
            .expect("failed to validate builder")
            .build()
            .await
            .unwrap();

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

        let participant = SessionController::builder()
            .with_id(session_id)
            .with_source(participant_name_id.clone())
            .with_destination(moderator_name.clone())
            .with_config(participant_config)
            .with_identity_provider(SharedSecret::new("participant", SHARED_SECRET))
            .with_identity_verifier(SharedSecret::new("participant", SHARED_SECRET))
            .with_storage_path(storage_path_participant.clone())
            .with_tx(tx_participant.clone())
            .with_tx_to_session_layer(tx_session_layer_participant)
            .ready()
            .expect("failed to validate builder")
            .build()
            .await
            .unwrap();

        // Discovery request message is automatically sent by the moderator on creation
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
            "Expected no more messages on moderator slim channel, received {:?}",
            no_more_moderator
                .ok()
                .and_then(|opt| opt)
                .and_then(|res| res.ok())
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
            .on_message(app_message, MessageDirection::South)
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
            .on_message(leave_request, MessageDirection::South)
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

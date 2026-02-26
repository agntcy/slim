// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{
        CommandPayload, ParticipantSettings, ProtoMessage as Message, ProtoSessionMessageType,
        ProtoSessionType,
    },
    messages::{
        Name,
        utils::{LEAVING_SESSION, TRUE_VAL},
    },
};

use slim_mls::mls::Mls;
use tracing::debug;

use crate::{
    common::SessionMessage,
    errors::SessionError,
    mls_state::MlsState,
    session_controller::SessionControllerCommon,
    session_settings::SessionSettings,
    traits::{MessageHandler, ProcessingState},
};

pub struct SessionParticipant<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// list of participants with their settings
    group_list: HashMap<Name, ParticipantSettings>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// common session state
    common: SessionControllerCommon<P, V>,

    /// connection id from where the remote messages are received
    conn_id: Option<u64>,

    subscribed: bool,

    /// inner layer
    inner: I,
}

impl<P, V, I> SessionParticipant<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    pub(crate) fn new(inner: I, settings: SessionSettings<P, V>) -> Self {
        let common = SessionControllerCommon::new(settings);

        SessionParticipant {
            moderator_name: None,
            group_list: HashMap::new(),
            mls_state: None,
            common,
            conn_id: None,
            subscribed: false,
            inner,
        }
    }
}

/// Implementation of MessageHandler trait for SessionParticipant
/// This allows the participant to be used as a layer in the generic layer system
#[async_trait]
impl<P, V, I> MessageHandler for SessionParticipant<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    async fn init(&mut self) -> Result<(), SessionError> {
        // Initialize MLS
        self.mls_state = if self.common.settings.config.mls_enabled {
            let mls_state = MlsState::new(Mls::new(
                self.common.settings.identity_provider.clone(),
                self.common.settings.identity_verifier.clone(),
            ))
            .await
            .expect("failed to create MLS state");

            Some(mls_state)
        } else {
            None
        };

        Ok(())
    }

    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        match message {
            SessionMessage::OnMessage {
                mut message,
                direction,
                ack_tx,
            } => {
                if message.get_session_message_type().is_command_message() {
                    debug!(
                        message = ?message.get_session_message_type(),
                        source = %message.get_source(),
                        "received message",
                    );
                    self.process_control_message(message).await
                } else {
                    // Apply MLS encryption/decryption if enabled
                    if let Some(mls_state) = &mut self.mls_state {
                        mls_state.process_message(&mut message, direction).await?;
                    }

                    self.inner
                        .on_message(SessionMessage::OnMessage {
                            message,
                            direction,
                            ack_tx,
                        })
                        .await
                }
            }
            SessionMessage::MessageError { error } => self.handle_message_error(error).await,
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    self.common
                        .sender
                        .on_timer_timeout(message_id, message_type)
                        .await
                } else {
                    self.inner
                        .on_message(SessionMessage::TimerTimeout {
                            message_id,
                            message_type,
                            name,
                            timeouts,
                        })
                        .await
                }
            }
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if message_type.is_command_message() {
                    self.common.sender.on_failure(message_id, message_type);
                    Ok(())
                } else {
                    self.inner
                        .on_message(SessionMessage::TimerFailure {
                            message_id,
                            message_type,
                            name,
                            timeouts,
                        })
                        .await
                }
            }
            SessionMessage::StartDrain {
                grace_period: duration,
            } => {
                debug!("received drain signal");
                // create a leave request message for the participant that
                // got disconnected and add the metadata to the message
                let p = CommandPayload::builder().leave_request(None).as_content();
                if let Some(moderator) = &self.moderator_name {
                    let mut msg = self.common.create_control_message(
                        moderator,
                        ProtoSessionMessageType::LeaveRequest,
                        rand::random::<u32>(),
                        p,
                        false,
                    )?;
                    debug!("start drain and notify the moderator");
                    msg.insert_metadata(LEAVING_SESSION.to_string(), TRUE_VAL.to_string());

                    // remove the route and the subscription for the group
                    // to avoid to get broadcast messages from the moderator
                    self.disconnect_from_group().await?;

                    self.common.sender.on_message(&msg).await?;
                }

                // propagate draining state
                self.common.processing_state = ProcessingState::Draining;
                self.inner
                    .on_message(SessionMessage::StartDrain {
                        grace_period: duration,
                    })
                    .await?;
                self.common.sender.start_drain();

                Ok(())
            }
            SessionMessage::ParticipantDisconnected { name: _ } => {
                debug!("The moderator is not anymore connected to the current session, close it",);

                // start drain
                self.common.processing_state = ProcessingState::Draining;
                self.inner
                    .on_message(SessionMessage::StartDrain {
                        grace_period: Duration::from_secs(1), // not used
                    })
                    .await?;
                self.common.sender.start_drain();

                Ok(())
            }
            _ => Err(SessionError::SessionMessageInternalUnexpected(Box::new(
                message,
            ))),
        }
    }

    async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        self.inner.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &Name) {
        self.inner.remove_endpoint(endpoint);
    }

    fn needs_drain(&self) -> bool {
        !self.common.sender.drain_completed() || self.inner.needs_drain()
    }

    fn processing_state(&self) -> ProcessingState {
        self.common.processing_state
    }

    fn participants_list(&self) -> Vec<Name> {
        self.group_list.keys().cloned().collect()
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Participant-specific cleanup
        self.subscribed = false;
        self.common.sender.close();

        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await
    }
}

impl<P, V, I> SessionParticipant<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// Helper method to handle MessageError
    /// Extracts context from the error and routes to appropriate handler
    async fn handle_message_error(&mut self, error: SessionError) -> Result<(), SessionError> {
        let Some(session_ctx) = error.session_context() else {
            tracing::warn!("Received MessageError without session context");
            return self
                .inner
                .on_message(SessionMessage::MessageError { error })
                .await;
        };

        if error.is_command_message_error() {
            // Handle command message failure
            self.common.sender.on_failure(
                session_ctx.message_id,
                session_ctx.get_session_message_type(),
            );
            Ok(())
        } else {
            // Pass non-command errors to inner handler
            self.inner
                .on_message(SessionMessage::MessageError { error })
                .await
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
            ProtoSessionMessageType::LeaveRequest | ProtoSessionMessageType::GroupClose => {
                self.on_leave_request(message).await
            }
            ProtoSessionMessageType::Ping => self.on_ping(message).await,
            ProtoSessionMessageType::LeaveReply => {
                // this message is received when the moderator ack the
                // reception of the leave request sent on Drain start
                // if the participant in not on drain state drop the message
                if self.common.processing_state == ProcessingState::Draining {
                    self.common.sender.on_message(&message).await?;
                }
                Ok(())
            }
            ProtoSessionMessageType::GroupProposal
            | ProtoSessionMessageType::GroupAck
            | ProtoSessionMessageType::GroupNack => todo!(),
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinReply => {
                debug!(
                    control_message_type = ?message.get_session_message_type(),
                    "Unexpected control message type",
                );
                Ok(())
            }
            _ => {
                debug!(
                    message_type = ?message.get_session_message_type(),
                    "Unexpected message type",
                );
                Ok(())
            }
        }
    }

    async fn on_join_request(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received join request",
        );
        let source = msg.get_source();
        self.moderator_name = Some(source.clone());

        self.common
            .add_route(&source, msg.get_incoming_conn())
            .await?;

        let key_package = if let Some(mls_state) = &mut self.mls_state {
            debug!("mls enabled, create the package key");
            let key = mls_state.generate_key_package().await?;
            Some(key)
        } else {
            None
        };

        let content = CommandPayload::builder()
            .join_reply(
                self.common.settings.direction.to_participant_settings(),
                key_package,
            )
            .as_content();

        debug!("send join reply message");
        let reply = self.common.create_control_message(
            &source,
            ProtoSessionMessageType::JoinReply,
            msg.get_id(),
            content,
            false,
        )?;

        self.common.send_to_slim(reply).await
    }

    async fn on_welcome(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received welcome message",
        );

        if let Some(mls_state) = &mut self.mls_state {
            mls_state.process_welcome_message(&msg).await?;
        }

        self.join(&msg).await?;

        let welcome_payload = msg
            .get_payload()
            .unwrap()
            .as_command_payload()?
            .as_welcome_payload()?;
        let participant_list = &welcome_payload.participants;
        let mut participant_settings_list = &welcome_payload.settings;

        // Empty settings vec means old moderator (pre-settings version); fallback to default
        // Non-empty but wrong length means a bug in the sender
        let default_settings;
        if participant_settings_list.is_empty() {
            debug!("welcome message has empty participant settings, using defaults");
            default_settings = vec![ParticipantSettings::default(); participant_list.len()];
            participant_settings_list = &default_settings;
        }

        if participant_settings_list.len() != participant_list.len() {
            return Err(SessionError::InvalidParticipantSettingsLength);
        }

        for (i, n) in participant_list.iter().enumerate() {
            let name = Name::from(n);
            let settings = participant_settings_list
                .get(i)
                .ok_or(SessionError::ParticipantSettingsNotFound(name.clone()))?;
            self.group_list.insert(name.clone(), *settings);

            if name != self.common.settings.source {
                debug!(name = %msg.get_source(), "add endpoint to the session");
                // add a route to the new endpoint, this is needed in case of message retransmission
                // skip the moderator as the route is already added in on_join_request
                if self.moderator_name.as_ref() != Some(&name) {
                    self.common
                        .add_route(&name, msg.get_incoming_conn())
                        .await?;
                }
                self.add_endpoint(&name).await?;
            }
        }

        let ack = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;

        self.common.send_to_slim(ack).await
    }

    async fn on_group_update_message(
        &mut self,
        msg: Message,
        add: bool,
    ) -> Result<(), SessionError> {
        debug!(
            name = %self.common.settings.source,
            id = %msg.get_id(),
            "received update",
        );

        if let Some(mls_state) = &mut self.mls_state {
            debug!("process mls control update");
            let ret = mls_state
                .process_control_message(msg.clone(), &self.common.settings.source)
                .await?;

            if !ret {
                debug!(
                    id = %msg.get_id(),
                    "Message already processed, drop it",
                );
                return Ok(());
            }
        }

        if add {
            let p = msg
                .get_payload()
                .unwrap()
                .as_command_payload()?
                .as_group_add_payload()?;
            if let Some(ref new_participant) = p.new_participant {
                let name = Name::from(new_participant);
                // None means old moderator (pre-settings version); use default settings.
                let settings = p.new_participant_settings.unwrap_or_default();

                self.group_list.insert(name.clone(), settings);

                debug!(name  = %msg.get_source(), "add endpoint to session");
                // add a route to the new endpoint, this is needed in case of message retransmission
                self.common
                    .add_route(&name, msg.get_incoming_conn())
                    .await?;
                self.add_endpoint(&name).await?;
            }
        } else {
            let p = msg
                .get_payload()
                .unwrap()
                .as_command_payload()?
                .as_group_remove_payload()?;
            if let Some(ref removed_participant) = p.removed_participant {
                let name = Name::from(removed_participant);
                self.group_list.remove(&name);

                debug!(name = %msg.get_source(), "remove endpoint from session");
                // remove a route to the endpoint
                self.common
                    .delete_route(&name, msg.get_incoming_conn())
                    .await?;
                self.inner.remove_endpoint(&name);
            }
        }

        let msg = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::GroupAck,
            msg.get_id(),
            CommandPayload::builder().group_ack().as_content(),
            false,
        )?;
        self.common.send_to_slim(msg).await
    }

    async fn on_leave_request(&mut self, msg: Message) -> Result<(), SessionError> {
        debug!("close session");
        self.common.processing_state = ProcessingState::Draining;
        self.inner
            .on_message(SessionMessage::StartDrain {
                grace_period: Duration::from_secs(60), // not used in session
            })
            .await?;
        self.common.sender.start_drain();

        let reply = if msg.get_session_message_type() == ProtoSessionMessageType::LeaveRequest {
            self.common.create_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::LeaveReply,
                msg.get_id(),
                CommandPayload::builder().leave_reply().as_content(),
                false,
            )?
        } else {
            self.common.create_control_message(
                &msg.get_source(),
                ProtoSessionMessageType::GroupAck,
                msg.get_id(),
                CommandPayload::builder().group_ack().as_content(),
                false,
            )?
        };

        self.common.send_to_slim(reply).await?;

        self.disconnect_from_group().await?;
        self.disconnect_from_moderator().await?;

        self.common
            .settings
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.settings.id,
            }))
            .await
            .map_err(|_e| SessionError::SessionDeleteMessageSendFailed)
    }

    async fn on_ping(&mut self, mut msg: Message) -> Result<(), SessionError> {
        debug!("received ping message, reply");
        // send ping to the local sender to register the reception
        self.common.sender.on_message(&msg).await?;

        // reply to the ping
        let header = msg.get_slim_header_mut();
        let src = header.get_source();
        header.set_source(&self.common.settings.source);
        header.set_destination(&src);
        self.common.send_to_slim(msg).await
    }

    async fn join(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        self.conn_id = Some(msg.get_incoming_conn());

        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        self.common
            .add_route(&self.common.settings.destination, msg.get_incoming_conn())
            .await?;
        self.common
            .add_subscription(&self.common.settings.destination, msg.get_incoming_conn())
            .await
    }

    async fn disconnect_from_group(&self) -> Result<(), SessionError> {
        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        if let Some(conn_id) = self.conn_id {
            self.common
                .delete_route(&self.common.settings.destination, conn_id)
                .await?;
            self.common
                .delete_subscription(&self.common.settings.destination, conn_id)
                .await?;
        }

        // remove also all the routes to the other participants except the moderator
        // it will be removed in disconnect_from_moderator
        for (n, _s) in self.group_list.iter() {
            if self.moderator_name.as_ref() != Some(n) {
                self.common.delete_route(n, self.conn_id.unwrap()).await?;
            }
        }

        Ok(())
    }

    async fn disconnect_from_moderator(&self) -> Result<(), SessionError> {
        if let Some(conn_id) = self.conn_id {
            self.common
                .delete_route(self.moderator_name.as_ref().unwrap(), conn_id)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Direction;
    use crate::session_config::SessionConfig;
    use crate::session_settings::SessionSettings;
    use crate::test_utils::{MockInnerHandler, MockTokenProvider, MockVerifier};
    use slim_datapath::Status;
    use slim_datapath::api::{CommandPayload, ProtoSessionType};
    use slim_datapath::messages::Name;
    use tokio::sync::mpsc;

    // --- Test Helpers -----------------------------------------------------------------------

    fn make_name(parts: &[&str; 3]) -> Name {
        Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    fn setup_participant(
        session_type: ProtoSessionType,
    ) -> (
        SessionParticipant<MockTokenProvider, MockVerifier, MockInnerHandler>,
        mpsc::Receiver<Result<Message, Status>>,
        mpsc::Receiver<Result<SessionMessage, SessionError>>,
    ) {
        let source = make_name(&["local", "participant", "v1"]).with_id(100);
        let destination = make_name(&["channel", "name", "v1"]).with_id(200);

        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::unbounded_channel();
        let (tx_session, _rx_session) = mpsc::channel(16);
        let tx = crate::transmitter::SessionTransmitter::new(tx_slim, tx_app);
        let (tx_session_layer, rx_session_layer) = mpsc::channel(16);

        let config = SessionConfig {
            session_type,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: false,
            metadata: Default::default(),
        };

        let settings = SessionSettings {
            id: 1,
            source,
            destination,
            config,
            direction: Direction::Bidirectional,
            tx,
            tx_session,
            tx_to_session_layer: tx_session_layer,
            identity_provider,
            identity_verifier,
            graceful_shutdown_timeout: None,
        };

        let inner = MockInnerHandler::new();
        let participant = SessionParticipant::new(inner, settings);

        (participant, rx_slim, rx_session_layer)
    }

    #[tokio::test]
    async fn test_participant_new() {
        let (participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);

        assert!(participant.moderator_name.is_none());
        assert!(participant.group_list.is_empty());
        assert!(participant.mls_state.is_none());
        assert!(!participant.subscribed);
    }

    #[tokio::test]
    async fn test_participant_init() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);

        let result = participant.init().await;
        assert!(result.is_ok());
        assert!(participant.mls_state.is_none()); // MLS is disabled in test setup
    }

    #[tokio::test]
    async fn test_participant_on_join_request() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let destination = participant.common.settings.source.clone();

        let join_msg = Message::builder()
            .source(moderator.clone())
            .destination(destination)
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        false,
                        Some(3),
                        Some(std::time::Duration::from_secs(1)),
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_join_request(join_msg).await;
        assert!(result.is_ok());

        // Should have set moderator name
        assert_eq!(participant.moderator_name, Some(moderator));

        // Should have sent messages (route + join reply)
        let mut message_count = 0;
        while let Ok(Ok(_msg)) = rx_slim.try_recv() {
            message_count += 1;
        }
        assert!(
            message_count > 0,
            "Should have sent messages including join reply"
        );
    }

    #[tokio::test]
    async fn test_participant_on_welcome_multicast() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let participant1 = make_name(&["participant1", "app", "v1"]).with_id(401);
        let participant2 = make_name(&["participant2", "app", "v1"]).with_id(402);
        let settings = ParticipantSettings::default();

        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .group_welcome(
                        vec![participant1.clone(), participant2.clone()],
                        vec![settings, settings],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_welcome(welcome_msg).await;
        assert!(result.is_ok());

        // Should have subscribed
        assert!(participant.subscribed);

        // Should have added participants to group list
        assert_eq!(participant.group_list.len(), 2);

        // Should have added endpoints (excluding self)
        assert_eq!(participant.inner.get_endpoints_added_count().await, 2);

        // Should have sent messages (subscribe + routes + group ack)
        let mut message_count = 0;
        while let Ok(Ok(_msg)) = rx_slim.try_recv() {
            message_count += 1;
        }
        assert!(
            message_count > 0,
            "Should have sent messages including group ack"
        );
    }

    #[tokio::test]
    async fn test_participant_on_group_add_message() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let new_participant = make_name(&["new_participant", "app", "v1"]).with_id(500);

        let add_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupAdd)
            .session_id(1)
            .message_id(300)
            .payload(
                CommandPayload::builder()
                    .group_add(
                        new_participant.clone(),
                        ParticipantSettings::default(),
                        vec![],
                        vec![],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_group_update_message(add_msg, true).await;
        assert!(result.is_ok());

        // Should have added participant to group list
        assert!(participant.group_list.contains_key(&new_participant));

        // Should have added endpoint
        assert_eq!(participant.inner.get_endpoints_added_count().await, 1);

        // Should have sent group ack
        let ack_msg = rx_slim.try_recv();
        assert!(ack_msg.is_ok());
    }

    #[tokio::test]
    async fn test_participant_on_group_remove_message() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let removed_participant = make_name(&["removed", "app", "v1"]).with_id(500);
        participant
            .group_list
            .insert(removed_participant.clone(), ParticipantSettings::default());

        let remove_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.destination.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupRemove)
            .session_id(1)
            .message_id(400)
            .payload(
                CommandPayload::builder()
                    .group_remove(removed_participant.clone(), vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_group_update_message(remove_msg, false).await;
        assert!(result.is_ok());

        // Should have removed participant from group list
        assert!(!participant.group_list.contains_key(&removed_participant));

        // Should have removed endpoint
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(participant.inner.get_endpoints_removed_count().await, 1);

        // Should have sent group ack
        let ack_msg = rx_slim.try_recv();
        assert!(ack_msg.is_ok());
    }

    #[tokio::test]
    async fn test_participant_on_leave_request() {
        let (mut participant, mut rx_slim, mut rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let leave_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::LeaveRequest)
            .session_id(1)
            .message_id(500)
            .payload(CommandPayload::builder().leave_request(None).as_content())
            .build_publish()
            .unwrap();

        let result = participant.on_leave_request(leave_msg).await;
        assert!(result.is_ok());

        // Should have sent leave reply
        let reply_msg = rx_slim.try_recv();
        assert!(reply_msg.is_ok());
        let msg = reply_msg.unwrap().unwrap();
        assert_eq!(
            msg.get_session_header().session_message_type(),
            ProtoSessionMessageType::LeaveReply
        );

        // Should have sent delete session message
        let delete_msg = rx_session_layer.try_recv();
        assert!(delete_msg.is_ok());
        if let Ok(Ok(SessionMessage::DeleteSession { session_id })) = delete_msg {
            assert_eq!(session_id, 1);
        } else {
            panic!("Expected DeleteSession message");
        }
    }

    #[tokio::test]
    async fn test_participant_join_multicast() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![], vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.join(&welcome_msg).await;
        assert!(result.is_ok());
        assert!(participant.subscribed);

        // Should have sent subscribe message
        let sub_msg = rx_slim.try_recv();
        assert!(sub_msg.is_ok());
    }

    #[tokio::test]
    async fn test_participant_join_point_to_point() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::PointToPoint);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::JoinRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .join_request(
                        false,
                        Some(3),
                        Some(std::time::Duration::from_secs(1)),
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.join(&msg).await;
        assert!(result.is_ok());
        assert!(participant.subscribed);
        // P2P doesn't send subscribe message
    }

    #[tokio::test]
    async fn test_participant_join_idempotent() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        let msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![], vec![], None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        // First join
        participant.join(&msg).await.unwrap();

        // Drain all messages from first join (routes + subscribe)
        let mut message_count = 0;
        while rx_slim.try_recv().is_ok() {
            message_count += 1;
        }
        assert!(message_count > 0, "First join should send messages");

        // Second join should do nothing
        participant.join(&msg).await.unwrap();
        let second_sub = rx_slim.try_recv();
        assert!(
            second_sub.is_err(),
            "Second join should not send any messages"
        );
    }

    #[tokio::test]
    async fn test_participant_application_message_forwarding() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let source = participant.common.settings.source.clone();
        let destination = participant.common.settings.destination.clone();

        let app_msg = Message::builder()
            .source(source)
            .destination(destination)
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::Msg)
            .session_id(1)
            .message_id(100)
            .application_payload("application/octet-stream", vec![1, 2, 3, 4])
            .build_publish()
            .unwrap();

        let result = participant
            .on_message(SessionMessage::OnMessage {
                message: app_msg,
                direction: crate::MessageDirection::South,
                ack_tx: None,
            })
            .await;

        assert!(result.is_ok());

        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_timer_timeout_control_message() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::JoinRequest,
                name: None,
                timeouts: 1,
            })
            .await;

        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_participant_timer_timeout_app_message() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerTimeout {
                message_id: 100,
                message_type: ProtoSessionMessageType::Msg,
                name: None,
                timeouts: 1,
            })
            .await;

        assert!(result.is_ok());
        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_timer_failure_control_message() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerFailure {
                message_id: 100,
                message_type: ProtoSessionMessageType::JoinRequest,
                name: None,
                timeouts: 3,
            })
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_participant_timer_failure_app_message() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let result = participant
            .on_message(SessionMessage::TimerFailure {
                message_id: 100,
                message_type: ProtoSessionMessageType::Msg,
                name: None,
                timeouts: 3,
            })
            .await;

        assert!(result.is_ok());
        // Should have forwarded to inner handler
        assert_eq!(participant.inner.get_messages_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_add_and_remove_endpoint() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let endpoint = make_name(&["endpoint", "app", "v1"]).with_id(400);

        // Add endpoint
        let result = participant.add_endpoint(&endpoint).await;
        assert!(result.is_ok());
        assert_eq!(participant.inner.get_endpoints_added_count().await, 1);

        // Remove endpoint
        participant.remove_endpoint(&endpoint);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(participant.inner.get_endpoints_removed_count().await, 1);
    }

    #[tokio::test]
    async fn test_participant_on_shutdown() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;

        let result = participant.on_shutdown().await;
        assert!(result.is_ok());
        assert!(!participant.subscribed);
    }

    #[tokio::test]
    async fn test_participant_unexpected_control_messages() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        // Test DiscoveryRequest (unexpected for participant)
        let discovery_msg = Message::builder()
            .source(make_name(&["someone", "app", "v1"]).with_id(300))
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(1)
            .message_id(100)
            .payload(
                CommandPayload::builder()
                    .discovery_request(None)
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.process_control_message(discovery_msg).await;
        assert!(result.is_ok()); // Should handle gracefully
    }

    #[tokio::test]
    async fn test_participant_leave_multicast_unsubscribes() {
        let (mut participant, mut rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();
        participant.subscribed = true;
        participant.conn_id = Some(12345); // Set conn_id so disconnect_from_group sends messages

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        // Manually call leave to test unsubscribe
        let result = participant.disconnect_from_group().await;
        assert!(result.is_ok());
        let result = participant.disconnect_from_moderator().await;
        assert!(result.is_ok());

        // disconnect_from_group sends: delete_route + unsubscribe
        // disconnect_from_moderator sends: delete_route
        let mut message_count = 0;
        while let Ok(Ok(_msg)) = rx_slim.try_recv() {
            message_count += 1;
        }
        assert!(
            message_count > 0,
            "Should have sent unsubscribe and route deletion messages"
        );
    }

    #[tokio::test]
    async fn test_welcome_stores_asymmetric_participant_settings() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let sender = make_name(&["sender", "app", "v1"]).with_id(401);
        let receiver = make_name(&["receiver", "app", "v1"]).with_id(402);

        let sender_settings = ParticipantSettings {
            sends_data: Some(true),
            receives_data: Some(false),
        };
        let receiver_settings = ParticipantSettings {
            sends_data: Some(false),
            receives_data: Some(true),
        };

        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(200)
            .payload(
                CommandPayload::builder()
                    .group_welcome(
                        vec![sender.clone(), receiver.clone()],
                        vec![sender_settings, receiver_settings],
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_welcome(welcome_msg).await;
        assert!(result.is_ok());

        let stored_sender = participant.group_list.get(&sender).unwrap();
        assert!(stored_sender.is_sender());
        assert!(!stored_sender.is_receiver());
        assert!(!stored_sender.is_legacy());

        let stored_receiver = participant.group_list.get(&receiver).unwrap();
        assert!(!stored_receiver.is_sender());
        assert!(stored_receiver.is_receiver());
        assert!(!stored_receiver.is_legacy());
    }

    #[tokio::test]
    async fn test_welcome_empty_settings_uses_legacy_defaults() {
        // Old moderator sends group_welcome with no settings vec → all participants get legacy defaults
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let p1 = make_name(&["p1", "app", "v1"]).with_id(401);

        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(201)
            .payload(
                CommandPayload::builder()
                    .group_welcome(vec![p1.clone()], vec![], None) // empty settings = old moderator
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_welcome(welcome_msg).await;
        assert!(result.is_ok());

        let stored = participant.group_list.get(&p1).unwrap();
        assert!(stored.is_legacy()); // fallback → legacy default
        assert!(stored.is_sender()); // legacy defaults to true
        assert!(stored.is_receiver()); // legacy defaults to true
    }

    #[tokio::test]
    async fn test_welcome_mismatched_settings_length_returns_error() {
        let (mut participant, _rx_slim, _rx_session_layer) =
            setup_participant(ProtoSessionType::Multicast);
        participant.init().await.unwrap();

        let moderator = make_name(&["moderator", "app", "v1"]).with_id(300);
        participant.moderator_name = Some(moderator.clone());

        let p1 = make_name(&["p1", "app", "v1"]).with_id(401);
        let p2 = make_name(&["p2", "app", "v1"]).with_id(402);

        let welcome_msg = Message::builder()
            .source(moderator.clone())
            .destination(participant.common.settings.source.clone())
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(202)
            .payload(
                CommandPayload::builder()
                    .group_welcome(
                        vec![p1.clone(), p2.clone()],
                        vec![ParticipantSettings::default()], // wrong: 1 setting for 2 participants
                        None,
                    )
                    .as_content(),
            )
            .build_publish()
            .unwrap();

        let result = participant.on_welcome(welcome_msg).await;
        assert!(matches!(
            result,
            Err(SessionError::InvalidParticipantSettingsLength)
        ));
    }
}

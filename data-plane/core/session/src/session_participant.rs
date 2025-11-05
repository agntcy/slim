// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{CommandPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType},
    messages::{Name, utils::SlimHeaderFlags},
};
use slim_mls::mls::Mls;
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    common::SessionMessage,
    errors::SessionError,
    traits::MessageHandler,
    mls_state::MlsState,
    session_builder::{ForParticipant, SessionBuilder},
    session_controller::SessionControllerCommon,
    session_settings::SessionSettings,
};

pub struct SessionParticipant<P, V>
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

    /// inner layer (Session)
    inner: crate::session::Session,
}

impl<P, V> SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Returns a new SessionBuilder for constructing a SessionParticipant
    pub fn builder() -> SessionBuilder<P, V, ForParticipant> {
        SessionBuilder::for_participant()
    }

    pub(crate) async fn new(settings: SessionSettings<P, V>) -> Self {
        let (tx_signals, _rx_signals) = tokio::sync::mpsc::channel(128);

        // Create the session (inner layer)
        let inner = crate::session::Session::new(
            settings.id,
            settings.config.clone(),
            &settings.source,
            settings.tx.clone(),
            tx_signals,
        );

        let mls_state = if settings.config.mls_enabled {
            Some(
                MlsState::new(Arc::new(Mutex::new(Mls::new(
                    settings.identity_provider.clone(),
                    settings.identity_verifier.clone(),
                    settings.storage_path.clone(),
                ))))
                .await
                .expect("failed to create MLS state"),
            )
        } else {
            None
        };

        let common = SessionControllerCommon::new_without_channels(
            settings.id,
            settings.source,
            settings.destination,
            settings.config,
            settings.tx,
            settings.tx_to_session_layer,
        );

        SessionParticipant {
            moderator_name: None,
            group_list: HashSet::new(),
            mls_state,
            common,
            subscribed: false,
            inner,
        }
    }
}

/// Implementation of MessageHandler trait for SessionParticipant
/// This allows the participant to be used as a layer in the generic layer system
#[async_trait]
impl<P, V> MessageHandler for SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        match message {
            SessionMessage::OnMessage { message, direction } => {
                if self
                    .common
                    .is_command_message(message.get_session_message_type())
                {
                    self.process_control_message(message).await
                } else {
                    self.inner
                        .on_message(SessionMessage::OnMessage { message, direction })
                        .await
                }
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts,
            } => {
                if self.common.is_command_message(message_type) {
                    self.common.sender.on_timer_timeout(message_id).await
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
                if self.common.is_command_message(message_type) {
                    self.common.sender.on_timer_failure(message_id).await;
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
            SessionMessage::StartDrain { grace_period_ms: _ } => todo!(),
            SessionMessage::DeleteSession { session_id: _ } => todo!(),
        }
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Participant-specific cleanup
        self.subscribed = false;
        // Shutdown inner layer
        MessageHandler::on_shutdown(&mut self.inner).await
    }
}

impl<P, V> SessionParticipant<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
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
        debug!(
            "received join request on {} with id {}",
            self.common.source,
            msg.get_id()
        );
        let source = msg.get_source();
        self.moderator_name = Some(source.clone());

        self.common
            .set_route(&source, msg.get_incoming_conn())
            .await?;

        let payload = if self.mls_state.is_some() {
            debug!("mls enabled, create the package key");
            let key = self
                .mls_state
                .as_mut()
                .unwrap()
                .generate_key_package()
                .await?;
            Some(key)
        } else {
            None
        };

        let content = CommandPayload::new_join_reply_payload(payload).as_content();

        debug!("send join reply message");
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
        debug!(
            "received welcome on {} with id {}",
            self.common.source,
            msg.get_id()
        );

        if self.mls_state.is_some() {
            self.mls_state
                .as_mut()
                .unwrap()
                .process_welcome_message(&msg)
                .await?;
        }

        self.join(&msg).await?;

        let list = msg
            .get_payload()
            .unwrap()
            .as_command_payload()?
            .as_welcome_payload()?
            .participants;
        for n in list {
            let name = Name::from(&n);
            self.group_list.insert(name.clone());

            if name != self.common.source {
                debug!("add endpoint to the session {}", msg.get_source());
                self.inner.add_endpoint(&name).await?;
            }
        }

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
        debug!(
            "received update on {} with id {}",
            self.common.source,
            msg.get_id()
        );

        if self.mls_state.is_some() {
            debug!("process mls control update");
            let ret = self
                .mls_state
                .as_mut()
                .unwrap()
                .process_control_message(msg.clone(), &self.common.source)
                .await?;

            if !ret {
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
                .as_command_payload()?
                .as_group_add_payload()?;
            if let Some(ref new_participant) = p.new_participant {
                let name = Name::from(new_participant);
                self.group_list.insert(name.clone());

                debug!("add endpoint to the session {}", msg.get_source());
                self.inner.add_endpoint(&name).await?;
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

                debug!("remove endpoint from the session {}", msg.get_source());
                self.inner.remove_endpoint(&name);
            }
        }

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
        let reply = self.common.create_control_message(
            &msg.get_source(),
            ProtoSessionMessageType::LeaveReply,
            msg.get_id(),
            CommandPayload::new_leave_reply_payload().as_content(),
            false,
        );

        self.common.send_to_slim(reply).await?;

        self.leave(&msg).await?;

        self.common
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.id,
            }))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to notify session layer: {}", e)))
    }

    async fn join(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        if self.common.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        self.common
            .set_route(&self.common.destination, msg.get_incoming_conn())
            .await?;
        let sub = Message::new_subscribe(
            &self.common.source,
            &self.common.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        );

        self.common.send_to_slim(sub).await
    }

    async fn leave(&self, msg: &Message) -> Result<(), SessionError> {
        self.common
            .delete_route(&self.common.destination, msg.get_incoming_conn())
            .await?;

        if self.common.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        self.common
            .delete_route(
                self.moderator_name.as_ref().unwrap(),
                msg.get_incoming_conn(),
            )
            .await?;
        let sub = Message::new_unsubscribe(
            &self.common.source,
            &self.common.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        );

        self.common.send_to_slim(sub).await
    }
}

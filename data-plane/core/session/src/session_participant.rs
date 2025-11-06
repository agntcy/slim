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
    common::SessionMessage, errors::SessionError, mls_state::MlsState,
    session_controller::SessionControllerCommon, session_settings::SessionSettings,
    traits::MessageHandler,
};

pub struct SessionParticipant<P, V, I>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    I: MessageHandler + Send + Sync + 'static,
{
    /// name of the moderator, used to send mls proposal messages
    moderator_name: Option<Name>,

    /// list of participants
    group_list: HashSet<Name>,

    /// mls state
    mls_state: Option<MlsState<P, V>>,

    /// common session state
    common: SessionControllerCommon<P, V>,

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
            group_list: HashSet::new(),
            mls_state: None,
            common,
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
            let mls_state = MlsState::new(Arc::new(Mutex::new(Mls::new(
                self.common.settings.identity_provider.clone(),
                self.common.settings.identity_verifier.clone(),
                self.common.settings.storage_path.clone(),
            ))))
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
            SessionMessage::OnMessage { message, direction } => {
                if message.get_session_message_type().is_command_message() {
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
                if message_type.is_command_message() {
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
                if message_type.is_command_message() {
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

    async fn add_endpoint(&mut self, endpoint: &Name) -> Result<(), SessionError> {
        self.inner.add_endpoint(endpoint).await
    }

    fn remove_endpoint(&mut self, endpoint: &Name) {
        self.inner.remove_endpoint(endpoint);
    }

    async fn on_shutdown(&mut self) -> Result<(), SessionError> {
        // Participant-specific cleanup
        self.subscribed = false;
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
            self.common.settings.source,
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
            self.common.settings.source,
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

        let list = &msg
            .get_payload()
            .unwrap()
            .as_command_payload()?
            .as_welcome_payload()?
            .participants;
        for n in list {
            let name = Name::from(n);
            self.group_list.insert(name.clone());

            if name != self.common.settings.source {
                debug!("add endpoint to the session {}", msg.get_source());
                self.add_endpoint(&name).await?;
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
            self.common.settings.source,
            msg.get_id()
        );

        if self.mls_state.is_some() {
            debug!("process mls control update");
            let ret = self
                .mls_state
                .as_mut()
                .unwrap()
                .process_control_message(msg.clone(), &self.common.settings.source)
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
            .settings
            .tx_to_session_layer
            .send(Ok(SessionMessage::DeleteSession {
                session_id: self.common.settings.id,
            }))
            .await
            .map_err(|e| SessionError::Processing(format!("failed to notify session layer: {}", e)))
    }

    async fn join(&mut self, msg: &Message) -> Result<(), SessionError> {
        if self.subscribed {
            return Ok(());
        }

        self.subscribed = true;

        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        self.common
            .set_route(&self.common.settings.destination, msg.get_incoming_conn())
            .await?;
        let sub = Message::new_subscribe(
            &self.common.settings.source,
            &self.common.settings.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        );

        self.common.send_to_slim(sub).await
    }

    async fn leave(&self, msg: &Message) -> Result<(), SessionError> {
        self.common
            .delete_route(&self.common.settings.destination, msg.get_incoming_conn())
            .await?;

        if self.common.settings.config.session_type == ProtoSessionType::PointToPoint {
            return Ok(());
        }

        self.common
            .delete_route(
                self.moderator_name.as_ref().unwrap(),
                msg.get_incoming_conn(),
            )
            .await?;
        let sub = Message::new_unsubscribe(
            &self.common.settings.source,
            &self.common.settings.destination,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(msg.get_incoming_conn())),
        );

        self.common.send_to_slim(sub).await
    }
}

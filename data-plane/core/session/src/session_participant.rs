// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashSet, marker::PhantomData, sync::Arc};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::{
    api::{CommandPayload, ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType},
    messages::{Name, utils::SlimHeaderFlags},
};
use slim_mls::mls::Mls;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{
    common::{MessageDirection, SessionMessage},
    errors::SessionError,
    mls_state::MlsState,
    session_builder::{ForParticipant, SessionBuilder},
    session_config::SessionConfig,
    session_controller::{SessionControllerCommon, SessionProcessor},
    session_settings::SessionSettings,
    transmitter::SessionTransmitter,
};

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
    /// Returns a new SessionBuilder for constructing a SessionParticipant
    pub fn builder() -> SessionBuilder<P, V, ForParticipant> {
        SessionBuilder::for_participant()
    }

    pub(crate) async fn new(settings: SessionSettings<P, V>) -> Self {
        let (tx_controller, rx_controller) = tokio::sync::mpsc::channel(128);

        let processor = SessionParticipantProcessor::new(
            settings.id,
            settings.source,
            settings.destination,
            settings.config,
            settings.identity_provider,
            settings.identity_verifier,
            settings.storage_path,
            settings.tx,
            tx_controller.clone(),
            rx_controller,
            settings.tx_to_session_layer,
            settings.cancellation_token,
        )
        .await;

        tokio::spawn(SessionControllerCommon::run_processor_loop(processor));

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

impl<P, V> SessionProcessor for SessionParticipantProcessor<P, V>
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

    fn common(&self) -> &SessionControllerCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut SessionControllerCommon {
        &mut self.common
    }
}

impl<P, V> SessionParticipantProcessor<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        id: u32,
        source: Name,
        destination: Name,
        config: SessionConfig,
        identity_provider: P,
        identity_verifier: V,
        storage_path: std::path::PathBuf,
        tx: SessionTransmitter,
        tx_controller: tokio::sync::mpsc::Sender<SessionMessage>,
        rx_controller: tokio::sync::mpsc::Receiver<SessionMessage>,
        tx_to_session_layer: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
        cancellation_token: CancellationToken,
    ) -> Self {
        let mls_state = if config.mls_enabled {
            Some(
                MlsState::new(Arc::new(tokio::sync::Mutex::new(Mls::new(
                    identity_provider,
                    identity_verifier,
                    storage_path,
                ))))
                .await
                .expect("failed to create MLS state"),
            )
        } else {
            None
        };

        let common = SessionControllerCommon::new(
            id,
            source,
            destination,
            config,
            tx,
            tx_controller,
            rx_controller,
            tx_to_session_layer,
            cancellation_token,
        );

        SessionParticipantProcessor {
            moderator_name: None,
            group_list: HashSet::new(),
            mls_state,
            common,
            subscribed: false,
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
                self.common.session.add_endpoint(&name).await?;
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
                self.common.session.add_endpoint(&name).await?;
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
                self.common.session.remove_endpoint(&name);
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

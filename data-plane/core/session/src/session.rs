// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use slim_datapath::{
    api::{ProtoMessage as Message, ProtoSessionMessageType},
    messages::Name,
};

// Local crate
use crate::{
    MessageDirection, common::SessionMessage, errors::SessionError,
    session_receiver::SessionReceiver, session_sender::SessionSender,
};

pub(crate) struct Session {
    sender: SessionSender,
    receiver: SessionReceiver,
}

impl Session {
    pub(crate) fn new(sender: SessionSender, receiver: SessionReceiver) -> Self {
        Session { sender, receiver }
    }

    pub async fn on_message(&mut self, message: SessionMessage) -> Result<(), SessionError> {
        match message {
            SessionMessage::OnMessage { message, direction } => {
                self.on_application_message(message, direction).await
            }
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_timeout(message_id, message_type, name).await,
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name,
                timeouts: _,
            } => self.on_timer_failure(message_id, message_type, name).await,
            SessionMessage::DeleteSession { session_id } => todo!(),
            SessionMessage::AddEndpoint { endpoint } => {
                self.sender.add_endpoint(endpoint);
                Ok(())
            }
            SessionMessage::RemoveEndpoint { endpoint } => {
                self.sender.remove_endpoint(&endpoint);
                Ok(())
            }
            SessionMessage::Drain { grace_period_ms } => todo!(),
        }
    }

    async fn on_application_message(
        &mut self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        match message.get_session_message_type() {
            ProtoSessionMessageType::Msg => {
                if direction == MessageDirection::South {
                    // message from app to slim, give it to the sender
                    self.sender.on_message(message).await.map(|_| ())
                } else {
                    // message from slim to the app, give it to the receiver
                    self.receiver.on_message(message).await.map(|_| ())
                }
            }
            ProtoSessionMessageType::MsgAck | ProtoSessionMessageType::RtxRequest => {
                self.sender.on_message(message).await.map(|_| ())
            }
            ProtoSessionMessageType::RtxReply => {
                self.receiver.on_message(message).await.map(|_| ())
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message.get_session_message_type()
            ))),
        }
    }

    async fn on_timer_timeout(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
    ) -> Result<(), SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => self.sender.on_timer_timeout(id).await,
            ProtoSessionMessageType::RtxRequest => {
                self.receiver.on_timer_timeout(id, name.unwrap()).await
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message_type
            ))),
        }
    }

    async fn on_timer_failure(
        &mut self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
    ) -> Result<(), SessionError> {
        match message_type {
            ProtoSessionMessageType::Msg => self.sender.on_timer_failure(id).await,
            ProtoSessionMessageType::RtxRequest => {
                self.receiver.on_timer_failure(id, name.unwrap()).await
            }
            _ => Err(SessionError::Processing(format!(
                "Unexpected message type {:?}",
                message_type
            ))),
        }
    }
}

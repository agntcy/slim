// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Third-party crates
use slim_datapath::api::ProtoMessage as Message;

// Local crate
use crate::session::info::Info;

/// Message wrapper
#[derive(Clone, PartialEq, Debug)]
pub struct SessionMessage {
    /// The message to be sent
    pub message: Message,
    /// The optional session info
    pub info: Info,
}

impl SessionMessage {
    /// Create a new session message
    pub fn new(message: Message, info: Info) -> Self {
        SessionMessage { message, info }
    }
}

impl From<(Message, Info)> for SessionMessage {
    fn from(tuple: (Message, Info)) -> Self {
        SessionMessage {
            message: tuple.0,
            info: tuple.1,
        }
    }
}

impl From<Message> for SessionMessage {
    fn from(message: Message) -> Self {
        let info = Info::from(&message);
        SessionMessage { message, info }
    }
}

impl From<SessionMessage> for Message {
    fn from(session_message: SessionMessage) -> Self {
        session_message.message
    }
}

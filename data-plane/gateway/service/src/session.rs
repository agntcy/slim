// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use parking_lot::RwLock;
use tonic::Status;

use crate::errors::SessionError;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::request_response::RequestResponseConfiguration;
use agp_datapath::messages::encoder::Agent;
use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;

/// Session ID
pub type Id = u32;

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

/// Channel used in the path service -> app
pub type AppChannelSender = tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>;
/// Channel used in the path app -> service
pub type AppChannelReceiver = tokio::sync::mpsc::Receiver<Result<SessionMessage, SessionError>>;
/// Channel used in the path service -> gw
pub type GwChannelSender = tokio::sync::mpsc::Sender<Result<Message, Status>>;
/// Channel used in the path gw -> service
pub type GwChannelReceiver = tokio::sync::mpsc::Receiver<Result<Message, Status>>;

/// Session Info
#[derive(Clone, PartialEq, Debug)]
pub struct Info {
    /// The id of the session
    pub id: Id,
    /// The message nonce used to identify the message
    pub message_id: Option<u32>,
    /// The identifier of the agent that sent the message
    pub message_source: Option<Agent>,
    /// The input connection id
    pub input_connection: Option<u64>,
}

impl Info {
    /// Create a new session info
    pub fn new(id: Id) -> Self {
        Info {
            id,
            message_id: None,
            message_source: None,
            input_connection: None,
        }
    }
}

impl From<&Message> for Info {
    fn from(message: &Message) -> Self {
        let session_header = utils::get_session_header(message).expect("session header not found");
        let agp_header = utils::get_agp_header(message).expect("AGP header not found");

        let id = session_header.session_id;
        let message_id = session_header.message_id;
        let message_source = utils::get_source(message).expect("message source not found");
        let input_connection = agp_header.incoming_conn;

        Info {
            id,
            message_id: Some(message_id),
            message_source: Some(message_source),
            input_connection,
        }
    }
}

/// The state of a session
#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Active,
    Inactive,
}

/// The type of a session
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum SessionDirection {
    #[allow(dead_code)]
    Sender,
    #[allow(dead_code)]
    Receiver,
    Bidirectional,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum MessageDirection {
    North,
    South,
}

#[derive(Clone, PartialEq, Debug)]
pub enum SessionConfig {
    FireAndForget(FireAndForgetConfiguration),
    RequestResponse(RequestResponseConfiguration),
}

impl std::fmt::Display for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionConfig::FireAndForget(ff) => write!(f, "{}", ff),
            SessionConfig::RequestResponse(rr) => write!(f, "{}", rr),
        }
    }
}

pub(crate) trait CommonSession {
    // Session ID
    #[allow(dead_code)]
    fn id(&self) -> Id;

    // get the session state
    #[allow(dead_code)]
    fn state(&self) -> &State;

    // get the session config
    #[allow(dead_code)]
    fn session_config(&self) -> SessionConfig;

    // set the session config
    #[allow(dead_code)]
    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError>;
}

#[async_trait]
pub(crate) trait Session: CommonSession {
    // publish a message as part of the session
    async fn on_message(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError>;
}

/// Common session data
pub(crate) struct Common {
    /// Session ID - unique identifier for the session
    #[allow(dead_code)]
    id: Id,

    /// Session state
    #[allow(dead_code)]
    state: State,

    /// Session type
    session_config: RwLock<SessionConfig>,

    /// Session direction
    #[allow(dead_code)]
    session_direction: SessionDirection,

    /// Sender for messages to gw
    tx_gw: GwChannelSender,

    /// Sender for messages to app
    tx_app: AppChannelSender,
}

impl CommonSession for Common {
    fn id(&self) -> Id {
        self.id
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn session_config(&self) -> SessionConfig {
        self.session_config.read().clone()
    }

    fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
        let mut conf = self.session_config.write();

        *conf = session_config.clone();
        Ok(())
    }
}

impl Common {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        session_type: SessionConfig,
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> Common {
        Common {
            id,
            state: State::Active,
            session_direction,
            session_config: RwLock::new(session_type),
            tx_gw,
            tx_app,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn tx_gw(&self) -> GwChannelSender {
        self.tx_gw.clone()
    }

    pub(crate) fn tx_gw_ref(&self) -> &GwChannelSender {
        &self.tx_gw
    }

    #[allow(dead_code)]
    pub(crate) fn tx_app(&self) -> AppChannelSender {
        self.tx_app.clone()
    }

    pub(crate) fn tx_app_ref(&self) -> &AppChannelSender {
        &self.tx_app
    }
}

// Define a macro to delegate trait implementation
macro_rules! delegate_common_behavior {
    ($parent:ident, $($tokens:ident),+) => {
        impl CommonSession for $parent {
            fn id(&self) -> Id {
                // concat the token stream
                self.$($tokens).+.id()
            }

            fn state(&self) -> &State {
                self.$($tokens).+.state()
            }

            fn session_config(&self) -> SessionConfig {
                self.$($tokens).+.session_config()
            }

            fn set_session_config(&self, session_config: &SessionConfig) -> Result<(), SessionError> {
                self.$($tokens).+.set_session_config(session_config)
            }
        }
    };
}

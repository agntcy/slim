// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;
use tonic::Status;

use agp_datapath::pubsub::proto::pubsub::v1::Message;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("error receiving message from gateway {0}")]
    #[allow(dead_code)]
    GatewayReception(String),
    #[error("error sending message to gateway {0}")]
    GatewayTransmission(String),
    #[error("error receiving message from app {0}")]
    #[allow(dead_code)]
    AppReception(String),
    #[error("error sending message to app {0}")]
    AppTransmission(String),
    #[error("error processing message {0}")]
    #[allow(dead_code)]
    Processing(String),
    #[error("session id already used {0}")]
    SessionIdAlreadyUsed(String),
    #[error("missing AGP header {0}")]
    MissingAgpHeader(String),
    #[error("missing session header")]
    MissingSessionHeader,
    #[error("session unknown: {0}")]
    SessionUnknown(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("missing session id: {0}")]
    MissingSessionId(String),
}

/// Session ID
pub type Id = u32;

/// Session Info
#[derive(Clone, PartialEq, Debug)]
pub struct Info {
    pub id: Id,
    pub session_type: SessionType,
    pub state: State,

    pub message_nonce: u32,
    pub message_count: u32,
}

impl Info {
    pub fn new(id: Id, session_type: SessionType, state: State) -> Info {
        Info {
            id,
            session_type,
            state,
            message_nonce: 0,
            message_count: 0,
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
pub enum SessionType {
    FireAndForget,
    RequestResponse,
    PublishSubscribe,
    Streaming,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::FireAndForget => write!(f, "FireAndForget"),
            SessionType::RequestResponse => write!(f, "RequestResponse"),
            SessionType::PublishSubscribe => write!(f, "PublishSubscribe"),
            SessionType::Streaming => write!(f, "Streaming"),
        }
    }
}

pub(crate) trait Session {
    // Session ID
    #[allow(dead_code)]
    fn id(&self) -> Id;

    // get the session state
    #[allow(dead_code)]
    fn state(&self) -> &State;

    // get the session type
    #[allow(dead_code)]
    fn session_type(&self) -> SessionType;

    // publish a message as part of the session
    fn on_message(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'static>>;
}

/// Common session data
pub(crate) struct Common {
    /// Session ID - unique identifier for the session
    id: Id,

    /// Session state
    state: State,

    /// Session direction
    #[allow(dead_code)]
    session_direction: SessionDirection,

    /// Sender for messages to gw
    tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,

    /// Sender for messages to app
    tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
}

impl Common {
    pub(crate) fn new(
        id: Id,
        session_direction: SessionDirection,
        tx_gw: tokio::sync::mpsc::Sender<Result<Message, Status>>,
        tx_app: tokio::sync::mpsc::Sender<(Message, Info)>,
    ) -> Common {
        Common {
            id,
            state: State::Active,
            session_direction,
            tx_gw,
            tx_app,
        }
    }

    /// get the session ID
    pub(crate) fn id(&self) -> Id {
        self.id
    }

    /// get the session state
    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn tx_gw(&self) -> tokio::sync::mpsc::Sender<Result<Message, Status>> {
        self.tx_gw.clone()
    }

    pub(crate) fn tx_app(&self) -> tokio::sync::mpsc::Sender<(Message, Info)> {
        self.tx_app.clone()
    }
}

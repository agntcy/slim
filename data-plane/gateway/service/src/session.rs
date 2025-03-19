// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use agp_datapath::pubsub::proto::pubsub::v1::Message;

/// Session ID
pub type Id = u32;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("error receiving message from gateway {0}")]
    GatewayReceptionError(String),
    #[error("error sending message to gateway {0}")]
    GatewayTransmissionError(String),
    #[error("error receiving message from app {0}")]
    AppReceptionError(String),
    #[error("error sending message to app {0}")]
    AppTransmissionError(String),
    #[error("error processing message {0}")]
    ProcessingError(String),
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

/// The state of a session
pub(crate) enum State {
    Active,
    Inactive,
}

/// The type of a session
pub(crate) enum SessionDirection {
    Sender,
    Receiver,
    Bidirectional,
}

#[derive(PartialEq)]
pub(crate) enum MessageDirection {
    North,
    South,
}

pub(crate) enum SessionType {
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
    fn id(&self) -> Id;

    // get the session state
    fn state(&self) -> &State;

    // get the session type
    fn session_type(&self) -> &SessionDirection;

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

    /// Session type
    session_type: SessionDirection,

    /// tx channel to send messages to the underlying gateway
    south_tx: tokio::sync::mpsc::Sender<Message>,

    /// rx channel to receive messages from the underlying gateway
    south_rx: tokio::sync::mpsc::Receiver<Message>,

    /// tx channel to send messages to the app
    north_tx: tokio::sync::mpsc::Sender<Message>,

    /// rx channel to receive messages from the app
    north_rx: tokio::sync::mpsc::Receiver<Message>,
}

impl Common {
    pub(crate) fn new(id: Id, session_type: SessionDirection) -> Common {
        // create the internal channel
        let (south_tx, south_rx) = tokio::sync::mpsc::channel(128);
        let (north_tx, north_rx) = tokio::sync::mpsc::channel(128);

        Common {
            id,
            state: State::Active,
            session_type,
            south_tx,
            south_rx,
            north_tx,
            north_rx,
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

    /// get the session type
    pub(crate) fn session_type(&self) -> &SessionDirection {
        &self.session_type
    }

    pub(crate) fn south_tx(&self) -> tokio::sync::mpsc::Sender<Message> {
        self.south_tx.clone()
    }

    pub(crate) fn north_tx(&self) -> tokio::sync::mpsc::Sender<Message> {
        self.north_tx.clone()
    }
}

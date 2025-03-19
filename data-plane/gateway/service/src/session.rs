// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::pin::Pin;

use agp_datapath::messages::AgentType;
use thiserror::Error;

use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::tables::pool::Pool;

pub(crate) type Id = u64;

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
}

pub(crate) enum State {
    Active,
    Inactive,
}

pub(crate) enum SessionType {
    Publisher,
    Subscriber,
}

pub(crate) trait Session {
    // Session ID
    fn id(&self) -> Id;

    // get the session state
    fn state(&self) -> &State;

    // get the session type
    fn session_type(&self) -> &SessionType;

    // publish a message as part of the session
    fn on_message_from_app(
        &self,
        message: Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

    // receive a message as part of the session
    fn on_message_from_gateway(
        &self,
        message: Message,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
}

/// Common session data
pub(crate) struct Common {
    /// Session ID - unique identifier for the session
    id: Id,

    /// Session state
    state: State,

    /// Session type
    session_type: SessionType,

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
    pub(crate) fn new(id: Id, session_type: SessionType) -> Common {
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

    pub(crate) fn id(&self) -> Id {
        self.id
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn session_type(&self) -> &SessionType {
        &self.session_type
    }

    pub(crate) fn south_tx(&self) -> tokio::sync::mpsc::Sender<Message> {
        self.south_tx.clone()
    }

    pub(crate) fn north_tx(&self) -> tokio::sync::mpsc::Sender<Message> {
        self.north_tx.clone()
    }


}

pub(crate) struct SessionPool {
    pub(crate) pool: Pool<Box<dyn Session>>,
}

impl SessionPool {
    pub(crate) fn new() -> SessionPool {
        SessionPool {
            pool: Pool::with_capacity(128),
        }
    }

    pub(crate) fn insert(&mut self, session: Box<dyn Session>) -> Id {
        self.pool.insert(session)
    }

    pub(crate) fn remove(&mut self, id: Id) {
        self.pool.remove(id);
    }

    pub(crate) fn get(&self, id: Id) -> Option<&Box<dyn Session>> {
        self.pool.get(id)
    }
}

impl std::fmt::Debug for SessionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionPool")
    }
}

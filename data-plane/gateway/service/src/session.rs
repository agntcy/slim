// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::hash::Hash;

use agp_datapath::messages::AgentType;
use thiserror::Error;

use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::tables::pool::Pool;

pub(crate) type Id = u64;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("error {0}")]
    GenericError(String),
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
    fn on_publish(&self, message: &mut Message) -> Result<(), Error>;

    // receive a message as part of the session
    fn on_receive(&self, message: &mut Message) -> Result<(), Error>;
}

pub(crate) struct Common {
    id: Id,
    state: State,
    session_type: SessionType,
}

impl Common {
    pub(crate) fn new(id: Id, session_type: SessionType) -> Common {
        Common {
            id,
            state: State::Active,
            session_type,
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
}

pub(crate) struct SessionMap {
    pool: Pool<Option<Box<dyn Session>>>,
}

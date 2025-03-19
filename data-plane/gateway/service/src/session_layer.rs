// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use rand::Rng;
use tokio::sync::RwLock;

use crate::fire_and_forget;
use crate::session::{Error, Id, MessageDirection, Session, SessionDirection, SessionType};
use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::ServiceHeaderType;

/// SessionLayer
pub(crate) struct SessionLayer {
    /// Session pool
    pool: RwLock<HashMap<Id, Box<dyn Session + Send + Sync>>>,
}

impl std::fmt::Debug for SessionLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionPool")
    }
}

impl SessionLayer {
    /// Create a new session pool
    pub(crate) fn new() -> SessionLayer {
        SessionLayer {
            pool: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a new session into the pool
    pub(crate) async fn insert_session(
        &self,
        id: Id,
        session: Box<dyn Session + Send + Sync>,
    ) -> Result<(), Error> {
        // get the write lock
        let mut pool = self.pool.write().await;

        // check if the session already exists
        if pool.contains_key(&id) {
            return Err(Error::SessionIdAlreadyUsed(id.to_string()));
        }

        pool.insert(id, session);

        Ok(())
    }

    pub(crate) async fn create_session(&self, session_type: SessionType) -> Result<Id, Error> {
        // generate a new session ID
        let id = rand::rng().random();

        // create a new session
        let session = match session_type {
            SessionType::FireAndForget => Box::new(fire_and_forget::FireAndForget::new(
                id,
                SessionDirection::Bidirectional,
            )),
            _ => return Err(Error::SessionUnknown(session_type.to_string())),
        };

        // insert the session into the pool
        self.insert_session(id, session).await?;

        Ok(id)
    }

    /// Remove a session from the pool
    pub(crate) async fn remove_session(&mut self, id: Id) -> bool {
        // get the write lock
        let mut pool = self.pool.write().await;

        pool.remove(&id).is_some()
    }

    /// Handle a message and pass it to the corresponding session
    pub(crate) async fn handle_message(
        &self,
        message: Message,
        direction: MessageDirection,
        session_id: Option<Id>,
    ) -> Result<(), Error> {
        match direction {
            MessageDirection::North => self.handle_message_from_gateway(message, direction).await,
            MessageDirection::South => {
                self.handle_message_from_app(message, direction, session_id)
                    .await
            }
        }
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_app(
        &self,
        mut message: Message,
        direction: MessageDirection,
        session_id: Option<Id>,
    ) -> Result<(), Error> {
        // if the session ID is not specified, return an error
        if session_id.is_none() {
            return Err(Error::MissingSessionId("None".to_string()));
        }

        let id = session_id.unwrap();

        // check if pool contains the session
        if let Some(session) = self.pool.read().await.get(&id) {
            // Set session id and session type to message
            let header = utils::get_session_header_as_mut(&mut message);
            if header.is_none() {
                return Err(Error::MissingSessionHeader);
            }

            let header = header.unwrap();
            header.id = id;

            // pass the message to the session
            return session.on_message(message, direction).await;
        }

        // if the session is not found, return an error
        Err(Error::SessionNotFound(id.to_string()))
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_gateway(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), Error> {
        let (id, session_type) = {
            // get the session type and the session id from the message
            let header = utils::get_session_header(&message);

            // if header is None, return an error
            if header.is_none() {
                return Err(Error::MissingAgpHeader("missing AGP header".to_string()));
            }

            let header = header.unwrap();

            // get the session type from the header
            let session_type = utils::int_to_service_type(header.header_type);

            // if the session type is not specified, return an error
            if session_type.is_none() {
                return Err(Error::SessionUnknown(header.header_type.to_string()));
            }

            // get the session ID
            let id = header.id;

            (id, session_type.unwrap())
        };

        // check if pool contains the session
        if let Some(session) = self.pool.read().await.get(&id) {
            // pass the message to the session
            let ret = session.on_message(message, direction).await;
            return ret;
        }

        // if the session is not found and the direction is North, create a new session
        if direction != MessageDirection::North {
            return Err(Error::SessionNotFound(id.to_string()));
        }

        let new_session_id = match session_type {
            ServiceHeaderType::CtrlFnf => self.create_session(SessionType::FireAndForget).await?,
            _ => {
                return Err(Error::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ))
            }
        };

        // retry the match
        if let Some(session) = self.pool.read().await.get(&new_session_id) {
            // pass the message
            return session.on_message(message, direction).await;
        }

        // this should never happen
        panic!("session not found: {}", "test");
    }
}

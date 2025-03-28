// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use rand::Rng;
use tokio::sync::RwLock;

use crate::errors::SessionError;
use crate::fire_and_forget;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::request_response;
use crate::session::{
    AppChannelSender, GwChannelSender, Id, MessageDirection, Session, SessionConfig,
    SessionDirection,
};
use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;

/// SessionLayer
pub(crate) struct SessionLayer {
    /// Session pool
    pool: RwLock<HashMap<Id, Box<dyn Session + Send + Sync>>>,

    /// ID of the local connection
    conn_id: u64,

    /// Tx channels
    tx_gw: GwChannelSender,
    tx_app: AppChannelSender,
}

impl std::fmt::Debug for SessionLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionPool")
    }
}

impl SessionLayer {
    /// Create a new session pool
    pub(crate) fn new(
        conn_id: u64,
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> SessionLayer {
        SessionLayer {
            pool: RwLock::new(HashMap::new()),
            conn_id,
            tx_gw,
            tx_app,
        }
    }

    pub(crate) fn tx_gw(&self) -> GwChannelSender {
        self.tx_gw.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn tx_app(&self) -> AppChannelSender {
        self.tx_app.clone()
    }

    pub(crate) fn conn_id(&self) -> u64 {
        self.conn_id
    }

    /// Insert a new session into the pool
    pub(crate) async fn insert_session(
        &self,
        id: Id,
        session: Box<dyn Session + Send + Sync>,
    ) -> Result<(), SessionError> {
        // get the write lock
        let mut pool = self.pool.write().await;

        // check if the session already exists
        if pool.contains_key(&id) {
            return Err(SessionError::SessionIdAlreadyUsed(id.to_string()));
        }

        pool.insert(id, session);

        Ok(())
    }

    pub(crate) async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<Id, SessionError> {
        // TODO(msardara): the session identifier should be a combination of the
        // session ID and the agent ID, to prevent collisions.

        // generate a new session ID
        let id = match id {
            Some(id) => id,
            None => rand::rng().random(),
        };

        // create a new session
        let session: Box<(dyn Session + Send + Sync + 'static)> = match session_config {
            SessionConfig::FireAndForget(conf) => Box::new(fire_and_forget::FireAndForget::new(
                id,
                conf,
                SessionDirection::Bidirectional,
                self.tx_gw.clone(),
                self.tx_app.clone(),
            )),
            SessionConfig::RequestResponse(conf) => {
                Box::new(request_response::RequestResponse::new(
                    id,
                    conf,
                    SessionDirection::Bidirectional,
                    self.tx_gw.clone(),
                    self.tx_app.clone(),
                ))
            }
        };

        // insert the session into the pool
        self.insert_session(id, session).await?;

        Ok(id)
    }

    /// Remove a session from the pool
    pub(crate) async fn remove_session(&self, id: Id) -> bool {
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
    ) -> Result<(), SessionError> {
        match direction {
            MessageDirection::North => self.handle_message_from_gateway(message, direction).await,
            MessageDirection::South => {
                // make sure the session ID is provided
                let session_id = match session_id {
                    Some(id) => id,
                    None => return Err(SessionError::MissingSessionId("none".to_string())),
                };

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
        session_id: Id,
    ) -> Result<(), SessionError> {
        // check if pool contains the session
        if let Some(session) = self.pool.read().await.get(&session_id) {
            // Set session id and session type to message
            let header = utils::get_session_header_as_mut(&mut message);
            if header.is_none() {
                return Err(SessionError::MissingSessionHeader);
            }

            let header = header.unwrap();
            header.session_id = session_id;

            // pass the message to the session
            return session.on_message(message, direction).await;
        }

        // if the session is not found, return an error
        Err(SessionError::SessionNotFound(session_id.to_string()))
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_gateway(
        &self,
        message: Message,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        let (id, session_type) = {
            // get the session type and the session id from the message
            let header = utils::get_session_header(&message);

            // if header is None, return an error
            if header.is_none() {
                return Err(SessionError::MissingAgpHeader(
                    "missing AGP header".to_string(),
                ));
            }

            let header = header.unwrap();

            // get the session type from the header
            let session_type = utils::int_to_service_type(header.header_type);

            // if the session type is not specified, return an error
            if session_type.is_none() {
                return Err(SessionError::SessionUnknown(header.header_type.to_string()));
            }

            // get the session ID
            let id = header.session_id;

            (id, session_type.unwrap())
        };

        // check if pool contains the session
        if let Some(session) = self.pool.read().await.get(&id) {
            // pass the message to the session
            let ret = session.on_message(message, direction).await;
            return ret;
        }

        let new_session_id = match session_type {
            SessionHeaderType::Fnf => {
                self.create_session(
                    SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                    Some(id),
                )
                .await?
            }
            SessionHeaderType::Request => {
                self.create_session(
                    SessionConfig::RequestResponse(
                        request_response::RequestResponseConfiguration::default(),
                    ),
                    Some(id),
                )
                .await?
            }
            _ => {
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ))
            }
        };

        debug_assert!(new_session_id == id);

        // retry the match
        if let Some(session) = self.pool.read().await.get(&new_session_id) {
            // pass the message
            return session.on_message(message, direction).await;
        }

        // this should never happen
        panic!("session not found: {}", "test");
    }

    /// Set the configuration of a session
    pub(crate) async fn set_session_config(
        &self,
        session_id: Id,
        session_config: &SessionConfig,
    ) -> Result<(), SessionError> {
        // get the write lock
        let mut pool = self.pool.write().await;

        // check if the session exists
        if let Some(session) = pool.get_mut(&session_id) {
            // set the session config
            session.set_session_config(session_config)?
        }

        Err(SessionError::SessionNotFound(session_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fire_and_forget::{FireAndForget, FireAndForgetConfiguration};
    use crate::session::State;

    use agp_datapath::messages::encoder;

    fn create_session_layer() -> SessionLayer {
        let (tx_gw, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);

        SessionLayer::new(0, tx_gw, tx_app)
    }

    #[tokio::test]
    async fn test_create_session_layer() {
        let session_layer = create_session_layer();

        assert!(session_layer.pool.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_insert_session() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_layer = SessionLayer::new(0, tx_gw.clone(), tx_app.clone());
        let session_config = FireAndForgetConfiguration {};

        let session = Box::new(FireAndForget::new(
            1,
            session_config,
            SessionDirection::Bidirectional,
            tx_gw.clone(),
            tx_app.clone(),
        ));

        let res = session_layer.insert_session(1, session).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_layer = SessionLayer::new(0, tx_gw.clone(), tx_app.clone());
        let session_config = FireAndForgetConfiguration {};

        let session = Box::new(FireAndForget::new(
            1,
            session_config,
            SessionDirection::Bidirectional,
            tx_gw.clone(),
            tx_app.clone(),
        ));

        session_layer.insert_session(1, session).await.unwrap();
        let res = session_layer.remove_session(1).await;

        assert!(res);
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);

        let session_layer = SessionLayer::new(0, tx_gw.clone(), tx_app.clone());

        let res = session_layer
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration {}),
                None,
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_handle_message() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);

        let session_layer = SessionLayer::new(0, tx_gw.clone(), tx_app.clone());

        let session_config = FireAndForgetConfiguration {};

        let session = Box::new(FireAndForget::new(
            1,
            session_config,
            SessionDirection::Bidirectional,
            tx_gw.clone(),
            tx_app.clone(),
        ));

        session_layer.insert_session(1, session).await.unwrap();

        let mut message = utils::create_publication(
            &encoder::encode_agent("cisco", "default", "local_agent", 0),
            &encoder::encode_agent_type("cisco", "default", "remote_agent"),
            Some(0),
            None,
            None,
            1,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = utils::get_session_header_as_mut(&mut message).unwrap();
        header.session_id = 1;

        let res = session_layer
            .handle_message(message.clone(), MessageDirection::North, Some(1))
            .await;

        assert!(res.is_ok());

        // message should have been delivered to the app
        let (msg, info) = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg, message);
        assert_eq!(info.id, 1);
        assert_eq!(info.state, State::Active);
    }
}

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::Duration;

use rand::Rng;
use tokio::sync::RwLock;

use crate::errors::SessionError;
use crate::fire_and_forget;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::request_response;
use crate::session::{
    AppChannelSender, GwChannelSender, Id, Info, MessageDirection, Session, SessionConfig,
    SessionDirection, SessionMessage, SESSION_RANGE,
};
use crate::streaming;
use agp_datapath::messages::encoder::Agent;
use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;

/// SessionLayer
pub(crate) struct SessionLayer {
    /// Session pool
    pool: RwLock<HashMap<Id, Box<dyn Session + Send + Sync>>>,

    /// Name of the local agent
    agent_name: Agent,

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
        agent_name: &Agent,
        conn_id: u64,
        tx_gw: GwChannelSender,
        tx_app: AppChannelSender,
    ) -> SessionLayer {
        SessionLayer {
            pool: RwLock::new(HashMap::new()),
            agent_name: agent_name.clone(),
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

    pub(crate) fn agent_name(&self) -> &Agent {
        &self.agent_name
    }

    pub(crate) async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<Info, SessionError> {
        // TODO(msardara): the session identifier should be a combination of the
        // session ID and the agent ID, to prevent collisions.

        // get a lock on the session pool
        let mut pool = self.pool.write().await;

        // generate a new session ID in the SESSION_RANGE if not provided
        let id = match id {
            Some(id) => {
                // make sure provided id is in range
                if !SESSION_RANGE.contains(&id) {
                    return Err(SessionError::InvalidSessionId(id.to_string()));
                }

                // check if the session ID is already used
                if pool.contains_key(&id) {
                    return Err(SessionError::SessionIdAlreadyUsed(id.to_string()));
                }

                id
            }
            None => {
                // generate a new session ID
                loop {
                    let id = rand::rng().random_range(SESSION_RANGE);
                    if !pool.contains_key(&id) {
                        break id;
                    }
                }
            }
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
            SessionConfig::Streaming(conf) => {
                let mut direction = SessionDirection::Receiver;
                if conf.max_retries == 0 {
                    direction = SessionDirection::Sender;
                }
                Box::new(streaming::Streaming::new(
                    id,
                    conf,
                    direction,
                    self.tx_gw.clone(),
                    self.tx_app.clone(),
                ))
            }
        };

        // insert the session into the pool
        let ret = pool.insert(id, session);

        // This should never happen, but just in case
        if ret.is_some() {
            panic!("session already exists: {}", ret.is_some());
        }

        Ok(Info::new(id))
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
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // Validate the message as first operation to prevent possible panic in case
        // necessary fields are missing
        if let Err(e) = message.message.validate() {
            return Err(SessionError::ValidationError(e.to_string()));
        }

        // Also make sure the message is a publication
        if !message.message.is_publish() {
            return Err(SessionError::ValidationError(
                "message is not a publish".to_string(),
            ));
        }

        // good to go
        match direction {
            MessageDirection::North => self.handle_message_from_gateway(message, direction).await,
            MessageDirection::South => self.handle_message_from_app(message, direction).await,
        }
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_app(
        &self,
        mut message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        // check if pool contains the session
        if let Some(session) = self.pool.read().await.get(&message.info.id) {
            // Set session id and session type to message
            let header = message.message.get_session_header_mut();
            header.session_id = message.info.id;

            // pass the message to the session
            return session.on_message(message, direction).await;
        }

        // if the session is not found, return an error
        Err(SessionError::SessionNotFound(message.info.id.to_string()))
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_gateway(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        let (id, session_type) = {
            // get the session type and the session id from the message
            let header = message.message.get_session_header();

            // get the session type from the header
            let session_type = match SessionHeaderType::try_from(header.header_type) {
                Ok(session_type) => session_type,
                Err(e) => {
                    return Err(SessionError::ValidationError(format!(
                        "session type is not valid: {}",
                        e
                    )));
                }
            };

            // get the session ID
            let id = header.session_id;

            (id, session_type)
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
            SessionHeaderType::Stream => {
                self.create_session(
                    SessionConfig::Streaming(streaming::StreamingConfiguration {
                        source: self.agent_name().clone(),
                        max_retries: 10,
                        timeout: Duration::from_millis(1000),
                    }),
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

        debug_assert!(new_session_id.id == id);

        // retry the match
        if let Some(session) = self.pool.read().await.get(&new_session_id.id) {
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
    use crate::fire_and_forget::FireAndForgetConfiguration;

    use agp_datapath::{
        messages::{Agent, AgentType},
        pubsub::ProtoMessage,
    };

    fn create_session_layer() -> SessionLayer {
        let (tx_gw, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        SessionLayer::new(&agent, 0, tx_gw, tx_app)
    }

    #[tokio::test]
    async fn test_create_session_layer() {
        let session_layer = create_session_layer();

        assert!(session_layer.pool.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_gw.clone(), tx_app.clone());
        let session_config = FireAndForgetConfiguration {};

        let ret = session_layer
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;

        assert!(ret.is_ok());

        let res = session_layer.remove_session(1).await;
        assert!(res);
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_gw, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_gw.clone(), tx_app.clone());

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
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_gw.clone(), tx_app.clone());

        let session_config = FireAndForgetConfiguration {};

        // create a new session
        let res = session_layer
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;
        assert!(res.is_ok());

        let mut message = ProtoMessage::new_publish(
            &Agent::from_strings("cisco", "default", "local_agent", 0),
            &AgentType::from_strings("cisco", "default", "remote_agent"),
            Some(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.header_type = i32::from(SessionHeaderType::Fnf);

        let res = session_layer
            .handle_message(
                SessionMessage::from(message.clone()),
                MessageDirection::North,
            )
            .await;

        assert!(res.is_ok());

        // message should have been delivered to the app
        let msg = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg.message, message);
        assert_eq!(msg.info.id, 1);
    }
}

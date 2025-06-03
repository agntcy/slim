// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use tokio::sync::RwLock as AsyncRwLock;
use tracing::warn;

use crate::errors::SessionError;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::request_response::{RequestResponse, RequestResponseConfiguration};
use crate::session::{
    AppChannelSender, Id, Info, MessageDirection, SESSION_RANGE, Session, SessionConfig,
    SessionConfigTrait, SessionDirection, SessionMessage, SessionType, SlimChannelSender,
};
use crate::streaming::{self, StreamingConfiguration};
use crate::{fire_and_forget, session};
use slim_datapath::api::proto::pubsub::v1::SessionHeaderType;
use slim_datapath::messages::encoder::Agent;

/// SessionLayer
pub(crate) struct SessionLayer {
    /// Session pool
    pool: AsyncRwLock<HashMap<Id, Box<dyn Session + Send + Sync>>>,

    /// Name of the local agent
    agent_name: Agent,

    /// ID of the local connection
    conn_id: u64,

    /// Tx channels
    tx_slim: SlimChannelSender,
    tx_app: AppChannelSender,

    /// Default configuration for the session
    default_ff_conf: SyncRwLock<FireAndForgetConfiguration>,
    default_rr_conf: SyncRwLock<RequestResponseConfiguration>,
    default_stream_conf: SyncRwLock<StreamingConfiguration>,
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
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
    ) -> SessionLayer {
        SessionLayer {
            pool: AsyncRwLock::new(HashMap::new()),
            agent_name: agent_name.clone(),
            conn_id,
            tx_slim,
            tx_app,
            default_ff_conf: SyncRwLock::new(FireAndForgetConfiguration::default()),
            default_rr_conf: SyncRwLock::new(RequestResponseConfiguration::default()),
            default_stream_conf: SyncRwLock::new(StreamingConfiguration::default()),
        }
    }

    pub(crate) fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

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
        let mut id = match id {
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
                self.agent_name().clone(),
                self.tx_slim.clone(),
                self.tx_app.clone(),
            )),
            SessionConfig::RequestResponse(conf) => Box::new(RequestResponse::new(
                id,
                conf,
                SessionDirection::Bidirectional,
                self.agent_name().clone(),
                self.tx_slim.clone(),
                self.tx_app.clone(),
            )),
            SessionConfig::Streaming(conf) => {
                let direction = conf.direction.clone();
                if direction == SessionDirection::Bidirectional {
                    // TODO(micpapal/msardara): this is a temporary solution to get a session
                    // id that is common to all the agents that subscribe
                    // for the same topic.
                    id = (slim_datapath::messages::encoder::calculate_hash(&conf.topic)
                        % (u32::MAX as u64)) as u32;
                }

                Box::new(streaming::Streaming::new(
                    id,
                    conf,
                    direction,
                    self.agent_name().clone(),
                    self.tx_slim.clone(),
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
            MessageDirection::North => self.handle_message_from_slim(message, direction).await,
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
    async fn handle_message_from_slim(
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
            SessionHeaderType::Fnf
            | SessionHeaderType::FnfReliable
            | SessionHeaderType::FnfDiscovery => {
                let conf = self.default_ff_conf.read().clone();
                self.create_session(SessionConfig::FireAndForget(conf), Some(id))
                    .await?
            }
            SessionHeaderType::Request => {
                let conf = self.default_rr_conf.read().clone();
                self.create_session(SessionConfig::RequestResponse(conf), Some(id))
                    .await?
            }
            SessionHeaderType::Stream | SessionHeaderType::BeaconStream => {
                let conf = self.default_stream_conf.read().clone();
                self.create_session(session::SessionConfig::Streaming(conf), Some(id))
                    .await?
            }
            SessionHeaderType::PubSub => {
                warn!("received pub/sub message with unknown session id");
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }
            SessionHeaderType::BeaconPubSub => {
                warn!("received beacon pub/sub message with unknown session id");
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }
            _ => {
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
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
        session_config: &SessionConfig,
        session_id: Option<Id>,
    ) -> Result<(), SessionError> {
        // If no session ID is provided, modify the default session
        let session_id = match session_id {
            Some(id) => id,
            None => {
                // modify the default session
                match &session_config {
                    SessionConfig::FireAndForget(_) => {
                        return self.default_ff_conf.write().replace(session_config);
                    }
                    SessionConfig::RequestResponse(_) => {
                        return self.default_rr_conf.write().replace(session_config);
                    }
                    SessionConfig::Streaming(_) => {
                        return self.default_stream_conf.write().replace(session_config);
                    }
                }
            }
        };

        // get the write lock
        let mut pool = self.pool.write().await;

        // check if the session exists
        if let Some(session) = pool.get_mut(&session_id) {
            // set the session config
            return session.set_session_config(session_config);
        }

        Err(SessionError::SessionNotFound(session_id.to_string()))
    }

    /// Get the session configuration
    pub(crate) async fn get_session_config(
        &self,
        session_id: Id,
    ) -> Result<SessionConfig, SessionError> {
        // get the read lock
        let pool = self.pool.read().await;

        // check if the session exists
        if let Some(session) = pool.get(&session_id) {
            return Ok(session.session_config());
        }

        Err(SessionError::SessionNotFound(session_id.to_string()))
    }

    /// Get the session configuration
    pub(crate) async fn get_default_session_config(
        &self,
        session_type: SessionType,
    ) -> Result<SessionConfig, SessionError> {
        match session_type {
            SessionType::FireAndForget => Ok(SessionConfig::FireAndForget(
                self.default_ff_conf.read().clone(),
            )),
            SessionType::RequestResponse => Ok(SessionConfig::RequestResponse(
                self.default_rr_conf.read().clone(),
            )),
            SessionType::Streaming => Ok(SessionConfig::Streaming(
                self.default_stream_conf.read().clone(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fire_and_forget::FireAndForgetConfiguration;

    use slim_datapath::{
        api::ProtoMessage,
        messages::{Agent, AgentType},
    };

    fn create_session_layer() -> SessionLayer {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        SessionLayer::new(&agent, 0, tx_slim, tx_app)
    }

    #[tokio::test]
    async fn test_create_session_layer() {
        let session_layer = create_session_layer();

        assert!(session_layer.pool.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_slim.clone(), tx_app.clone());
        let session_config = FireAndForgetConfiguration::default();

        let ret = session_layer
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;

        assert!(ret.is_ok());

        let res = session_layer.remove_session(1).await;
        assert!(res);
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_slim.clone(), tx_app.clone());

        let res = session_layer
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                None,
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_slim.clone(), tx_app.clone());

        let res = session_layer
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                Some(1),
            )
            .await;
        assert!(res.is_ok());

        let res = session_layer.remove_session(1).await;
        assert!(res);

        // try to delete a non-existing session
        let res = session_layer.remove_session(1).await;
        assert!(!res);
    }

    #[tokio::test]
    async fn test_handle_message() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = SessionLayer::new(&agent, 0, tx_slim.clone(), tx_app.clone());

        let session_config = FireAndForgetConfiguration::default();

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

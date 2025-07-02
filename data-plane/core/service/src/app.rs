// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use drain::Watch;
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use slim_auth::simple::Simple;
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{MessageType, SessionHeader, SlimHeader};
use slim_datapath::messages::AgentType;
use slim_datapath::messages::utils::SlimHeaderFlags;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::errors::SessionError;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::request_response::{RequestResponse, RequestResponseConfiguration};
use crate::session::{
    AppChannelSender, CommonSession, Id, Info, Interceptor, MessageDirection, MessageHandler,
    SESSION_RANGE, Session, SessionConfig, SessionConfigTrait, SessionDirection,
    SessionInterceptor, SessionMessage, SessionType, SlimChannelSender,
};
use crate::streaming::{self, StreamingConfiguration};
use crate::{ServiceError, fire_and_forget, session};
use slim_datapath::Status;
use slim_datapath::api::proto::pubsub::v1::Message;
use slim_datapath::api::proto::pubsub::v1::SessionHeaderType;
use slim_datapath::messages::encoder::Agent;

/// SessionLayer
struct SessionLayer<P = Simple, V = Simple>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: AsyncRwLock<HashMap<Id, Session<P, V>>>,

    /// Name of the local agent
    agent_name: Agent,

    /// Identity provider for the local agent
    identity_provider: P,

    /// Identity verifier
    identity_verifier: V,

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

pub struct App<P = Simple, V = Simple>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    session_layer: Arc<SessionLayer<P, V>>,
}

impl<P, V> std::fmt::Debug for App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionPool")
    }
}

impl<P, V> App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new session pool
    pub(crate) fn new(
        agent_name: &Agent,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
    ) -> Self {
        // Create default configurations
        let default_ff_conf = SyncRwLock::new(FireAndForgetConfiguration::default());
        let default_rr_conf = SyncRwLock::new(RequestResponseConfiguration::default());
        let default_stream_conf = SyncRwLock::new(StreamingConfiguration::default());

        // Create the session layer
        let session_layer = Arc::new(SessionLayer {
            pool: AsyncRwLock::new(HashMap::new()),
            agent_name: agent_name.clone(),
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            default_ff_conf,
            default_rr_conf,
            default_stream_conf,
        });

        Self { session_layer }
    }

    pub async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<Info, SessionError> {
        self.session_layer.create_session(session_config, id).await
    }

    pub async fn delete_session(&self, id: Id) -> bool {
        self.session_layer.remove_session(id).await
    }

    /// Set config for a session
    pub async fn set_session_config(
        &self,
        session_config: &session::SessionConfig,
        session_id: Option<session::Id>,
    ) -> Result<(), SessionError> {
        // set the session config
        self.session_layer
            .set_session_config(session_config, session_id)
            .await
    }

    /// Get config for a session
    pub async fn get_session_config(
        &self,
        session_id: session::Id,
    ) -> Result<session::SessionConfig, SessionError> {
        // get the session config
        self.session_layer.get_session_config(session_id).await
    }

    /// Get default session config
    pub async fn get_default_session_config(
        &self,
        session_type: session::SessionType,
    ) -> Result<session::SessionConfig, SessionError> {
        // get the default session config
        self.session_layer
            .get_default_session_config(session_type)
            .await
    }

    /// Add an interceptor to a session
    pub async fn add_interceptor(
        &self,
        session_id: session::Id,
        interceptor: Box<dyn session::SessionInterceptor + Send + Sync>,
    ) -> Result<(), SessionError> {
        self.session_layer
            .add_session_interceptor(session_id, interceptor)
            .await
    }

    async fn send_message(
        &self,
        msg: Message,
        info: Option<session::Info>,
    ) -> Result<(), ServiceError> {
        // save session id for later use
        match info {
            Some(info) => {
                let id = info.id;
                self.session_layer
                    .handle_message(SessionMessage::from((msg, info)), MessageDirection::South)
                    .await
                    .map_err(|e| {
                        error!("error sending the message to session {}: {}", id, e);
                        ServiceError::SessionError(e.to_string())
                    })
            }
            None => self
                .session_layer
                .tx_slim()
                .send(Ok(msg))
                .await
                .map_err(|e| {
                    error!("error sending message {}", e);
                    ServiceError::MessageSendingError(e.to_string())
                }),
        }
    }

    pub async fn invite(
        &self,
        destination: &AgentType,
        session_info: session::Info,
    ) -> Result<(), ServiceError> {
        let slim_header = Some(SlimHeader::new(
            self.session_layer.agent_name(),
            destination,
            None,
            None,
        ));

        let session_header = Some(SessionHeader::new(
            SessionHeaderType::ChannelDiscoveryRequest.into(),
            session_info.id,
            rand::random::<u32>(),
        ));

        let payload = match bincode::encode_to_vec(
            self.session_layer.agent_name(),
            bincode::config::standard(),
        ) {
            Ok(payload) => payload,
            Err(_) => {
                return Err(ServiceError::PublishError(
                    "error while parsing the payload".to_string(),
                ));
            }
        };

        let msg = Message::new_publish_with_headers(slim_header, session_header, "", payload);

        self.send_message(msg, Some(session_info)).await
    }

    pub async fn subscribe(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: Option<u64>,
    ) -> Result<(), ServiceError> {
        debug!("subscribe to {}/{:?}", agent_type, agent_id);

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };
        let msg = Message::new_subscribe(
            self.session_layer.agent_name(),
            agent_type,
            agent_id,
            header,
        );
        self.send_message(msg, None).await
    }

    pub async fn unsubscribe(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: Option<u64>,
    ) -> Result<(), ServiceError> {
        debug!("unsubscribe from {}/{:?}", agent_type, agent_id);

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };
        let msg = Message::new_subscribe(
            self.session_layer.agent_name(),
            agent_type,
            agent_id,
            header,
        );
        self.send_message(msg, None).await
    }

    pub async fn set_route(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        debug!("set route to {}/{:?}", agent_type, agent_id);

        // send a message with subscription from
        let msg = Message::new_subscribe(
            self.session_layer.agent_name(),
            agent_type,
            agent_id,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(msg, None).await
    }

    pub async fn remove_route(
        &self,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), ServiceError> {
        debug!("unset route to {}/{:?}", agent_type, agent_id);

        //  send a message with unsubscription from
        let msg = Message::new_unsubscribe(
            self.session_layer.agent_name(),
            agent_type,
            agent_id,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(msg, None).await
    }

    pub async fn publish_to(
        &self,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        forward_to: u64,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            session_info,
            agent_type,
            agent_id,
            SlimHeaderFlags::default().with_forward_to(forward_to),
            blob,
        )
        .await
    }

    pub async fn publish(
        &self,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            session_info,
            agent_type,
            agent_id,
            SlimHeaderFlags::default(),
            blob,
        )
        .await
    }

    pub async fn publish_with_flags(
        &self,
        session_info: session::Info,
        agent_type: &AgentType,
        agent_id: Option<u64>,
        flags: SlimHeaderFlags,
        blob: Vec<u8>,
    ) -> Result<(), ServiceError> {
        debug!(
            "sending publication to {}/{:?}. Flags: {}",
            agent_type, agent_id, flags
        );

        let msg = Message::new_publish(
            self.session_layer.agent_name(),
            agent_type,
            agent_id,
            Some(flags),
            "msg",
            blob,
        );

        self.send_message(msg, Some(session_info)).await
    }

    /// SLIM receiver loop
    pub(crate) fn process_messages(
        &self,
        mut rx: mpsc::Receiver<Result<Message, Status>>,
        watch: Watch,
    ) {
        let agent_name = self.session_layer.agent_name.clone();
        let session_layer = self.session_layer.clone();

        tokio::spawn(async move {
            debug!("starting message processing loop for agent {}", agent_name);

            // subscribe for local agent running this loop
            let subscribe_msg = Message::new_subscribe(
                &agent_name,
                agent_name.agent_type(),
                Some(agent_name.agent_id()),
                None,
            );
            let tx = session_layer.tx_slim();
            tx.send(Ok(subscribe_msg))
                .await
                .expect("error sending subscription");

            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                debug!("no more messages to process");
                                break;
                            }
                            Some(msg) => {
                                match msg {
                                    Ok(msg) => {
                                        debug!("received message in service processing: {:?}", msg);

                                        // filter only the messages of type publish
                                        match msg.message_type.as_ref() {
                                            Some(MessageType::Publish(_)) => {},
                                            None => {
                                                continue;
                                            }
                                            _ => {
                                                continue;
                                            }
                                        }

                                        // Handle the message
                                        let res = session_layer
                                            .handle_message(SessionMessage::from(msg), MessageDirection::North)
                                            .await;

                                        if let Err(e) = res {
                                            error!("error handling message: {}", e);
                                        }
                                    }
                                    Err(e) => {
                                        error!("error: {}", e);

                                        // if internal error, forward it to application
                                        let tx_app = session_layer.tx_app();
                                        tx_app.send(Err(SessionError::Forward(e.to_string())))
                                            .await
                                            .expect("error sending error to application");
                                    }
                                }
                            }
                        }
                    }
                    _ = watch.clone().signaled() => {
                        debug!("shutting down processing on drain for agent: {}", session_layer.agent_name());
                        break;
                    }
                }
            }
        });
    }
}

impl<P, V> SessionLayer<P, V>
where
    P: TokenProvider + Send + Sync + Clone,
    V: Verifier + Send + Sync + Clone,
{
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
        let session = match session_config {
            SessionConfig::FireAndForget(conf) => {
                Session::FireAndForget(fire_and_forget::FireAndForget::new(
                    id,
                    conf,
                    SessionDirection::Bidirectional,
                    self.agent_name().clone(),
                    self.tx_slim(),
                    self.tx_app(),
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                ))
            }
            SessionConfig::RequestResponse(conf) => Session::RequestResponse(RequestResponse::new(
                id,
                conf,
                SessionDirection::Bidirectional,
                self.agent_name().clone(),
                self.tx_slim(),
                self.tx_app(),
                self.identity_provider.clone(),
                self.identity_verifier.clone(),
            )),
            SessionConfig::Streaming(conf) => {
                let direction = conf.direction.clone();

                Session::Streaming(streaming::Streaming::new(
                    id,
                    conf,
                    direction,
                    self.agent_name().clone(),
                    self.conn_id,
                    self.tx_slim(),
                    self.tx_app(),
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
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

            session.on_message_from_app_interceptors(&mut message.message)?;
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
        mut message: SessionMessage,
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
            session.on_message_from_slim_interceptors(&mut message.message)?;
            return session.on_message(message, direction).await;
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
            SessionHeaderType::ChannelDiscoveryRequest => {
                // TODO(micpapal/msardara): The discovery message should be handled directly here without creating the session yet
                // the session should be created on  SessionHeaderType::ChannelJoinRequest
                let mut conf = self.default_stream_conf.read().clone();
                conf.direction = SessionDirection::Bidirectional;
                self.create_session(session::SessionConfig::Streaming(conf), Some(id))
                    .await?
            }
            SessionHeaderType::ChannelDiscoveryReply
            | SessionHeaderType::ChannelJoinRequest
            | SessionHeaderType::ChannelJoinReply => {
                warn!("received channel message with unknown session id");
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
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
            session.on_message_from_slim_interceptors(&mut message.message)?;
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

    /// Add an interceptor to a session
    pub async fn add_session_interceptor(
        &self,
        session_id: Id,
        interceptor: Box<dyn SessionInterceptor + Send + Sync>,
    ) -> Result<(), SessionError> {
        let mut pool = self.pool.write().await;

        if let Some(session) = pool.get_mut(&session_id) {
            session.add_interceptor(interceptor);
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(session_id.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fire_and_forget::FireAndForgetConfiguration;

    use slim_datapath::{
        api::ProtoMessage,
        messages::{Agent, AgentType, utils::SLIM_IDENTITY},
    };

    fn create_app() -> App {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim,
            tx_app,
        )
    }

    #[tokio::test]
    async fn test_create_app() {
        let app = create_app();

        assert!(app.session_layer.pool.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let app = App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
        );
        let session_config = FireAndForgetConfiguration::default();

        let ret = app
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;

        assert!(ret.is_ok());

        let res = app.delete_session(1).await;
        assert!(res);
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
        );

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

        let session_layer = App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
        );

        let res = session_layer
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                Some(1),
            )
            .await;
        assert!(res.is_ok());

        let res = session_layer.delete_session(1).await;
        assert!(res);

        // try to delete a non-existing session
        let res = session_layer.delete_session(1).await;
        assert!(!res);
    }

    #[tokio::test]
    async fn test_handle_message_from_slim() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let app = App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
        );

        let session_config = FireAndForgetConfiguration::default();

        // create a new session
        let res = app
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

        let res = app
            .session_layer
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

    #[tokio::test]
    async fn test_handle_message_from_app() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let app = App::new(
            &agent,
            Simple::new("a"),
            Simple::new("a"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
        );

        let session_config = FireAndForgetConfiguration::default();

        // create a new session
        let res = app
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;
        assert!(res.is_ok());

        let source = Agent::from_strings("cisco", "default", "local_agent", 0);

        let mut message = ProtoMessage::new_publish(
            &source.clone(),
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

        let res = app
            .session_layer
            .handle_message(
                SessionMessage::from(message.clone()),
                MessageDirection::South,
            )
            .await;

        assert!(res.is_ok());

        // message should have been delivered to the app
        let mut msg = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        // add identity to message and reset the msg id msg to allow the comparison
        message.insert_metadata(SLIM_IDENTITY.to_owned(), agent.to_string());
        msg.set_message_id(0);
        assert_eq!(msg, message);
    }
}

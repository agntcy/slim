// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::channel_endpoint::handle_channel_discovery_message;
use crate::errors::SessionError;
use crate::fire_and_forget::FireAndForgetConfiguration;
use crate::interceptor::SessionInterceptor;
use crate::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
use crate::session::{
    AppChannelSender, CommonSession, Id, Info, MessageDirection, MessageHandler, SESSION_RANGE,
    Session, SessionConfig, SessionConfigTrait, SessionDirection, SessionMessage,
    SessionTransmitter, SessionType, SlimChannelSender,
};
use crate::streaming::{self, StreamingConfiguration};
use crate::transmitter::Transmitter;
use crate::{ServiceError, fire_and_forget, session};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::api::{MessageType, SessionHeader, SlimHeader};
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::AgentType;
use slim_datapath::messages::encoder::Agent;
use slim_datapath::messages::utils::SlimHeaderFlags;

use crate::interceptor_mls::METADATA_MLS_ENABLED;

/// SessionLayer
struct SessionLayer<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: AsyncRwLock<HashMap<Id, Session<P, V, T>>>,

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

    /// Transmitter for sessions
    transmitter: T,

    /// Default configuration for the session
    default_ff_conf: SyncRwLock<FireAndForgetConfiguration>,
    default_stream_conf: SyncRwLock<StreamingConfiguration>,

    /// Storage path for app data
    storage_path: std::path::PathBuf,
}

pub struct App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Session layer that manages sessions
    session_layer: Arc<SessionLayer<P, V, Transmitter>>,

    /// Cancelation token for the app receiver loop
    cancel_token: tokio_util::sync::CancellationToken,
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

impl<P, V> Drop for App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        // cancel the app receiver loop
        self.cancel_token.cancel();
    }
}

impl<P, V> App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create new App instance
    pub(crate) fn new(
        agent_name: &Agent,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
        storage_path: std::path::PathBuf,
    ) -> Self {
        // Create default configurations
        let default_ff_conf = SyncRwLock::new(FireAndForgetConfiguration::default());
        let default_stream_conf = SyncRwLock::new(StreamingConfiguration::default());

        // Create identity interceptor
        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            identity_provider.clone(),
            identity_verifier.clone(),
        ));

        // Create the transmitter
        let transmitter = Transmitter {
            slim_tx: tx_slim.clone(),
            app_tx: tx_app.clone(),
            interceptors: Arc::new(SyncRwLock::new(Vec::new())),
        };

        transmitter.add_interceptor(identity_interceptor);

        // Create the session layer
        let session_layer = Arc::new(SessionLayer {
            pool: AsyncRwLock::new(HashMap::new()),
            agent_name: agent_name.clone(),
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            default_ff_conf,
            default_stream_conf,
            storage_path,
        });

        // Create a new cancellation token for the app receiver loop
        let cancel_token = tokio_util::sync::CancellationToken::new();

        Self {
            session_layer,
            cancel_token,
        }
    }

    /// Create a new session with the given configuration
    pub async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<Info, SessionError> {
        let ret = self
            .session_layer
            .create_session(session_config, id)
            .await?;

        // return the session info
        Ok(ret)
    }

    /// Get a session by its ID
    pub async fn delete_session(&self, id: Id) -> Result<(), SessionError> {
        // remove the session from the pool
        if self.session_layer.remove_session(id).await {
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(id.to_string()))
        }
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

    /// Send a message to the session layer
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

    /// Invite a new participant to a session
    pub async fn invite_participant(
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
            session_info.get_session_type().into(),
            ProtoSessionMessageType::ChannelDiscoveryRequest.into(),
            session_info.id,
            rand::random::<u32>(),
        ));

        let msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        self.send_message(msg, Some(session_info)).await
    }

    /// Remove a participant from a session
    pub async fn remove_participant(
        &self,
        destination: &Agent,
        session_info: session::Info,
    ) -> Result<(), ServiceError> {
        let slim_header = Some(SlimHeader::new(
            self.session_layer.agent_name(),
            destination.agent_type(),
            destination.agent_id_option(),
            None,
        ));

        let session_header = Some(SessionHeader::new(
            ProtoSessionType::SessionUnknown.into(),
            ProtoSessionMessageType::ChannelLeaveRequest.into(),
            session_info.id,
            rand::random::<u32>(),
        ));

        let msg = Message::new_publish_with_headers(slim_header, session_header, "", vec![]);

        self.send_message(msg, Some(session_info)).await
    }

    /// Subscribe the app to receive messages for a name
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

    /// Unsubscribe the app
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

    /// Set a route towards another app
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

    /// Publish a message to a specific connection
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

    /// Publish a message to a specific app name
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

    /// Publish a message with specific flags
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
    pub(crate) fn process_messages(&self, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let agent_name = self.session_layer.agent_name.clone();
        let session_layer = self.session_layer.clone();
        let token_clone = self.cancel_token.clone();

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
                    _ = token_clone.cancelled() => {
                        debug!("message processing loop cancelled");
                        break;
                    }
                }
            }
        });
    }
}

impl<P, V, T> SessionLayer<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    pub(crate) fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

    pub(crate) fn tx_app(&self) -> AppChannelSender {
        self.tx_app.clone()
    }

    #[allow(dead_code)]
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

        // Create a new transmitter with identity interceptros
        let tx = self.transmitter.derive_new();

        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            self.identity_provider.clone(),
            self.identity_verifier.clone(),
        ));

        tx.add_interceptor(identity_interceptor);

        // create a new session
        let session = match session_config {
            SessionConfig::FireAndForget(conf) => {
                Session::FireAndForget(fire_and_forget::FireAndForget::new(
                    id,
                    conf,
                    SessionDirection::Bidirectional,
                    self.agent_name().clone(),
                    tx,
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                    self.storage_path.clone(),
                ))
            }
            SessionConfig::Streaming(conf) => {
                let direction = conf.direction.clone();

                Session::Streaming(streaming::Streaming::new(
                    id,
                    conf,
                    direction,
                    self.agent_name().clone(),
                    tx,
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                    self.storage_path.clone(),
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

        // Make sure the message is a publication
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

    /// Handle session from slim without creating a session
    /// return true is the message processing is done and no
    /// other action is needed, false otherwise
    async fn handle_message_from_slim_without_session(
        &self,
        message: &Message,
        session_type: ProtoSessionType,
        session_message_type: ProtoSessionMessageType,
        session_id: u32,
    ) -> Result<bool, SessionError> {
        match session_message_type {
            ProtoSessionMessageType::ChannelDiscoveryRequest => {
                // reply direcetly without creating any new Session
                let msg = handle_channel_discovery_message(
                    message,
                    self.agent_name(),
                    session_id,
                    session_type,
                );

                self.transmitter
                    .send_to_slim(Ok(msg))
                    .await
                    .map(|_| true)
                    .map_err(|e| {
                        SessionError::SlimTransmission(format!(
                            "error sending discovery reply: {}",
                            e
                        ))
                    })
            }
            _ => Ok(false),
        }
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    async fn handle_message_from_slim(
        &self,
        message: SessionMessage,
        direction: MessageDirection,
    ) -> Result<(), SessionError> {
        let (id, session_type, session_message_type) = {
            // get the session type and the session id from the message
            let header = message.message.get_session_header();

            // get the session type from the header
            let session_type = header.session_type();

            // get the session message type
            let session_message_type = header.session_message_type();

            // get the session ID
            let id = header.session_id;

            (id, session_type, session_message_type)
        };

        match self
            .handle_message_from_slim_without_session(
                &message.message,
                session_type,
                session_message_type,
                id,
            )
            .await
        {
            Ok(done) => {
                if done {
                    // message process concluded
                    return Ok(());
                }
            }
            Err(e) => {
                // return an error
                return Err(SessionError::SlimReception(format!(
                    "error processing packets from slim {}",
                    e
                )));
            }
        }

        if session_message_type == ProtoSessionMessageType::ChannelLeaveRequest {
            // send message to the session and delete it after
            if let Some(session) = self.pool.read().await.get(&id) {
                session.on_message(message, direction).await?;
            } else {
                warn!(
                    "received Channel Leave Request message with unknown session id, drop the message"
                );
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }
            // remove the session
            self.remove_session(id).await;
            return Ok(());
        }

        if let Some(session) = self.pool.read().await.get(&id) {
            // pass the message to the session
            return session.on_message(message, direction).await;
        }

        let new_session_id = match session_message_type {
            ProtoSessionMessageType::FnfMsg | ProtoSessionMessageType::FnfReliable => {
                let mut conf = self.default_ff_conf.read().clone();

                // Set that the session was initiated by another agent
                conf.initiator = false;

                // If other session is reliable, set the timeout
                if session_message_type == ProtoSessionMessageType::FnfReliable {
                    if conf.timeout.is_none() {
                        conf.timeout = Some(std::time::Duration::from_secs(5));
                    }

                    if conf.max_retries.is_none() {
                        conf.max_retries = Some(5);
                    }
                }

                self.create_session(SessionConfig::FireAndForget(conf), Some(id))
                    .await?
            }
            ProtoSessionMessageType::StreamMsg | ProtoSessionMessageType::BeaconStream => {
                let conf = self.default_stream_conf.read().clone();
                self.create_session(session::SessionConfig::Streaming(conf), Some(id))
                    .await?
            }
            ProtoSessionMessageType::ChannelJoinRequest => {
                // Create a new session based on the SessionType contained in the message
                match message.message.get_session_header().session_type() {
                    ProtoSessionType::SessionFireForget => {
                        let mut conf = self.default_ff_conf.read().clone();
                        conf.initiator = false;

                        if conf.timeout.is_none() {
                            conf.timeout = Some(std::time::Duration::from_secs(5));
                        }

                        if conf.max_retries.is_none() {
                            conf.max_retries = Some(5);
                        }

                        conf.mls_enabled = message.message.contains_metadata(METADATA_MLS_ENABLED);

                        self.create_session(SessionConfig::FireAndForget(conf), Some(id))
                            .await?
                    }
                    ProtoSessionType::SessionPubSub => {
                        let mut conf = self.default_stream_conf.read().clone();
                        conf.direction = SessionDirection::Bidirectional;
                        conf.mls_enabled = message.message.contains_metadata(METADATA_MLS_ENABLED);
                        self.create_session(SessionConfig::Streaming(conf), Some(id))
                            .await?
                    }
                    ProtoSessionType::SessionStreaming => {
                        let mut conf = self.default_stream_conf.read().clone();
                        conf.direction = SessionDirection::Receiver;
                        conf.mls_enabled = message.message.contains_metadata(METADATA_MLS_ENABLED);
                        self.create_session(SessionConfig::Streaming(conf), Some(id))
                            .await?
                    }
                    _ => {
                        warn!(
                            "received channel join request with unknown session type: {}",
                            session_type.as_str_name()
                        );
                        return Err(SessionError::SessionUnknown(
                            session_type.as_str_name().to_string(),
                        ));
                    }
                }
            }
            ProtoSessionMessageType::ChannelDiscoveryRequest
            | ProtoSessionMessageType::ChannelDiscoveryReply
            | ProtoSessionMessageType::ChannelJoinReply
            | ProtoSessionMessageType::ChannelLeaveRequest
            | ProtoSessionMessageType::ChannelLeaveReply
            | ProtoSessionMessageType::ChannelMlsCommit
            | ProtoSessionMessageType::ChannelMlsWelcome
            | ProtoSessionMessageType::ChannelMlsAck => {
                warn!("received channel message with unknown session id");
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }
            ProtoSessionMessageType::PubSubMsg => {
                warn!("received pub/sub message with unknown session id");
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }
            ProtoSessionMessageType::BeaconPubSub => {
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
            SessionType::Streaming => Ok(SessionConfig::Streaming(
                self.default_stream_conf.read().clone(),
            )),
        }
    }

    /// Add an interceptor to a session
    #[allow(dead_code)]
    pub async fn add_session_interceptor(
        &self,
        session_id: Id,
        interceptor: Arc<dyn SessionInterceptor + Send + Sync>,
    ) -> Result<(), SessionError> {
        let mut pool = self.pool.write().await;

        if let Some(session) = pool.get_mut(&session_id) {
            session.tx_ref().add_interceptor(interceptor);
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

    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::{
        api::ProtoMessage,
        messages::{Agent, AgentType, utils::SLIM_IDENTITY},
    };

    fn create_app() -> App<SharedSecret, SharedSecret> {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        App::new(
            &agent,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
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
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );
        let session_config = FireAndForgetConfiguration::default();

        let ret = app
            .create_session(SessionConfig::FireAndForget(session_config), Some(1))
            .await;

        assert!(ret.is_ok());

        app.delete_session(1).await.unwrap();
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let session_layer = App::new(
            &agent,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
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
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let res = session_layer
            .create_session(
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
                Some(1),
            )
            .await;
        assert!(res.is_ok());

        session_layer.delete_session(1).await.unwrap();

        // try to delete a non-existing session
        let res = session_layer.delete_session(1).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_handle_message_from_slim() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);
        let agent = Agent::from_strings("org", "ns", "type", 0);

        let identity = SharedSecret::new("a", "group");

        let app = App::new(
            &agent,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
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
        header.set_session_type(ProtoSessionType::SessionFireForget);
        header.set_session_message_type(ProtoSessionMessageType::FnfMsg);

        app.session_layer
            .handle_message(
                SessionMessage::from(message.clone()),
                MessageDirection::North,
            )
            .await
            .unwrap();

        // sleep to allow the message to be processed
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // As there is no identity, we should not get any message in the app
        rx_app
            .try_recv()
            .expect_err("message received when it should not have been");

        // Add identity to message
        message.insert_metadata(SLIM_IDENTITY.to_string(), identity.get_token().unwrap());

        // Try again
        app.session_layer
            .handle_message(
                SessionMessage::from(message.clone()),
                MessageDirection::North,
            )
            .await
            .unwrap();

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

        let identity = SharedSecret::new("a", "group");

        let app = App::new(
            &agent,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
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
        header.set_session_type(ProtoSessionType::SessionFireForget);
        header.set_session_message_type(ProtoSessionMessageType::FnfMsg);

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

        // Add identity to message
        message.insert_metadata(SLIM_IDENTITY.to_string(), identity.get_token().unwrap());

        msg.set_message_id(0);
        assert_eq!(msg, message);
    }
}

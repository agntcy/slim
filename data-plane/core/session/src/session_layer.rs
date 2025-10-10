// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use slim_datapath::messages::utils::SLIM_IDENTITY;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;

use crate::MessageHandler;
use crate::common::SessionMessage;
use crate::notification::Notification;
use crate::transmitter::{AppTransmitter, SessionTransmitter};

// Local crate
use super::context::SessionContext;
use super::interceptor::{IdentityInterceptor, SessionInterceptor};
use super::interceptor_mls::METADATA_MLS_ENABLED;
use super::multicast::{self, MulticastConfiguration};
use super::point_to_point::PointToPointConfiguration;
use super::{
    Id, MessageDirection, SESSION_RANGE, Session, SessionConfig, SessionConfigTrait, SessionType,
    SlimChannelSender, Transmitter,
};
use super::{SessionError, channel_endpoint::handle_channel_discovery_message};
use crate::interceptor::SessionInterceptorProvider; // needed for add_interceptor

/// SessionLayer manages sessions and their lifecycle
pub struct SessionLayer<P, V, T = AppTransmitter<P, V>>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: Arc<AsyncRwLock<HashMap<Id, Arc<Session<P, V>>>>>,

    /// Name of the local app
    app_name: Name,

    /// Identity provider for the local app
    identity_provider: P,

    /// Identity verifier
    identity_verifier: V,

    /// ID of the local connection
    conn_id: u64,

    /// Tx channels
    tx_slim: SlimChannelSender,
    tx_app: Sender<Result<Notification<P, V>, SessionError>>,

    // Transmitter to bypass sessions
    transmitter: T,

    /// Default configuration for the session
    default_p2p_conf: SyncRwLock<PointToPointConfiguration>,
    default_multicast_conf: SyncRwLock<MulticastConfiguration>,

    /// Storage path for app data
    storage_path: std::path::PathBuf,

    /// Channel to clone on session creation
    tx_session: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,
}

impl<P, V, T> SessionLayer<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Create a new SessionLayer
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app_name: Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: Sender<Result<Notification<P, V>, SessionError>>,
        transmitter: T,
        storage_path: std::path::PathBuf,
    ) -> Self {
        // Create default configurations
        let default_p2p_conf = SyncRwLock::new(PointToPointConfiguration::default());
        let default_multicast_conf = SyncRwLock::new(MulticastConfiguration::default());
        let (tx_session, rx_session) = tokio::sync::mpsc::channel(16);

        let sl = SessionLayer {
            pool: Arc::new(AsyncRwLock::new(HashMap::new())),
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            default_p2p_conf,
            default_multicast_conf,
            storage_path,
            tx_session,
        };

        sl.listen_from_sessions(rx_session);

        sl
    }

    pub fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

    pub fn tx_app(&self) -> Sender<Result<Notification<P, V>, SessionError>> {
        self.tx_app.clone()
    }

    #[allow(dead_code)]
    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }

    pub fn app_name(&self) -> &Name {
        &self.app_name
    }

    /// Get identity token from the identity provider
    pub fn get_identity_token(&self) -> Result<String, String> {
        self.identity_provider
            .get_token()
            .map_err(|e| e.to_string())
    }

    pub async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<SessionContext<P, V>, SessionError> {
        // TODO(msardara): the session identifier should be a combination of the
        // session ID and the app ID, to prevent collisions.

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

        // Create a new transmitter with identity interceptors
        let (app_tx, app_rx) = tokio::sync::mpsc::channel(128);
        let tx = SessionTransmitter::new(self.tx_slim.clone(), app_tx);

        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            self.identity_provider.clone(),
            self.identity_verifier.clone(),
        ));

        tx.add_interceptor(identity_interceptor);

        // create a new session
        let session = match session_config {
            SessionConfig::PointToPoint(conf) => Arc::new(Session::from_point_to_point(
                super::point_to_point::PointToPoint::new(
                    id,
                    conf,
                    self.app_name().clone(),
                    tx,
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                    self.storage_path.clone(),
                ),
            )),
            SessionConfig::Multicast(conf) => {
                Arc::new(Session::from_multicast(multicast::Multicast::new(
                    id,
                    conf,
                    self.app_name().clone(),
                    tx,
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                    self.storage_path.clone(),
                    self.tx_session.clone(),
                )))
            }
        };

        // insert the session into the pool
        let ret = pool.insert(id, session.clone());

        // This should never happen, but just in case
        if ret.is_some() {
            panic!("session already exists: {}", ret.is_some());
        }

        Ok(SessionContext::new(session, app_rx))
    }

    /// Remove a session from the pool
    pub async fn remove_session(&self, id: Id) -> bool {
        // get the write lock
        let mut pool = self.pool.write().await;
        pool.remove(&id).is_some()
    }

    pub fn listen_from_sessions(
        &self,
        mut rx_session: tokio::sync::mpsc::Receiver<Result<SessionMessage, SessionError>>,
    ) {
        let pool_clone = self.pool.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = rx_session.recv() => {
                        match next {
                            Some(Ok(SessionMessage::DeleteSession { session_id })) => {
                                debug!("received closing signal from session {}, cancel it from the pool", session_id);
                                let mut pool = pool_clone.write().await;
                                if pool.remove(&session_id).is_none() {
                                    warn!("requested to delete unknown session id {}", session_id);
                                }
                            }
                            Some(Ok(_)) => {
                                error!("received unexpected message");
                            }
                            Some(Err(e)) => {
                                warn!("error from session: {:?}", e);
                            }
                            None => {
                                // All senders dropped; exit loop.
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    pub async fn handle_message_from_app(
        &self,
        message: Message,
        context: &SessionContext<P, V>,
    ) -> Result<(), SessionError> {
        context
            .session()
            .upgrade()
            .ok_or(SessionError::SessionNotFound(0))?
            .publish_message(message)
            .await
    }

    /// Handle session from slim without creating a session
    /// return true is the message processing is done and no
    /// other action is needed, false otherwise
    pub(crate) async fn handle_message_from_slim_without_session(
        &self,
        message: &slim_datapath::api::ProtoMessage,
        session_type: ProtoSessionType,
        session_message_type: ProtoSessionMessageType,
        session_id: u32,
    ) -> Result<bool, SessionError> {
        match session_message_type {
            ProtoSessionMessageType::ChannelDiscoveryRequest => {
                // reply directly without creating any new Session
                let msg = handle_channel_discovery_message(
                    message,
                    self.app_name(),
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
    pub async fn handle_message_from_slim(&self, mut message: Message) -> Result<(), SessionError> {
        // Pass message to interceptors in the transmitter
        self.transmitter
            .on_msg_from_slim_interceptors(&mut message)
            .await?;

        tracing::trace!(
            "received message from SLIM {} {}",
            message.get_session_message_type().as_str_name(),
            message.get_id()
        );

        let (id, session_type, session_message_type) = {
            // get the session type and the session id from the message
            let header = message.get_session_header();

            // get the session type from the header
            let session_type = header.session_type();

            // get the session message type
            let session_message_type = header.session_message_type();

            // get the session ID
            let id = header.session_id;

            (id, session_type, session_message_type)
        };

        if session_message_type == ProtoSessionMessageType::ChannelDiscoveryRequest {
            // received a discovery message
            if let Some(session) = self.pool.read().await.get(&id)
                && session.session_config().initiator()
            {
                // if the message is for a session that already exists and the local app
                // is the initiator of the session this message is coming from the controller
                // that wants to add new participant to the session
                return session.on_message(message, MessageDirection::North).await;
            } else {
                // in this case we handle the message without creating a new local session
                match self
                    .handle_message_from_slim_without_session(
                        &message,
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
            }
        }

        if session_message_type == ProtoSessionMessageType::ChannelLeaveRequest {
            let mut drop_session = true;
            // send message to the session and delete it after
            if let Some(session) = self.pool.read().await.get(&id) {
                if message.get_session_type() == ProtoSessionType::SessionMulticast {
                    if let Some(string_name) = message.get_metadata("PARTICIPANT_NAME") {
                        debug!(
                            "received a Leave Request message on multicast session with PARTICIPANT_NAME"
                        );

                        let participant_vec = base64::engine::general_purpose::STANDARD
                            .decode(string_name)
                            .map_err(|e| SessionError::ParseProposalMessage(e.to_string()))?;

                        let participant: Name = bincode::decode_from_slice(
                            &participant_vec,
                            bincode::config::standard(),
                        )
                        .map_err(|e| SessionError::ParseProposalMessage(e.to_string()))?
                        .0;

                        if &participant == session.source() {
                            // the controller want to delete the session on the moderator.
                            // this is equivalent to delete the full group.
                            // TODO (micpapal/msardara): move the moderator role
                            // to another participant and keep the group alive
                            message.remove_metadata("PARTICIPANT_NAME");
                            message.insert_metadata("DELETE_GROUP".to_string(), "true".to_string());

                            debug!("try to remove the moderator, close the session");
                        }

                        drop_session = false;
                    } else if message.contains_metadata("DELETE_GROUP") {
                        debug!(
                            "received a Leave Request message on multicast session with DELETE GROUP"
                        );
                        drop_session = false;
                    }
                }
                session.on_message(message, MessageDirection::North).await?;
            } else {
                warn!(
                    "received Channel Leave Request message with unknown session id, drop the message"
                );
                return Err(SessionError::SessionUnknown(
                    session_type.as_str_name().to_string(),
                ));
            }

            if drop_session {
                // remove the session
                self.remove_session(id).await;
            }
            return Ok(());
        }

        if let Some(session) = self.pool.read().await.get(&id) {
            // pass the message to the session
            return session.on_message(message, MessageDirection::North).await;
        }

        let new_session = match session_message_type {
            ProtoSessionMessageType::P2PMsg | ProtoSessionMessageType::P2PReliable => {
                let mut conf = self.default_p2p_conf.read().clone();

                // Set that the session was initiated by another app
                conf.initiator = false;

                // If other session is reliable, set the timeout
                if session_message_type == ProtoSessionMessageType::P2PReliable {
                    if conf.timeout.is_none() {
                        conf.timeout = Some(std::time::Duration::from_secs(5));
                    }

                    if conf.max_retries.is_none() {
                        conf.max_retries = Some(5);
                    }
                }

                self.create_session(SessionConfig::PointToPoint(conf), Some(id))
                    .await?
            }
            ProtoSessionMessageType::ChannelJoinRequest => {
                // Create a new session based on the SessionType contained in the message
                match message.get_session_header().session_type() {
                    ProtoSessionType::SessionPointToPoint => {
                        let mut conf = self.default_p2p_conf.read().clone();
                        conf.initiator = false;

                        // TODO (micpapal): this timer should be part of the session context
                        // to be added in the JoinRequest
                        if conf.timeout.is_none() {
                            conf.timeout = Some(std::time::Duration::from_secs(5));
                        }

                        if conf.max_retries.is_none() {
                            conf.max_retries = Some(5);
                        }

                        conf.peer_name = Some(message.get_source());
                        conf.mls_enabled = message.contains_metadata(METADATA_MLS_ENABLED);
                        conf.metadata = message.get_metadata_map();

                        self.create_session(SessionConfig::PointToPoint(conf), Some(id))
                            .await?
                    }
                    ProtoSessionType::SessionMulticast => {
                        let mut conf = self.default_multicast_conf.read().clone();
                        conf.mls_enabled = message.contains_metadata(METADATA_MLS_ENABLED);

                        // the metadata of the first received message are copied in the metadata of the session
                        // and then added to the messages sent by this session. so we need to erase the entries
                        // the we want to keep local: IS_MODERATOR and SLIM_IDENTITY
                        conf.initiator = message.remove_metadata("IS_MODERATOR").is_some();
                        message.remove_metadata(SLIM_IDENTITY);

                        conf.metadata = message.get_metadata_map();

                        conf.channel_name = message
                            .get_session_header()
                            .get_destination()
                            .ok_or(SessionError::MissingChannelName)?;

                        self.create_session(SessionConfig::Multicast(conf), Some(id))
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
            | ProtoSessionMessageType::ChannelMlsAck
            | ProtoSessionMessageType::P2PAck
            | ProtoSessionMessageType::RtxRequest
            | ProtoSessionMessageType::RtxReply
            | ProtoSessionMessageType::MulticastMsg
            | ProtoSessionMessageType::BeaconMulticast => {
                debug!(
                    "received channel message with unknown session id {:?} ",
                    message
                );
                // We can ignore these messages
                return Ok(());
            }
            _ => {
                return Err(SessionError::SessionUnknown(
                    session_message_type.as_str_name().to_string(),
                ));
            }
        };

        debug_assert!(new_session.session().upgrade().unwrap().id() == id);

        // process the message
        new_session
            .session()
            .upgrade()
            .ok_or(SessionError::SessionClosed(
                "newly created session already closed: this should not happen".to_string(),
            ))?
            .on_message(message, MessageDirection::North)
            .await?;

        // send new session to the app
        self.tx_app
            .send(Ok(Notification::NewSession(new_session)))
            .await
            .map_err(|e| SessionError::AppTransmission(format!("error sending new session: {}", e)))
    }

    /// Set the configuration of a session
    pub fn set_default_session_config(
        &self,
        session_config: &SessionConfig,
    ) -> Result<(), SessionError> {
        // If no session ID is provided, modify the default session
        match session_config {
            SessionConfig::PointToPoint(_) => self.default_p2p_conf.write().replace(session_config),
            SessionConfig::Multicast(_) => {
                self.default_multicast_conf.write().replace(session_config)
            }
        }
    }

    /// Get the session configuration
    pub fn get_default_session_config(
        &self,
        session_type: SessionType,
    ) -> Result<SessionConfig, SessionError> {
        match session_type {
            SessionType::PointToPoint => Ok(SessionConfig::PointToPoint(
                self.default_p2p_conf.read().clone(),
            )),
            SessionType::Multicast => Ok(SessionConfig::Multicast(
                self.default_multicast_conf.read().clone(),
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
            Err(SessionError::SessionNotFound(session_id))
        }
    }

    /// Check if the session pool is empty (for testing purposes)
    #[cfg(test)]
    pub async fn is_pool_empty(&self) -> bool {
        self.pool.read().await.is_empty()
    }
}

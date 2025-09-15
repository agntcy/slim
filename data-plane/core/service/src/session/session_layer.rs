// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use tokio::sync::RwLock as AsyncRwLock;
use tracing::{debug, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;

// Local crate
use super::interceptor::{IdentityInterceptor, SessionInterceptor};
use super::interceptor_mls::METADATA_MLS_ENABLED;
use super::multicast::{self, MulticastConfiguration};
use super::point_to_point::PointToPointConfiguration;
use super::{
    AppChannelSender, CommonSession, Id, Info, MessageDirection, MessageHandler, SESSION_RANGE,
    Session, SessionConfig, SessionConfigTrait, SessionMessage, SessionTransmitter, SessionType,
    SlimChannelSender,
};
use super::{SessionError, channel_endpoint::handle_channel_discovery_message};

/// SessionLayer manages sessions and their lifecycle
pub(crate) struct SessionLayer<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: AsyncRwLock<HashMap<Id, Session<P, V, T>>>,

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
    tx_app: AppChannelSender,

    /// Transmitter for sessions
    transmitter: T,

    /// Default configuration for the session
    default_ff_conf: SyncRwLock<PointToPointConfiguration>,
    default_multicast_conf: SyncRwLock<MulticastConfiguration>,

    /// Storage path for app data
    storage_path: std::path::PathBuf,
}

impl<P, V, T> SessionLayer<P, V, T>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: SessionTransmitter + Send + Sync + Clone + 'static,
{
    /// Create a new SessionLayer
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app_name: Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
        transmitter: T,
        storage_path: std::path::PathBuf,
    ) -> Self {
        // Create default configurations
        let default_ff_conf = SyncRwLock::new(PointToPointConfiguration::default());
        let default_multicast_conf = SyncRwLock::new(MulticastConfiguration::default());

        Self {
            pool: AsyncRwLock::new(HashMap::new()),
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            default_ff_conf,
            default_multicast_conf,
            storage_path,
        }
    }

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

    pub(crate) fn app_name(&self) -> &Name {
        &self.app_name
    }

    /// Get identity token from the identity provider
    pub(crate) fn get_identity_token(&self) -> Result<String, String> {
        self.identity_provider
            .get_token()
            .map_err(|e| e.to_string())
    }

    pub(crate) async fn create_session(
        &self,
        session_config: SessionConfig,
        id: Option<Id>,
    ) -> Result<Info, SessionError> {
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
        let tx = self.transmitter.derive_new();

        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            self.identity_provider.clone(),
            self.identity_verifier.clone(),
        ));

        tx.add_interceptor(identity_interceptor);

        // create a new session
        let session = match session_config {
            SessionConfig::PointToPoint(conf) => {
                Session::PointToPoint(super::point_to_point::PointToPoint::new(
                    id,
                    conf,
                    self.app_name().clone(),
                    tx,
                    self.identity_provider.clone(),
                    self.identity_verifier.clone(),
                    self.storage_path.clone(),
                ))
            }
            SessionConfig::Multicast(conf) => Session::Multicast(multicast::Multicast::new(
                id,
                conf,
                self.app_name().clone(),
                tx,
                self.identity_provider.clone(),
                self.identity_verifier.clone(),
                self.storage_path.clone(),
            )),
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
            ProtoSessionMessageType::P2PMsg | ProtoSessionMessageType::P2PReliable => {
                let mut conf = self.default_ff_conf.read().clone();

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
                match message.message.get_session_header().session_type() {
                    ProtoSessionType::SessionPointToPoint => {
                        let mut conf = self.default_ff_conf.read().clone();
                        conf.initiator = false;

                        if conf.timeout.is_none() {
                            conf.timeout = Some(std::time::Duration::from_secs(5));
                        }

                        if conf.max_retries.is_none() {
                            conf.max_retries = Some(5);
                        }

                        conf.mls_enabled = message.message.contains_metadata(METADATA_MLS_ENABLED);

                        self.create_session(SessionConfig::PointToPoint(conf), Some(id))
                            .await?
                    }
                    ProtoSessionType::SessionMulticast => {
                        let mut conf = self.default_multicast_conf.read().clone();
                        conf.mls_enabled = message.message.contains_metadata(METADATA_MLS_ENABLED);
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
                debug!("received channel message with unknown session id");
                // We can ignore these messages
                return Ok(());
            }
            _ => {
                return Err(SessionError::SessionUnknown(
                    session_message_type.as_str_name().to_string(),
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
                    SessionConfig::PointToPoint(_) => {
                        return self.default_ff_conf.write().replace(session_config);
                    }
                    SessionConfig::Multicast(_) => {
                        return self.default_multicast_conf.write().replace(session_config);
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
            SessionType::PointToPoint => Ok(SessionConfig::PointToPoint(
                self.default_ff_conf.read().clone(),
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
            Err(SessionError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Check if the session pool is empty (for testing purposes)
    #[cfg(test)]
    pub(crate) async fn is_pool_empty(&self) -> bool {
        self.pool.read().await.is_empty()
    }
}

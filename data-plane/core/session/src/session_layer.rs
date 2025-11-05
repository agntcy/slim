// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use slim_datapath::messages::utils::IS_MODERATOR;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;

use crate::common::SessionMessage;
use crate::notification::Notification;
use crate::session_config::SessionConfig;
use crate::session_controller::SessionController;

use crate::transmitter::{AppTransmitter, SessionTransmitter};

// Local crate
use super::context::SessionContext;
use super::interceptor::IdentityInterceptor;
use super::{MessageDirection, SESSION_RANGE, SlimChannelSender, Transmitter};
use super::{SessionError, session_controller::handle_channel_discovery_message};
use crate::interceptor::SessionInterceptorProvider; // needed for add_interceptor

/// SessionLayer manages sessions and their lifecycle
pub struct SessionLayer<P, V, T = AppTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: Arc<AsyncRwLock<HashMap<u32, Arc<SessionController>>>>,

    /// Default name of the local app
    app_id: u64,

    /// Names registered by local app
    app_names: SyncRwLock<HashSet<Name>>,

    /// Identity provider for the local app
    identity_provider: P,

    /// Identity verifier
    identity_verifier: V,

    /// ID of the local connection
    conn_id: u64,

    /// Tx channels
    tx_slim: SlimChannelSender,
    tx_app: Sender<Result<Notification, SessionError>>,

    // Transmitter to bypass sessions
    transmitter: T,

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
        tx_app: Sender<Result<Notification, SessionError>>,
        transmitter: T,
        storage_path: std::path::PathBuf,
    ) -> Self {
        let (tx_session, rx_session) = tokio::sync::mpsc::channel(16);

        let sl = SessionLayer {
            pool: Arc::new(AsyncRwLock::new(HashMap::new())),
            app_id: app_name.id(),
            app_names: SyncRwLock::new(HashSet::from([app_name.with_id(Name::NULL_COMPONENT)])),
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            storage_path,
            tx_session,
        };

        sl.listen_from_sessions(rx_session);

        sl
    }

    pub fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

    pub fn tx_app(&self) -> Sender<Result<Notification, SessionError>> {
        self.tx_app.clone()
    }

    #[allow(dead_code)]
    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }

    pub fn app_id(&self) -> u64 {
        self.app_id
    }

    pub fn add_app_name(&self, name: Name) {
        // unset last component for fast lookups
        self.app_names
            .write()
            .insert(name.with_id(Name::NULL_COMPONENT));
    }

    pub fn remove_app_name(&self, name: &Name) {
        let removed = match name.id() {
            Name::NULL_COMPONENT => self.app_names.write().remove(name),
            _ => {
                let name = name.clone().with_id(Name::NULL_COMPONENT);
                self.app_names.write().remove(&name)
            }
        };

        if !removed {
            warn!("tried to remove unknown app name {}", name);
        }
    }

    fn get_local_name_for_session(&self, dst: Name) -> Result<Name, SessionError> {
        let name = dst.with_id(Name::NULL_COMPONENT);

        self.app_names
            .read()
            .get(&name)
            .cloned()
            .map(|n| n.with_id(self.app_id))
            .ok_or(SessionError::SubscriptionNotFound(name.to_string()))
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
        local_name: Name,
        destination: Name,
        id: Option<u32>,
    ) -> Result<SessionContext, SessionError> {
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
        let (app_tx, app_rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = SessionTransmitter::new(self.tx_slim.clone(), app_tx);

        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            self.identity_provider.clone(),
            self.identity_verifier.clone(),
        ));

        tx.add_interceptor(identity_interceptor);

        let session_controller = Arc::new(
            SessionController::builder()
                .with_id(id)
                .with_source(local_name)
                .with_destination(destination)
                .with_config(session_config)
                .with_identity_provider(self.identity_provider.clone())
                .with_identity_verifier(self.identity_verifier.clone())
                .with_storage_path(self.storage_path.clone())
                .with_tx(tx)
                .with_tx_to_session_layer(self.tx_session.clone())
                .with_cancellation_token(tokio_util::sync::CancellationToken::new())
                .ready()?
                .build()
                .await?,
        );

        let ret = pool.insert(id, session_controller.clone());

        // This should never happen, but just in case
        if ret.is_some() {
            panic!("session already exists: {}", ret.is_some());
        }

        Ok(SessionContext::new(session_controller, app_rx))
    }

    /// Remove a session from the pool
    pub async fn remove_session(&self, id: u32) -> bool {
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
        context: &SessionContext,
    ) -> Result<(), SessionError> {
        context
            .session()
            .upgrade()
            .ok_or(SessionError::SessionNotFound(0))?
            .on_message(message, MessageDirection::South)
            .await
    }

    /// Handle session from slim without creating a session
    /// return true is the message processing is done and no
    /// other action is needed, false otherwise
    pub(crate) async fn handle_message_from_slim_without_session(
        &self,
        local_name: &Name,
        message: &slim_datapath::api::ProtoMessage,
        session_type: ProtoSessionType,
        session_message_type: ProtoSessionMessageType,
        session_id: u32,
    ) -> Result<(), SessionError> {
        match session_message_type {
            ProtoSessionMessageType::DiscoveryRequest => {
                // reply directly without creating any new Session
                let msg =
                    handle_channel_discovery_message(message, local_name, session_id, session_type);

                self.transmitter.send_to_slim(Ok(msg)).await.map_err(|e| {
                    SessionError::SlimTransmission(format!("error sending discovery reply: {}", e))
                })
            }
            _ => Err(SessionError::SlimTransmission(
                "unexpected message type".to_string(),
            )),
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

        // special handling for discovery request messages
        if session_message_type == ProtoSessionMessageType::DiscoveryRequest {
            return self
                .handle_discovery_request(message, id, session_type, session_message_type)
                .await;
        }

        // check if we have a session for the given session ID
        if let Some(controller) = self.pool.read().await.get(&id) {
            // pass the message to the session
            return controller
                .on_message(message, MessageDirection::North)
                .await;
        }

        // get local name for the session
        let local_name = self.get_local_name_for_session(message.get_slim_header().get_dst())?;

        let new_session = match session_message_type {
            ProtoSessionMessageType::JoinRequest => {
                // Create a new session based on the message payload
                let payload = message
                    .get_payload()
                    .ok_or(SessionError::Processing(
                        "missing join request payload".to_string(),
                    ))?
                    .as_command_payload()?
                    .as_join_request_payload()?;

                match message.get_session_header().session_type() {
                    ProtoSessionType::PointToPoint => {
                        let conf = crate::SessionConfig::from_join_request(
                            ProtoSessionType::PointToPoint,
                            message.get_payload().unwrap().as_command_payload()?,
                            message.get_metadata_map(),
                            false,
                        )?;

                        self.create_session(
                            conf,
                            local_name,
                            message.get_source(),
                            Some(message.get_session_header().session_id),
                        )
                        .await?
                    }
                    ProtoSessionType::Multicast => {
                        // Multicast sessions require timer settings; reject if missing.
                        if payload.timer_settings.is_none() {
                            return Err(SessionError::Processing(
                                "missing timer options".to_string(),
                            ));
                        }

                        // Determine initiator (moderator) before building metadata snapshot.
                        let initiator = message.remove_metadata(IS_MODERATOR).is_some();

                        let conf = crate::SessionConfig::from_join_request(
                            ProtoSessionType::Multicast,
                            message.get_payload().unwrap().as_command_payload()?,
                            message.get_metadata_map(),
                            initiator,
                        )?;

                        let channel = if let Some(c) = payload.channel {
                            Name::from(&c)
                        } else {
                            return Err(SessionError::Processing(
                                "missing channel name".to_string(),
                            ));
                        };

                        self.create_session(
                            conf,
                            local_name,
                            channel,
                            Some(message.get_session_header().session_id),
                        )
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
            ProtoSessionMessageType::DiscoveryRequest
            | ProtoSessionMessageType::DiscoveryReply
            | ProtoSessionMessageType::JoinReply
            | ProtoSessionMessageType::LeaveRequest
            | ProtoSessionMessageType::LeaveReply
            | ProtoSessionMessageType::GroupAdd
            | ProtoSessionMessageType::GroupRemove
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupAck
            | ProtoSessionMessageType::Msg
            | ProtoSessionMessageType::MsgAck
            | ProtoSessionMessageType::RtxRequest
            | ProtoSessionMessageType::RtxReply => {
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

    /// Handle a discovery request message.
    async fn handle_discovery_request(
        &self,
        message: Message,
        id: u32,
        session_type: ProtoSessionType,
        session_message_type: ProtoSessionMessageType,
    ) -> Result<(), SessionError> {
        // received a discovery message
        if let Some(controller) = self.pool.read().await.get(&id)
            && controller.is_initiator()
            && message
                .get_payload()
                .unwrap()
                .as_command_payload()
                .ok()
                .and_then(|p| p.as_discovery_request_payload().ok())
                .and_then(|p| p.destination)
                .is_some()
        {
            // Existing session where we are the initiator and payload includes a destination name:
            // controller is requesting to add a new participant to this session.
            controller
                .on_message(message, MessageDirection::North)
                .await
        } else {
            // Handle the discovery request without creating a local session.
            let local_name =
                self.get_local_name_for_session(message.get_slim_header().get_dst())?;

            self.handle_message_from_slim_without_session(
                &local_name,
                &message,
                session_type,
                session_message_type,
                id,
            )
            .await
        }
    }

    /// Check if the session pool is empty (for testing purposes)
    #[cfg(test)]
    pub async fn is_pool_empty(&self) -> bool {
        self.pool.read().await.is_empty()
    }
}

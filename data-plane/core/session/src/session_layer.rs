// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;

use crate::common::SessionMessage;
use crate::controller_sender::ControllerSender;
use crate::notification::Notification;
use crate::session_controller::{self, SessionController, SessionModerator, SessionParticipant};
use crate::session_receiver::SessionReceiver;
use crate::session_sender::SessionSender;
use crate::timer;
use crate::timer_factory::TimerSettings;
use crate::transmitter::{AppTransmitter, SessionTransmitter};

// Local crate
use super::context::SessionContext;
use super::interceptor::{IdentityInterceptor, SessionInterceptor};
use super::multicast::{self};
use super::{MessageDirection, SESSION_RANGE, SlimChannelSender, Transmitter};
use super::{SessionError, session_controller::handle_channel_discovery_message};
use crate::interceptor::SessionInterceptorProvider; // needed for add_interceptor

/// SessionLayer manages sessions and their lifecycle
pub struct SessionLayer<P, V, T = AppTransmitter<P, V>>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: Arc<AsyncRwLock<HashMap<u32, Arc<SessionController<P, V>>>>>,

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
    tx_app: Sender<Result<Notification<P, V>, SessionError>>,

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
        tx_app: Sender<Result<Notification<P, V>, SessionError>>,
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

    pub fn tx_app(&self) -> Sender<Result<Notification<P, V>, SessionError>> {
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
        session_config: crate::session_controller::SessionConfig,
        session_type: ProtoSessionType,
        local_name: Name,
        destination: Name,
        id: Option<u32>,
        conn: Option<u64>,
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

        let session_controller = Arc::new(SessionController::new(
            id,
            session_type,
            local_name,
            destination,
            conn,
            session_config,
            self.identity_provider.clone(),
            self.identity_verifier.clone(),
            self.storage_path.clone(),
            tx,
            self.tx_session.clone(),
        ));

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
        context: &SessionContext<P, V>,
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
            _ => Err(SessionError::SlimTransmission(format!(
                "unexpected message type",
            ))),
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

        if session_message_type == ProtoSessionMessageType::DiscoveryRequest {
            // received a discovery message
            if let Some(controller) = self.pool.read().await.get(&id)
                && controller.is_initiator()
                && message
                    .get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_discovery_request_payload()
                    .destination
                    .is_some()
            {
                // if the message is for a session that already exists and the session
                // is the initiator of the session and the message contains a name in the payload
                // this message is coming from the controller that wants to add new participant to the session
                return controller
                    .on_message(message, MessageDirection::North)
                    .await;
            } else {
                // in this case we handle the message without creating a new local session
                let local_name =
                    self.get_local_name_for_session(message.get_slim_header().get_dst())?;

                return self
                    .handle_message_from_slim_without_session(
                        &local_name,
                        &message,
                        session_type,
                        session_message_type,
                        id,
                    )
                    .await;
            }
        }

        if let Some(controller) = self.pool.read().await.get(&id) {
            // pass the message to the session
            return controller
                .on_message(message, MessageDirection::North)
                .await;
        }

        // get local name for the session
        let local_name = self.get_local_name_for_session(message.get_slim_header().get_dst())?;

        let new_session = match session_message_type {
            // TODO: do we want to keep this option?
            ProtoSessionMessageType::Msg => {
                // if this is a point to point session create an anycast session otherwise drop the message
                if message.get_session_type() == ProtoSessionType::PointToPoint {
                    let conf = crate::session_controller::SessionConfig {
                        max_retries: None,
                        duration: None,
                        mls_enabled: false,
                        initiator: false,
                        metadata: message.get_metadata_map(),
                    };

                    self.create_session(
                        conf,
                        ProtoSessionType::PointToPoint,
                        local_name,
                        message.get_source(),
                        Some(message.get_session_header().session_id),
                        Some(message.get_incoming_conn()),
                    )
                    .await?
                } else {
                    debug!("received a multicast message for an unknown session, drop message");
                    return Ok(());
                }
            }
            ProtoSessionMessageType::JoinRequest => {
                // Create a new session based on the message payload
                let payload = message
                    .get_payload()
                    .unwrap()
                    .as_command_payload()
                    .as_join_request_payload();

                match message.get_session_header().session_type() {
                    ProtoSessionType::PointToPoint => {
                        let mut conf = crate::session_controller::SessionConfig::default();

                        match payload.timer_settings {
                            Some(s) => {
                                conf.duration =
                                    Some(std::time::Duration::from_millis(s.timeout() as u64));
                                conf.max_retries = Some(s.max_retries())
                            }
                            None => {
                                conf.duration = None;
                                conf.max_retries = None;
                            }
                        }

                        conf.mls_enabled = payload.enable_mls;
                        conf.initiator = false;
                        conf.metadata = message.get_metadata_map();

                        self.create_session(
                            conf,
                            ProtoSessionType::PointToPoint,
                            local_name,
                            message.get_source(),
                            Some(message.get_session_header().session_id),
                            Some(message.get_incoming_conn()),
                        )
                        .await?
                    }
                    ProtoSessionType::Multicast => {
                        let mut conf = crate::session_controller::SessionConfig::default();
                        let timer_settings = payload.timer_settings.ok_or(
                            SessionError::Processing("missing timer options".to_string()),
                        )?;

                        conf.duration = Some(std::time::Duration::from_millis(
                            timer_settings.timeout() as u64,
                        ));
                        conf.max_retries = timer_settings.max_retries;
                        conf.mls_enabled = payload.enable_mls;
                        conf.metadata = message.get_metadata_map();

                        // if the packet comes from the controller this app may be a moderator and so we need to
                        // tag it as session initiator
                        // in addition the metadata of the first received message are copied in the metadata of the session
                        // and then added to the messages sent by this session. so we need to erase the entries
                        // the we want to keep local: IS_MODERATOR
                        conf.initiator = message.remove_metadata("IS_MODERATOR").is_some();
                        conf.metadata = message.get_metadata_map();

                        self.create_session(
                            conf,
                            ProtoSessionType::PointToPoint,
                            local_name,
                            message.get_source(),
                            Some(message.get_session_header().session_id),
                            Some(message.get_incoming_conn()),
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
            | ProtoSessionMessageType::GroupUpdate
            | ProtoSessionMessageType::GroupWelcome
            | ProtoSessionMessageType::GroupAck
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

    /// Check if the session pool is empty (for testing purposes)
    #[cfg(test)]
    pub async fn is_pool_empty(&self) -> bool {
        self.pool.read().await.is_empty()
    }
}

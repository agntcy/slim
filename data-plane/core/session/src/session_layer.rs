// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;
use slim_datapath::messages::utils::IS_MODERATOR;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{ProtoMessage as Message, ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;

use crate::common::SessionMessage;
use crate::completion_handle::CompletionHandle;
use crate::notification::Notification;
use crate::session_config::SessionConfig;
use crate::session_controller::SessionController;

use crate::transmitter::{AppTransmitter, SessionTransmitter};

// Local crate
use super::context::SessionContext;
use super::interceptor::IdentityInterceptor;
use super::{SESSION_RANGE, SlimChannelSender};
use super::{SessionError, session_controller::handle_channel_discovery_message};
use crate::interceptor::SessionInterceptorProvider;
use crate::traits::Transmitter; // needed for add_interceptor

/// SessionLayer manages sessions and their lifecycle
pub struct SessionLayer<P, V, T = AppTransmitter>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
    T: Transmitter + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: Arc<SyncRwLock<HashMap<u32, Arc<SessionController>>>>,

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
            pool: Arc::new(SyncRwLock::new(HashMap::new())),
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

    /// Public interface to create a new session
    pub async fn create_session(
        &self,
        mut session_config: SessionConfig,
        local_name: Name,
        destination: Name,
        id: Option<u32>,
    ) -> Result<(SessionContext, CompletionHandle), SessionError> {
        // Sanity check
        session_config.initiator = true;

        // Store values before they are moved
        let is_p2p = session_config.session_type == ProtoSessionType::PointToPoint;
        let destination_clone = destination.clone();

        let session = self.create_session_internal(session_config, local_name, destination, id)?;

        // If session is p2p, initiate the discovery request now and return the ack
        // Otherwise, return an immediately resolved future
        let init_ack = if is_p2p {
            session
                .session()
                .upgrade()
                .ok_or_else(|| SessionError::SessionNotFound(session.session_id()))?
                .invite_participant_internal(&destination_clone)
                .await
                .inspect_err(|_| {
                    // If invite_participant_internal fails, remove the session from the pool
                    let _ = self.remove_session(session.session_id());
                })?
        } else {
            // For non-P2P sessions, return an immediately resolved future
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(Ok(()));
            CompletionHandle::from_oneshot_receiver(rx)
        };

        // return the session info and initialization ack
        Ok((session, init_ack))
    }

    /// Create a new session and add it to the pool
    fn create_session_internal(
        &self,
        session_config: SessionConfig,
        local_name: Name,
        destination: Name,
        id: Option<u32>,
    ) -> Result<SessionContext, SessionError> {
        // Retry loop to handle race conditions when generating random IDs
        loop {
            // get a lock on the session pool
            let session_id = {
                let pool = self.pool.read();

                // generate a new session ID in the SESSION_RANGE if not provided
                match id {
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
                            let session_id = rand::rng().random_range(SESSION_RANGE);
                            if !pool.contains_key(&session_id) {
                                break session_id;
                            }
                        }
                    }
                }
            }; // lock is dropped here

            // Create a new transmitter with identity interceptors
            let (app_tx, app_rx) = tokio::sync::mpsc::unbounded_channel();
            let tx = SessionTransmitter::new(self.tx_slim.clone(), app_tx);

            let identity_interceptor = Arc::new(IdentityInterceptor::new(
                self.identity_provider.clone(),
                self.identity_verifier.clone(),
            ));

            tx.add_interceptor(identity_interceptor);

            // Build the session controller (this is async, so no locks are held)
            let builder = SessionController::builder()
                .with_id(session_id)
                .with_source(local_name.clone())
                .with_destination(destination.clone())
                .with_config(session_config.clone())
                .with_identity_provider(self.identity_provider.clone())
                .with_identity_verifier(self.identity_verifier.clone())
                .with_storage_path(self.storage_path.clone())
                .with_tx(tx)
                .with_tx_to_session_layer(self.tx_session.clone())
                .ready()?;

            // Perform the async build operation without holding any lock
            let session_controller = Arc::new(builder.build()?);

            // Reacquire lock to insert the session
            let mut pool = self.pool.write();

            // Double-check that the ID wasn't taken while we didn't hold the lock
            if pool.contains_key(&session_id) {
                // If a specific ID was provided, return an error
                if id.is_some() {
                    return Err(SessionError::SessionIdAlreadyUsed(session_id.to_string()));
                }
                // If ID was randomly generated, retry with a new ID
                continue;
            }

            let ret = pool.insert(session_id, session_controller.clone());

            // This should never happen, but just in case
            if ret.is_some() {
                error!(
                    "session ID {} was taken during insertion: this should not happen",
                    session_id
                );
                return Err(SessionError::SessionIdAlreadyUsed(session_id.to_string()));
            }

            return Ok(SessionContext::new(session_controller, app_rx));
        }
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
                                if pool_clone.write().remove(&session_id).is_none() {
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

    /// Remove a session from the pool and return a handle to optionally wait on
    pub fn remove_session(&self, id: u32) -> Result<CompletionHandle, SessionError> {
        debug!("try to remove session {}", id);
        // get the read lock
        let binding = self.pool.read();
        let session = binding.get(&id).ok_or(SessionError::SessionNotFound(id))?;

        // close the session and get the join handle
        let join_handle = session.close()?;

        // Return a CompletionHandle wrapping the oneshot receiver
        Ok(CompletionHandle::from_join_handle(join_handle))
    }

    /// Clear all sessions and return completion handles to await on
    pub fn clear_all_sessions(&self) -> HashMap<u32, Result<CompletionHandle, SessionError>> {
        let pool = {
            let mut pool = self.pool.write();
            let copy = pool.clone();
            pool.clear();
            copy
        };

        // Close all sessions and return completion handles
        pool.iter()
            .map(|(id, session)| {
                let result = session.close().map(|join_handle| {
                    // Spawn a task to await the join handle and send the result through a oneshot channel
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        let result = join_handle.await.map(|_| ()).map_err(|e| {
                            SessionError::Processing(format!("Session cleanup failed: {}", e))
                        });
                        let _ = tx.send(result);
                    });
                    CompletionHandle::from_oneshot_receiver(rx)
                });
                (*id, result)
            })
            .collect()
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
                let msg = handle_channel_discovery_message(
                    message,
                    local_name,
                    session_id,
                    session_type,
                )?;

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
        let session_controller = self.pool.read().get(&id).cloned();
        if let Some(controller) = session_controller {
            // pass the message to the session
            return controller.on_message_from_slim(message).await;
        }

        // get local name for the session
        let local_name = self.get_local_name_for_session(message.get_slim_header().get_dst())?;

        let new_session = match session_message_type {
            ProtoSessionMessageType::JoinRequest => {
                match message.get_session_header().session_type() {
                    ProtoSessionType::PointToPoint => {
                        let conf = crate::SessionConfig::from_join_request(
                            ProtoSessionType::PointToPoint,
                            message.extract_command_payload().map_err(|e| {
                                SessionError::Processing(format!(
                                    "failed to extract command payload: {}",
                                    e
                                ))
                            })?,
                            message.get_metadata_map(),
                            false,
                        )?;

                        self.create_session_internal(
                            conf,
                            local_name,
                            message.get_source(),
                            Some(message.get_session_header().session_id),
                        )?
                    }
                    ProtoSessionType::Multicast => {
                        // Multicast sessions require timer settings; reject if missing.
                        let payload = message.extract_join_request().map_err(|e| {
                            SessionError::Processing(format!(
                                "failed to extract join request payload: {}",
                                e
                            ))
                        })?;

                        if payload.timer_settings.is_none() {
                            return Err(SessionError::Processing(
                                "missing timer options".to_string(),
                            ));
                        }

                        let channel = if let Some(c) = &payload.channel {
                            Name::from(c)
                        } else {
                            return Err(SessionError::Processing(
                                "missing channel name".to_string(),
                            ));
                        };

                        // Determine initiator (moderator) before building metadata snapshot.
                        let initiator = message.remove_metadata(IS_MODERATOR).is_some();
                        let conf = crate::SessionConfig::from_join_request(
                            ProtoSessionType::Multicast,
                            message.extract_command_payload().map_err(|e| {
                                SessionError::Processing(format!(
                                    "failed to extract command payload: {}",
                                    e
                                ))
                            })?,
                            message.get_metadata_map(),
                            initiator,
                        )?;

                        self.create_session_internal(
                            conf,
                            local_name,
                            channel,
                            Some(message.get_session_header().session_id),
                        )?
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
                tracing::debug!(
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
            .on_message_from_slim(message)
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
        let controller = self.pool.read().get(&id).cloned();
        if let Some(controller) = controller
            && controller.is_initiator()
            && message
                .extract_discovery_request()
                .ok()
                .and_then(|p| p.destination.as_ref())
                .is_some()
        {
            // Existing session where we are the initiator and payload includes a destination name:
            // controller is requesting to add a new participant to this session.
            controller.on_message_from_slim(message).await?;
            Ok(())
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
    pub fn is_pool_empty(&self) -> bool {
        self.pool.read().is_empty()
    }

    /// Get the number of sessions in the pool (for testing purposes)
    pub fn pool_size(&self) -> usize {
        self.pool.read().len()
    }

    /// Get a session from the pool (for testing purposes)
    pub fn get_session(&self, id: u32) -> Option<Arc<SessionController>> {
        self.pool.read().get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{MockTokenProvider, MockTransmitter, MockVerifier};
    use slim_datapath::Status;
    use slim_datapath::api::ProtoSessionType;
    use slim_datapath::messages::Name;
    use tokio::sync::mpsc;

    // --- Test Mocks -----------------------------------------------------------------------

    fn make_name(parts: &[&str; 3]) -> Name {
        Name::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    type TestSessionLayer = SessionLayer<MockTokenProvider, MockVerifier, MockTransmitter>;
    type SlimReceiver = mpsc::Receiver<Result<Message, Status>>;
    type AppReceiver = mpsc::Receiver<Result<Notification, SessionError>>;
    type TransmitterReceiver = mpsc::UnboundedReceiver<Result<Message, Status>>;

    fn setup_session_layer() -> (
        TestSessionLayer,
        SlimReceiver,
        AppReceiver,
        TransmitterReceiver,
    ) {
        let app_name = make_name(&["test", "app", "v1"]);
        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;
        let conn_id = 12345u64;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, rx_app) = mpsc::channel(16);
        let (tx_transmitter, rx_transmitter) = mpsc::unbounded_channel();

        let transmitter = MockTransmitter {
            slim_tx: tx_transmitter,
            interceptors: Arc::new(parking_lot::Mutex::new(vec![])),
        };

        let storage_path = std::path::PathBuf::from("/tmp/test");

        let session_layer = SessionLayer::new(
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            storage_path,
        );

        (session_layer, rx_slim, rx_app, rx_transmitter)
    }

    #[tokio::test]
    async fn test_new_session_layer() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        assert_eq!(session_layer.app_id(), 0);
        assert_eq!(session_layer.conn_id(), 12345);
        assert!(session_layer.is_pool_empty());
    }

    #[tokio::test]
    async fn test_add_and_remove_app_name() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let name1 = make_name(&["service", "v1", "api"]);
        let name2 = make_name(&["service", "v2", "api"]);

        session_layer.add_app_name(name1.clone());
        session_layer.add_app_name(name2.clone());

        // Verify names are added
        assert_eq!(session_layer.app_names.read().len(), 3); // initial + 2 added

        session_layer.remove_app_name(&name1);
        assert_eq!(session_layer.app_names.read().len(), 2);

        session_layer.remove_app_name(&name2);
        assert_eq!(session_layer.app_names.read().len(), 1);
    }

    #[tokio::test]
    async fn test_get_identity_token() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let token = session_layer.get_identity_token();
        assert!(token.is_ok());
        assert_eq!(token.unwrap(), "mock_token");
    }

    #[tokio::test]
    async fn test_create_session_with_auto_id() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let result = session_layer.create_session_internal(config, local_name, destination, None);

        assert!(result.is_ok());
        assert_eq!(session_layer.pool_size(), 1);
    }

    #[tokio::test]
    async fn test_create_session_with_specific_id() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let session_id = 100u32;
        let result = session_layer.create_session_internal(
            config,
            local_name,
            destination,
            Some(session_id),
        );

        assert!(result.is_ok());
        assert_eq!(session_layer.pool_size(), 1);

        let session = session_layer.get_session(session_id);
        assert!(session.is_some());
    }

    #[tokio::test]
    async fn test_create_session_with_invalid_id() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        // Use an ID outside the SESSION_RANGE (SESSION_RANGE is 0..u32::MAX-1000)
        let invalid_id = u32::MAX - 500; // This is outside SESSION_RANGE
        let result = session_layer.create_session_internal(
            config,
            local_name,
            destination,
            Some(invalid_id),
        );

        assert!(result.is_err());
        match result {
            Err(SessionError::InvalidSessionId(_)) => {}
            _ => panic!("Expected InvalidSessionId error"),
        }
    }

    #[tokio::test]
    async fn test_create_session_with_duplicate_id() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let session_id = 100u32;

        // Create first session
        let result1 = session_layer.create_session_internal(
            config.clone(),
            local_name.clone(),
            destination.clone(),
            Some(session_id),
        );
        assert!(result1.is_ok());

        // Try to create second session with same ID
        let result2 = session_layer.create_session_internal(
            config,
            local_name,
            destination,
            Some(session_id),
        );

        assert!(result2.is_err());
        match result2 {
            Err(SessionError::SessionIdAlreadyUsed(_)) => {}
            _ => panic!("Expected SessionIdAlreadyUsed error"),
        }
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        let session_id = 100u32;
        let _context = session_layer
            .create_session_internal(config, local_name, destination, Some(session_id))
            .unwrap();

        assert_eq!(session_layer.pool_size(), 1);

        let removed = session_layer
            .remove_session(session_id)
            .expect("error removing connection");
        // await for the handler
        removed.await.expect("error awaiting the handler");
        assert!(session_layer.is_pool_empty());

        // Try to remove non-existent session
        let removed_again = session_layer.remove_session(session_id);
        assert!(removed_again.is_err());
    }

    #[tokio::test]
    async fn test_get_local_name_for_session() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let name = make_name(&["service", "api", "v1"]);
        session_layer.add_app_name(name.clone());

        let dst = name.with_id(123);
        let result = session_layer.get_local_name_for_session(dst);

        assert!(result.is_ok());
        let local_name = result.unwrap();
        assert_eq!(local_name.id(), session_layer.app_id());
    }

    #[tokio::test]
    async fn test_get_local_name_for_session_not_found() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let unknown_name = make_name(&["unknown", "service", "v1"]);
        let result = session_layer.get_local_name_for_session(unknown_name);

        assert!(result.is_err());
        match result {
            Err(SessionError::SubscriptionNotFound(_)) => {}
            _ => panic!("Expected SubscriptionNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_tx_slim_and_tx_app_cloning() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let tx_slim = session_layer.tx_slim();
        let tx_app = session_layer.tx_app();

        // Just verify that we can clone these channels
        let _tx_slim2 = tx_slim.clone();
        let _tx_app2 = tx_app.clone();
    }

    #[tokio::test]
    async fn test_handle_discovery_request_without_session() {
        let (session_layer, _rx_slim, _rx_app, mut rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        session_layer.add_app_name(local_name.clone());

        let source = make_name(&["remote", "app", "v1"]);
        let message = Message::builder()
            .source(source)
            .destination(local_name.clone().with_id(session_layer.app_id()))
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(100)
            .message_id(0)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        let result = session_layer
            .handle_message_from_slim_without_session(
                &local_name,
                &message,
                ProtoSessionType::PointToPoint,
                ProtoSessionMessageType::DiscoveryRequest,
                100,
            )
            .await;

        assert!(result.is_ok());

        // Verify that a discovery reply was sent
        let sent_message = rx_transmitter.try_recv();
        assert!(
            sent_message.is_ok(),
            "Expected a message to be sent to transmitter"
        );
        let msg = sent_message.unwrap().unwrap();
        assert_eq!(
            msg.get_session_header().session_message_type(),
            ProtoSessionMessageType::DiscoveryReply
        );
    }

    #[tokio::test]
    async fn test_handle_message_from_slim_without_session_invalid_type() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let source = make_name(&["remote", "app", "v1"]);
        let message = Message::builder()
            .source(source)
            .destination(local_name.clone().with_id(session_layer.app_id()))
            .application_payload("application/octet-stream", vec![])
            .build_publish()
            .unwrap();

        let result = session_layer
            .handle_message_from_slim_without_session(
                &local_name,
                &message,
                ProtoSessionType::PointToPoint,
                ProtoSessionMessageType::Msg, // Not a DiscoveryRequest
                100,
            )
            .await;

        assert!(result.is_err());
        match result {
            Err(SessionError::SlimTransmission(_)) => {}
            _ => panic!("Expected SlimTransmission error"),
        }
    }

    #[tokio::test]
    async fn test_multiple_sessions_in_pool() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_enabled: false,
            initiator: true,
            metadata: Default::default(),
        };

        // Create multiple sessions
        for i in 0..5 {
            let destination = make_name(&["remote", &format!("app{}", i), "v1"]);
            let result = session_layer.create_session_internal(
                config.clone(),
                local_name.clone(),
                destination,
                None,
            );
            assert!(result.is_ok());
        }

        assert_eq!(session_layer.pool_size(), 5);
    }

    #[tokio::test]
    async fn test_remove_app_name_with_null_component() {
        let (session_layer, _rx_slim, _rx_app, _rx_transmitter) = setup_session_layer();

        let name = make_name(&["service", "v1", "api"]).with_id(123);
        session_layer.add_app_name(name.clone());

        // Remove with specific ID (should normalize to NULL_COMPONENT)
        session_layer.remove_app_name(&name);

        // The name with NULL_COMPONENT should be removed
        let name_null = name.with_id(Name::NULL_COMPONENT);
        assert!(!session_layer.app_names.read().contains(&name_null));
    }
}

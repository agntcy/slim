// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::sync::Arc;

use display_error_chain::ErrorChainExt;
// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use rand::Rng;

use tokio::sync::Semaphore;
use tokio::sync::mpsc::Sender;
use tracing::{Instrument, debug, error, warn};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::{
    EncodedName, NameId, ParticipantSettings, ProtoMessage as Message, ProtoName,
    ProtoSessionMessageType, ProtoSessionType,
};

use crate::common::SessionMessage;
use crate::completion_handle::CompletionHandle;
use crate::notification::Notification;
use crate::session_config::SessionConfig;
use crate::session_controller::SessionController;
use crate::subscription_manager::SubscriptionManager;

// Local crate
use super::context::SessionContext;

use super::{SESSION_RANGE, SlimChannelSender};
use super::{SessionError, session_controller::handle_channel_discovery_message};
/// Direction enum for session creation
/// Indicates whether the session can send, receive, both, or neither data messages.
#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Send,          // Can only send data messages (shutdown_send: false, shutdown_receive: true)
    Recv,          // Can only receive data messages (shutdown_send: true, shutdown_receive: false)
    Bidirectional, // Can send and receive data messages (shutdown_send: false, shutdown_receive: false)
    None, // Neither send nor receive data messages (shutdown_send: true, shutdown_receive: true)
}

impl Direction {
    pub fn to_flags(self) -> (bool, bool) {
        match self {
            Direction::Send => (false, true),
            Direction::Recv => (true, false),
            Direction::Bidirectional => (false, false),
            Direction::None => (true, true),
        }
    }

    pub fn to_participant_settings(self) -> ParticipantSettings {
        match self {
            // None (absent) means true, so only set fields explicitly when false
            Direction::Send => ParticipantSettings {
                sends_data: true,
                receives_data: false,
            },
            Direction::Recv => ParticipantSettings {
                sends_data: false,
                receives_data: true,
            },
            Direction::Bidirectional => ParticipantSettings {
                sends_data: true,
                receives_data: true,
            },
            Direction::None => ParticipantSettings {
                sends_data: false,
                receives_data: false,
            },
        }
    }
}

/// SessionLayer manages sessions and their lifecycle
pub struct SessionLayer<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Session pool
    pool: Arc<SyncRwLock<HashMap<u32, Arc<SessionController>>>>,

    /// Default name of the local app
    app_id: u128,

    /// Names registered by local app, keyed by encoded name (null id) → subscription_id
    app_names: SyncRwLock<HashMap<EncodedName, u64>>,

    /// Identity provider for the local app
    identity_provider: P,

    /// Identity verifier
    identity_verifier: V,

    /// ID of the local connection
    conn_id: u64,

    /// Tx channels
    tx_slim: SlimChannelSender,
    tx_app: Sender<Result<Notification, SessionError>>,

    /// Channel to clone on session creation
    tx_session: tokio::sync::mpsc::Sender<Result<SessionMessage, SessionError>>,

    /// map from session id to session context
    /// once a new session is created on reception of a join request, store it temporarily
    /// in this map waiting for the welcome message before notifying it to the application
    to_notify: SyncRwLock<HashMap<u32, SessionContext>>,

    /// direction to use for the new sessions
    direction: Direction,

    /// Shared subscription manager — used by both this layer and all sessions it creates
    subscription_manager: SubscriptionManager,

    /// Service ID propagated into every session span
    service_id: String,

    /// Bounds concurrent identity verifications for messages without a session.
    /// Caps the blast radius of an unknown-session flood with slow verifications.
    pre_session_verify_slots: Arc<Semaphore>,
}

impl<P, V> SessionLayer<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    const PRE_SESSION_VERIFY_SLOTS: usize = 128;

    /// Create a new SessionLayer
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app_name: ProtoName,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: Sender<Result<Notification, SessionError>>,
        direction: Direction,
        service_id: String,
    ) -> Self {
        let (tx_session, rx_session) = tokio::sync::mpsc::channel(16);

        let subscription_manager = SubscriptionManager::new(tx_slim.clone());

        let initial_key = Self::name_to_key(&app_name);
        let sl = SessionLayer {
            pool: Arc::new(SyncRwLock::new(HashMap::new())),
            app_id: app_name.id(),
            app_names: SyncRwLock::new(HashMap::from([(initial_key, 0)])),
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            tx_session,
            to_notify: SyncRwLock::new(HashMap::new()),
            direction,
            subscription_manager,
            service_id,
            pre_session_verify_slots: Arc::new(Semaphore::new(Self::PRE_SESSION_VERIFY_SLOTS)),
        };

        sl.listen_from_sessions(rx_session);

        sl
    }

    pub fn tx_slim(&self) -> SlimChannelSender {
        self.tx_slim.clone()
    }

    pub fn subscription_manager(&self) -> SubscriptionManager {
        self.subscription_manager.clone()
    }

    pub fn tx_app(&self) -> Sender<Result<Notification, SessionError>> {
        self.tx_app.clone()
    }

    #[allow(dead_code)]
    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }

    pub fn app_id(&self) -> u128 {
        self.app_id
    }

    /// Build the HashMap key (EncodedName with null component_3) from a ProtoName.
    fn name_to_key(name: &ProtoName) -> EncodedName {
        let enc = name.name.as_ref().unwrap();
        EncodedName {
            component_0: enc.component_0,
            component_1: enc.component_1,
            component_2: enc.component_2,
            name_id: Some(NameId::from(NameId::NULL_COMPONENT)),
        }
    }

    pub fn add_app_name(&self, name: ProtoName, subscription_id: u64) {
        let key = Self::name_to_key(&name);
        self.app_names.write().insert(key, subscription_id);
    }

    pub fn remove_app_name(&self, name: &ProtoName) -> Option<u64> {
        let key = Self::name_to_key(name);
        let removed = self.app_names.write().remove(&key);
        if removed.is_none() {
            warn!(%name, "tried to remove unknown app name");
        }
        removed
    }

    fn get_local_name_for_session(&self, dst: ProtoName) -> Result<ProtoName, SessionError> {
        let key = Self::name_to_key(&dst);
        if self.app_names.read().contains_key(&key) {
            Ok(dst.with_id(self.app_id))
        } else {
            Err(SessionError::SubscriptionNotFound(dst))
        }
    }

    /// Get identity token from the identity provider
    pub fn get_identity_token(&self) -> Result<String, SessionError> {
        let token = self.identity_provider.get_token()?;
        Ok(token)
    }

    /// Public interface to create a new session
    #[tracing::instrument(skip_all, fields(service_id = %self.service_id))]
    pub async fn create_session(
        &self,
        mut session_config: SessionConfig,
        local_name: ProtoName,
        destination: ProtoName,
        id: Option<u32>,
    ) -> Result<(SessionContext, CompletionHandle), SessionError> {
        // Sanity check
        session_config.initiator = true;

        // Store values before they are moved
        let is_p2p = session_config.session_type == ProtoSessionType::PointToPoint;
        let destination_proto = destination.clone();

        let session = self.create_session_internal(session_config, local_name, destination, id)?;

        // If session is p2p, initiate the discovery request now and return the ack
        // Otherwise, return an immediately resolved future
        let init_ack = if is_p2p {
            session
                .session()
                .upgrade()
                .ok_or(SessionError::SessionNotFound(u32::MAX))?
                .invite_participant_internal(&destination_proto)
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
        local_name: ProtoName,
        destination: ProtoName,
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
                            return Err(SessionError::InvalidSessionId(id));
                        }

                        // check if the session ID is already used
                        if pool.contains_key(&id) {
                            return Err(SessionError::SessionIdAlreadyUsed(id));
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

            // Create app channel for this session
            let (app_tx, app_rx) = tokio::sync::mpsc::unbounded_channel();

            // Build the session controller (this is async, so no locks are held)
            // The builder will automatically force DATA_CHANNEL_ID for multicast destinations
            let builder = SessionController::builder()
                .with_id(session_id)
                .with_source(local_name.clone())
                .with_destination(destination.clone())
                .with_config(session_config.clone())
                .with_identity_provider(self.identity_provider.clone())
                .with_identity_verifier(self.identity_verifier.clone())
                .with_slim_tx(self.tx_slim.clone())
                .with_app_tx(app_tx)
                .with_tx_to_session_layer(self.tx_session.clone())
                .with_direction(self.direction)
                .with_subscription_manager(self.subscription_manager.clone())
                .with_service_id(self.service_id.clone())
                .ready()?;

            // Perform the async build operation without holding any lock
            let session_controller = Arc::new(builder.build()?);

            // Reacquire lock to insert the session
            let mut pool = self.pool.write();

            // Double-check that the ID wasn't taken while we didn't hold the lock
            if pool.contains_key(&session_id) {
                // If a specific ID was provided, return an error
                if id.is_some() {
                    return Err(SessionError::SessionIdAlreadyUsed(session_id));
                }
                // If ID was randomly generated, retry with a new ID
                continue;
            }

            let ret = pool.insert(session_id, session_controller.clone());

            // This should never happen, but just in case
            if ret.is_some() {
                error!(
                    %session_id,
                    "session ID was taken during insertion: this should not happen",
                );
                return Err(SessionError::SessionIdAlreadyUsed(session_id));
            }

            return Ok(SessionContext::new(session_controller, app_rx));
        }
    }

    pub fn listen_from_sessions(
        &self,
        mut rx_session: tokio::sync::mpsc::Receiver<Result<SessionMessage, SessionError>>,
    ) {
        let pool_clone = self.pool.clone();
        let sessions_span = tracing::info_span!(parent: None, "listen_from_sessions", service_id = %self.service_id);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    next = rx_session.recv() => {
                        match next {
                            Some(Ok(SessionMessage::DeleteSession { session_id })) => {
                                debug!(%session_id, "received closing signal, cancel session from the pool");
                                if pool_clone.write().remove(&session_id).is_none() {
                                    warn!(%session_id, "requested to delete unknown session");
                                }
                            }
                            Some(Ok(m)) => {
                                error!(?m, "received unexpected message");
                            }
                            Some(Err(e)) => {
                                warn!(error = %e.chain(), "error from session");
                            }
                            None => {
                                // All senders dropped; exit loop.
                                break;
                            }
                        }
                    }
                }
            }
        }.instrument(sessions_span));
    }

    /// Remove a session from the pool and return a handle to optionally wait on
    #[tracing::instrument(skip_all, fields(service_id = %self.service_id, session_id = id))]
    pub fn remove_session(&self, id: u32) -> Result<CompletionHandle, SessionError> {
        debug!(%id, "try to remove session");
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
                let result = session.close().map(CompletionHandle::from_join_handle);
                (*id, result)
            })
            .collect()
    }

    /// Handle an error coming from SLIM. Forward it to the corresponding session.
    #[tracing::instrument(skip_all, fields(service_id = %self.service_id))]
    pub async fn handle_error_from_slim(&self, error: SessionError) -> Result<(), SessionError> {
        // Extract context and session ID from the error
        let Some(session_ctx) = error.session_context() else {
            debug!(
                error = %error.chain(),
                "received error without session context in handle_error_from_slim",
            );
            return Ok(());
        };

        let session_id = session_ctx.session_id;
        let session_controller = self.pool.read().get(&session_id).cloned();

        if let Some(controller) = session_controller {
            debug!(
                error = %error.chain(),
                session_id = %session_id,
                "received error from SLIM for session id",
            );

            // pass the error to the session
            return controller.on_error_message_from_slim(error).await;
        }

        debug!(
            error = %error.chain(),
            "received error from SLIM for unknown session id",
        );

        Ok(())
    }

    /// Handle a message from the message processor, and pass it to the
    /// corresponding session
    #[tracing::instrument(skip_all, fields(service_id = %self.service_id))]
    pub async fn handle_message_from_slim(
        self: &Arc<Self>,
        message: Message,
    ) -> Result<(), SessionError> {
        tracing::trace!(
            msg_type = %message.get_session_message_type().as_str_name(),
            session_id = %message.get_id(),
            "received message from SLIM",
        );

        let (id, session_type, session_message_type) = {
            let header = message.get_session_header();
            (
                header.session_id,
                header.session_type(),
                header.session_message_type(),
            )
        };

        // Fast path: known session — route to its controller. The controller's
        // processing loop verifies identity in its own task, so sessions don't
        // serialize behind each other.
        let session_controller = self.pool.read().get(&id).cloned();
        if let Some(controller) = session_controller {
            controller.on_message_from_slim(message).await?;

            if session_message_type == ProtoSessionMessageType::GroupWelcome {
                let new_session = self
                    .to_notify
                    .write()
                    .remove(&id)
                    .ok_or(SessionError::NewSessionSendFailed)?;
                return self
                    .tx_app
                    .send(Ok(Notification::NewSession(new_session)))
                    .await
                    .map_err(|_e| SessionError::NewSessionSendFailed);
            }

            return Ok(());
        }

        // Slow path: no session yet. JoinRequest is processed inline so that
        // the session is registered before the next message (e.g. GroupWelcome)
        // arrives on this same receive loop. Its identity will be verified by
        // the new controller's processing loop (single verify, no replay
        // collision). Stateless DiscoveryRequest is verified off-task before
        // replying. Everything else is dropped.
        match session_message_type {
            ProtoSessionMessageType::JoinRequest => {
                self.handle_join_request(message, id, session_type).await
            }
            ProtoSessionMessageType::DiscoveryRequest => {
                self.handle_discovery_request(message, id, session_type)
            }
            _ => {
                tracing::debug!(?message, "received channel message with unknown session id");
                Ok(())
            }
        }
    }

    fn handle_discovery_request(
        self: &Arc<Self>,
        message: Message,
        id: u32,
        session_type: ProtoSessionType,
    ) -> Result<(), SessionError> {
        let layer = self.clone();
        tokio::spawn(async move {
            let _permit = match layer.pre_session_verify_slots.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };

            if let Err(e) =
                // For Discovery messages header validation is always turned on because at this
                // point there is no Session to collect the MlsSettings from.
                crate::session_controller::verify_identity(
                    &message,
                    &layer.identity_verifier,
                    true,
                )
                .await
            {
                let err = e.chain();
                error!(
                    error = %err,
                    msg_type = %ProtoSessionMessageType::DiscoveryRequest.as_str_name(),
                    "dropping pre-session message: identity verification failed",
                );
                return;
            }

            if let Err(e) = layer
                .process_discovery_request(message, id, session_type)
                .await
            {
                debug!(error = %e.chain(), "error handling discovery request");
            }
        });

        Ok(())
    }

    async fn process_discovery_request(
        &self,
        message: Message,
        id: u32,
        session_type: ProtoSessionType,
    ) -> Result<(), SessionError> {
        let local_name = self.get_local_name_for_session(message.get_slim_header().get_dst())?;
        let mut reply = handle_channel_discovery_message(&message, &local_name, id, session_type)?;
        crate::session_controller::sign_outbound_control_message(
            &mut reply,
            &self.identity_provider,
        )?;
        if let Err(e) = self.tx_slim.send(Ok(reply)).await {
            debug!(error = %e.chain(), "error sending discovery reply");
        }
        Ok(())
    }

    async fn handle_join_request(
        &self,
        message: Message,
        id: u32,
        session_type: ProtoSessionType,
    ) -> Result<(), SessionError> {
        let local_name = self.get_local_name_for_session(message.get_slim_header().get_dst())?;

        let new_session = match session_type {
            ProtoSessionType::PointToPoint => {
                let conf = crate::SessionConfig::from_join_request(
                    ProtoSessionType::PointToPoint,
                    message.extract_command_payload()?,
                    message.get_metadata_map(),
                    false,
                )?;
                self.create_session_internal(conf, local_name, message.get_source(), Some(id))?
            }
            ProtoSessionType::Multicast => {
                let payload = message.extract_join_request()?;
                if payload.timer_settings.is_none() {
                    return Err(SessionError::MissingPayload {
                        context: "timer options",
                    });
                }
                let channel = payload
                    .channel
                    .clone()
                    .ok_or(SessionError::MissingChannelName)?;
                let conf = crate::SessionConfig::from_join_request(
                    ProtoSessionType::Multicast,
                    message.extract_command_payload()?,
                    message.get_metadata_map(),
                    false,
                )?;
                self.create_session_internal(conf, local_name, channel, Some(id))?
            }
            _ => {
                warn!(
                    session_type = %session_type.as_str_name(),
                    "received channel join request with unknown session type",
                );
                return Err(SessionError::SessionTypeUnknown(session_type));
            }
        };

        let session_controller = new_session
            .session()
            .upgrade()
            .ok_or(SessionError::SessionClosed)?;

        session_controller.on_message_from_slim(message).await?;

        self.to_notify
            .write()
            .insert(new_session.session_id(), new_session);

        Ok(())
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
    use crate::test_utils::{MockTokenProvider, MockVerifier, sign_test_control_message};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::Status;
    use slim_datapath::api::{NameId, ProtoName, ProtoSessionType};
    use tokio::sync::mpsc;

    // --- Test Mocks -----------------------------------------------------------------------

    fn make_name(parts: &[&str; 3]) -> ProtoName {
        ProtoName::from_strings([parts[0], parts[1], parts[2]]).with_id(0)
    }

    type TestSessionLayer = Arc<SessionLayer<MockTokenProvider, MockVerifier>>;
    type SlimReceiver = mpsc::Receiver<Result<Message, Status>>;
    type AppReceiver = mpsc::Receiver<Result<Notification, SessionError>>;

    fn setup_session_layer() -> (TestSessionLayer, SlimReceiver, AppReceiver) {
        let app_name = make_name(&["test", "app", "v1"]);
        let identity_provider = MockTokenProvider;
        let identity_verifier = MockVerifier;
        let conn_id = 12345u64;

        let (tx_slim, rx_slim) = mpsc::channel(16);
        let (tx_app, rx_app) = mpsc::channel(16);

        let session_layer = Arc::new(SessionLayer::new(
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            Direction::Bidirectional,
            "test-service".to_string(),
        ));

        (session_layer, rx_slim, rx_app)
    }

    #[tokio::test]
    async fn test_new_session_layer() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        assert_eq!(session_layer.app_id(), 0);
        assert_eq!(session_layer.conn_id(), 12345);
        assert!(session_layer.is_pool_empty());
    }

    #[tokio::test]
    async fn test_add_and_remove_app_name() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let name1 = make_name(&["service", "v1", "api"]);
        let name2 = make_name(&["service", "v2", "api"]);

        session_layer.add_app_name(name1.clone(), 0);
        session_layer.add_app_name(name2.clone(), 0);

        // Verify names are added
        assert_eq!(session_layer.app_names.read().len(), 3); // initial + 2 added

        session_layer.remove_app_name(&name1);
        assert_eq!(session_layer.app_names.read().len(), 2);

        session_layer.remove_app_name(&name2);
        assert_eq!(session_layer.app_names.read().len(), 1);
    }

    #[tokio::test]
    async fn test_get_identity_token() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let token = session_layer.get_identity_token();
        assert!(token.is_ok());
        assert_eq!(token.unwrap(), "");
    }

    #[tokio::test]
    async fn test_create_session_with_auto_id() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
            initiator: true,
            metadata: Default::default(),
        };

        let result = session_layer.create_session_internal(config, local_name, destination, None);

        assert!(result.is_ok());
        assert_eq!(session_layer.pool_size(), 1);
    }

    #[tokio::test]
    async fn test_create_session_with_specific_id() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
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
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
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
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
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
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let destination = make_name(&["remote", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
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
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let name = make_name(&["service", "api", "v1"]);
        session_layer.add_app_name(name.clone(), 0);

        let dst = name.with_id(123);
        let result = session_layer.get_local_name_for_session(dst);

        assert!(result.is_ok());
        let local_name = result.unwrap();
        assert_eq!(local_name.id(), session_layer.app_id());
    }

    #[tokio::test]
    async fn test_get_local_name_for_session_not_found() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

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
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let tx_slim = session_layer.tx_slim();
        let tx_app = session_layer.tx_app();

        // Just verify that we can clone these channels
        let _tx_slim2 = tx_slim.clone();
        let _tx_app2 = tx_app.clone();
    }

    #[tokio::test]
    async fn test_handle_discovery_request_without_session() {
        const TEST_SECRET: &str = "abcdefghijklmnopqrstuvwxyz012345";

        let app_name = make_name(&["test", "app", "v1"]);
        let remote_auth = SharedSecret::new("remote", TEST_SECRET).unwrap();
        let local_verifier = SharedSecret::new("local", TEST_SECRET).unwrap();
        let (tx_slim, mut rx_slim) = mpsc::channel(16);
        let (tx_app, _rx_app) = mpsc::channel(16);
        let session_layer = Arc::new(SessionLayer::new(
            app_name,
            MockTokenProvider,
            local_verifier,
            12345u64,
            tx_slim,
            tx_app,
            Direction::Bidirectional,
            "test-service".to_string(),
        ));

        let local_name = make_name(&["local", "app", "v1"]);
        session_layer.add_app_name(local_name.clone(), 0);

        let source = make_name(&["remote", "app", "v1"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(local_name.clone().with_id(session_layer.app_id()))
            .identity("")
            .forward_to(0)
            .incoming_conn(12345)
            .session_type(ProtoSessionType::PointToPoint)
            .session_message_type(ProtoSessionMessageType::DiscoveryRequest)
            .session_id(100)
            .message_id(1)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        sign_test_control_message(&mut message, &remote_auth).unwrap();

        session_layer
            .handle_message_from_slim(message)
            .await
            .unwrap();

        let sent = tokio::time::timeout(std::time::Duration::from_secs(1), rx_slim.recv())
            .await
            .expect("expected a discovery reply")
            .expect("slim channel closed")
            .expect("slim delivered an error");

        assert_eq!(
            sent.get_session_header().session_message_type(),
            ProtoSessionMessageType::DiscoveryReply
        );
    }

    #[tokio::test]
    async fn test_handle_discovery_request_without_e2e_sig_is_dropped() {
        let (session_layer, mut rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        session_layer.add_app_name(local_name.clone(), 0);

        let source = make_name(&["remote", "app", "v1"]);
        let message = Message::builder()
            .source(source.clone())
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

        session_layer
            .handle_message_from_slim(message)
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(rx_slim.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_pre_session_unknown_message_is_dropped() {
        let (session_layer, mut rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        session_layer.add_app_name(local_name.clone(), 0);

        let source = make_name(&["remote", "app", "v1"]);
        let mut message = Message::builder()
            .source(source.clone())
            .destination(local_name.clone().with_id(session_layer.app_id()))
            .application_payload("application/octet-stream", vec![])
            .build_publish()
            .unwrap();
        let header = message.get_session_header_mut();
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);
        header.session_id = 100;

        session_layer
            .handle_message_from_slim(message)
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(rx_slim.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_multiple_sessions_in_pool() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let local_name = make_name(&["local", "app", "v1"]);
        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_secs(1)),
            mls_settings: None,
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

    #[test]
    fn test_direction_to_participant_settings() {
        let s = Direction::Send.to_participant_settings();
        assert!(s.sends_data);
        assert!(!s.receives_data);

        let s = Direction::Recv.to_participant_settings();
        assert!(!s.sends_data);
        assert!(s.receives_data);

        let s = Direction::Bidirectional.to_participant_settings();
        assert!(s.sends_data);
        assert!(s.receives_data);

        let s = Direction::None.to_participant_settings();
        assert!(!s.sends_data);
        assert!(!s.receives_data);
    }

    #[tokio::test]
    async fn test_remove_app_name_with_null_component() {
        let (session_layer, _rx_slim, _rx_app) = setup_session_layer();

        let name = make_name(&["service", "v1", "api"]).with_id(123);
        session_layer.add_app_name(name.clone(), 0);

        // Remove with specific ID (should normalize to NULL_COMPONENT)
        session_layer.remove_app_name(&name);

        // The name with NULL_COMPONENT should be removed
        let name_null = name.with_id(NameId::NULL_COMPONENT);
        assert!(
            !session_layer
                .app_names
                .read()
                .contains_key(
                    &SessionLayer::<MockTokenProvider, MockVerifier>::name_to_key(&name_null)
                )
        );
    }
}

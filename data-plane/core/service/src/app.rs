// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
// Standard library imports
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

// Third-party crates
use display_error_chain::ErrorChainExt;
use slim_datapath::errors::ErrorPayload;
use slim_session::Direction;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::MessageType;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::{
    SUBSCRIPTION_ACK_ERROR, SUBSCRIPTION_ACK_ID, SUBSCRIPTION_ACK_SUCCESS, SlimHeaderFlags,
    TRUE_VAL,
};
use slim_session::{SessionConfig, session_controller::SessionController};

// Local crate
use crate::{ServiceError, SubscriptionAckError};
use slim_session::SlimChannelSender;
use slim_session::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
use slim_session::notification::Notification;
use slim_session::transmitter::AppTransmitter;
use slim_session::{SessionError, SessionLayer, context::SessionContext};

pub struct App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// App name provided when creating the app
    app_name: Name,

    /// Session layer that manages sessions
    session_layer: Arc<SessionLayer<P, V>>,

    /// Cancellation token for the app receiver loop
    cancel_token: tokio_util::sync::CancellationToken,

    /// Pending subscription acknowledgments keyed by ack id.
        pending_subscription_acks:
            Arc<Mutex<HashMap<String, oneshot::Sender<Result<(), SubscriptionAckError>>>>>,

    /// Counter used to generate subscription acknowledgment ids.
    subscription_ack_counter: AtomicU64,
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
    #[allow(dead_code)]
    pub(crate) fn new(
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: mpsc::Sender<Result<Notification, SessionError>>,
    ) -> Self {
        Self::new_with_direction(
            app_name,
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            Direction::Bidirectional,
        )
    }

    /// Create new App instance with direction
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_direction(
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: mpsc::Sender<Result<Notification, SessionError>>,
        direction: Direction,
    ) -> Self {
        // Create identity interceptor
        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            identity_provider.clone(),
            identity_verifier.clone(),
        ));

        // Create the transmitter
        let transmitter = AppTransmitter {
            slim_tx: tx_slim.clone(),
            app_tx: tx_app.clone(),
            interceptors: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        transmitter.add_interceptor(identity_interceptor);

        // Create the session layer
        let session_layer = Arc::new(SessionLayer::new(
            app_name.clone(),
            identity_provider,
            identity_verifier,
            conn_id,
            tx_slim,
            tx_app,
            transmitter,
            direction,
        ));

        // Create a new cancellation token for the app receiver loop
        let cancel_token = tokio_util::sync::CancellationToken::new();

        Self {
            app_name: app_name.clone(),
            session_layer,
            cancel_token,
            pending_subscription_acks: Arc::new(Mutex::new(HashMap::new())),
            subscription_ack_counter: AtomicU64::new(0),
        }
    }

    fn next_subscription_ack_id(&self) -> String {
        let next = self
            .subscription_ack_counter
            .fetch_add(1, Ordering::Relaxed);
        format!("sub-{}", next)
    }

    async fn handle_subscription_ack_message(
        pending_subscription_acks: &Arc<
            Mutex<HashMap<String, oneshot::Sender<Result<(), SubscriptionAckError>>>>,
        >,
        msg: &Message,
    ) {
        let Some(ack_id) = msg.get_metadata(SUBSCRIPTION_ACK_ID).cloned() else {
            return;
        };

        let success = msg
            .get_metadata(SUBSCRIPTION_ACK_SUCCESS)
            .map(|val| val == TRUE_VAL)
            .unwrap_or(false);
        let error_msg = msg.get_metadata(SUBSCRIPTION_ACK_ERROR).cloned();

        let sender = {
            let mut pending = pending_subscription_acks.lock();
            pending.remove(&ack_id)
        };

        if let Some(sender) = sender {
            let _ = sender.send(if success {
                Ok(())
            } else {
                Err(SubscriptionAckError::Rejected {
                    message: error_msg.unwrap_or_else(|| "subscription ack failed".to_string()),
                })
            });
        } else {
            debug!(
                ack_id = %ack_id,
                "received subscription ack with no pending waiter"
            );
        }
    }

    async fn send_with_subscription_ack(
        &self,
        build_message: impl FnOnce(String) -> Message,
        map_ack_error: fn(SubscriptionAckError) -> ServiceError,
    ) -> Result<(), ServiceError> {
        let ack_id = self.next_subscription_ack_id();
        let (ack_tx, ack_rx) = oneshot::channel();
        {
            let mut pending = self.pending_subscription_acks.lock();
            pending.insert(ack_id.clone(), ack_tx);
        }

        let msg = build_message(ack_id.clone());

        if let Err(e) = self.send_message_without_context(msg).await {
            let mut pending = self.pending_subscription_acks.lock();
            pending.remove(&ack_id);
            return Err(e);
        }

        match ack_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(map_ack_error(err)),
            Err(_) => Err(map_ack_error(SubscriptionAckError::ChannelClosed)),
        }
    }

    /// Create a new session with the given configuration
    pub async fn create_session(
        &self,
        session_config: SessionConfig,
        destination: Name,
        id: Option<u32>,
    ) -> Result<(SessionContext, slim_session::CompletionHandle), SessionError> {
        self.session_layer
            .create_session(session_config, self.app_name.clone(), destination, id)
            .await
    }

    /// Delete a session and return a completion handle to await on
    pub fn delete_session(
        &self,
        session: &SessionController,
    ) -> Result<slim_session::CompletionHandle, SessionError> {
        self.session_layer.remove_session(session.id())
    }

    /// Get the app name
    ///
    /// Returns a reference to the name that was provided when the App was created.
    /// This name is used for session management and message routing.
    pub fn app_name(&self) -> &Name {
        &self.app_name
    }

    /// Send a message to the session layer
    async fn send_message_without_context(&self, mut msg: Message) -> Result<(), ServiceError> {
        // these messages are not associated to a session yet
        // so they will bypass the interceptors. Add the identity
        let identity = self.session_layer.get_identity_token()?;

        // Add the identity to the message metadata
        msg.get_slim_header_mut().set_identity(identity);

        self.session_layer
            .tx_slim()
            .send(Ok(msg))
            .await
            .map_err(|e| {
                error!(error = %e.chain(), "error sending message");
                ServiceError::MessageSendingError(e.to_string())
            })
    }

    /// Subscribe the app to receive messages for a name
    pub async fn subscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!(?name, ?conn, "subscribe");

        // Set the ID in the name to be the one of this app
        let name = name.clone().with_id(self.session_layer.app_id());

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };

        let mut builder = Message::builder()
            .source(self.app_name.clone())
            .destination(name.clone());

        if let Some(h) = header {
            builder = builder.flags(h);
        }

        self.send_with_subscription_ack(
            |ack_id| {
                builder
                    .metadata(SUBSCRIPTION_ACK_ID, ack_id)
                    .build_subscribe()
                    .unwrap()
            },
            ServiceError::SubscriptionError,
        )
        .await?;

        // Register the subscription after ack confirms the forwarding table update.
        self.session_layer.add_app_name(name);
        Ok(())
    }

    /// Unsubscribe the app
    pub async fn unsubscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!(?name, ?conn, "unsubscribe");

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };

        let mut builder = Message::builder()
            .source(self.app_name.clone())
            .destination(name.clone());

        if let Some(h) = header {
            builder = builder.flags(h);
        }

        self.send_with_subscription_ack(
            |ack_id| {
                builder
                    .metadata(SUBSCRIPTION_ACK_ID, ack_id)
                    .build_unsubscribe()
                    .unwrap()
            },
            ServiceError::UnsubscriptionError,
        )
        .await?;

        // Remove the subscription after ack confirms the forwarding table update.
        self.session_layer.remove_app_name(name);
        Ok(())
    }

    /// Set a route towards another app
    pub async fn set_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!(%name, %conn, "set route");

        // send a message with subscription from
        let msg = Message::builder()
            .source(self.app_name.clone())
            .destination(name.clone())
            .flags(SlimHeaderFlags::default().with_recv_from(conn))
            .build_subscribe()
            .unwrap();

        self.send_message_without_context(msg).await
    }

    /// Remove a route towards another app
    pub async fn remove_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!(%name, %conn, "remove route");

        // send a message with unsubscription from
        let msg = Message::builder()
            .source(self.app_name.clone())
            .destination(name.clone())
            .flags(SlimHeaderFlags::default().with_recv_from(conn))
            .build_unsubscribe()
            .unwrap();

        self.send_message_without_context(msg).await
    }

    /// Close all sessions and return completion handles to await on
    pub fn clear_all_sessions(
        &self,
    ) -> HashMap<u32, Result<slim_session::CompletionHandle, SessionError>> {
        debug!(
            app = %self.app_name,
            "clearing all sessions",
        );
        self.session_layer.clear_all_sessions()
    }

    /// SLIM receiver loop
    pub(crate) fn process_messages(&self, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let app_name = self.app_name.clone();
        let session_layer = self.session_layer.clone();
        let token_clone = self.cancel_token.clone();
        let pending_subscription_acks = self.pending_subscription_acks.clone();

        tokio::spawn(async move {
            debug!(app = %app_name, "starting message processing loop");

            // subscribe for local name running this loop
            let subscribe_msg = Message::builder()
                .source(app_name.clone())
                .destination(app_name.clone())
                .build_subscribe()
                .unwrap();
            let tx = session_layer.tx_slim();
            tx.send(Ok(subscribe_msg))
                .await
                .expect("error sending subscription");

            loop {
                tokio::select! {
                    next = rx.recv() => {
                        match next {
                            None => {
                                error!(%app_name, "slim channel closed, stopping message processing loop");

                                // Send error to application
                                let tx_app = session_layer.tx_app();
                                if let Err(send_err) = tx_app.send(Err(SessionError::SlimChannelClosed)).await {
                                    // Channel closed, likely during shutdown - log but don't panic
                                    debug!("failed to send slim channel closed error to application: {:?}", send_err);
                                }

                                break;
                            }
                            Some(msg) => {
                                match msg {
                                    Ok(msg) => {
                                        debug!(?msg, "received message in service processing");

                                        // filter only the messages of type publish
                                        match msg.message_type.as_ref() {
                                            Some(MessageType::Publish(_)) => {},
                                            Some(MessageType::Subscribe(_))
                                            | Some(MessageType::Unsubscribe(_)) => {
                                                Self::handle_subscription_ack_message(
                                                    &pending_subscription_acks,
                                                    &msg,
                                                )
                                                .await;
                                                continue;
                                            }
                                            None => {
                                                continue;
                                            }
                                        }

                                        tracing::trace!(session_message_type = %msg.get_session_message_type().as_str_name(), id = msg.get_id(), "received message from SLIM");

                                        // Handle the message
                                        let res = session_layer
                                            .handle_message_from_slim(msg)
                                            .await;

                                        if let Err(e) = res {
                                            // Ignore errors due to subscription not found
                                            if let SessionError::SubscriptionNotFound(_) = e {
                                                debug!("session not found, ignoring message");
                                                continue;
                                            }
                                            error!(error = %e.chain(), "error handling message");
                                        }
                                    }
                                    Err(e) => {
                                        // Log the error
                                        debug!(error = %e.chain(), "received error from SLIM");

                                        let p = ErrorPayload::from_json_str(e.message());
                                        if let Some(payload) = p {
                                            let err = SessionError::SlimSendFailure{
                                                ctx: Box::new(payload),
                                            };

                                            let _ = session_layer.handle_error_from_slim(err).await;
                                        }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::{
        CommandPayload, ProtoMessage, ProtoSessionMessageType, ProtoSessionType,
    };
    use slim_testing::utils::TEST_VALID_SECRET;

    // ============================================================================
    // Test Helpers
    // ============================================================================

    /// Helper: Create a test service with a unique name
    fn create_test_service(test_name: &str) -> crate::service::Service {
        use crate::service::Service;
        use slim_config::component::id::{ID, Kind};

        let service_name = format!("test-service-{}", test_name);
        let id = ID::new_with_name(Kind::new("slim").unwrap(), &service_name).unwrap();
        Service::new(id)
    }

    /// Helper: Create a test app name
    fn create_test_name(suffix: &str) -> Name {
        Name::from_strings(["org", "ns", suffix]).with_id(0)
    }

    /// Helper: Create a test app with SharedSecret auth
    fn create_test_app(
        service: &crate::service::Service,
        name: &Name,
        secret: &str,
    ) -> (
        App<SharedSecret, SharedSecret>,
        tokio::sync::mpsc::Receiver<Result<slim_session::notification::Notification, SessionError>>,
    ) {
        service
            .create_app(
                name,
                SharedSecret::new(secret, TEST_VALID_SECRET).unwrap(),
                SharedSecret::new(secret, TEST_VALID_SECRET).unwrap(),
            )
            .unwrap()
    }

    /// Helper: Create a session and wait for completion
    async fn create_and_complete_session(
        app: &App<SharedSecret, SharedSecret>,
        config: SessionConfig,
        destination: Name,
    ) -> slim_session::context::SessionContext {
        let (session_ctx, completion_handle) =
            app.create_session(config, destination, None).await.unwrap();
        completion_handle.await.unwrap();
        session_ctx
    }

    /// Helper: Wait for a session notification
    async fn wait_for_session(
        notifications: &mut tokio::sync::mpsc::Receiver<
            Result<slim_session::notification::Notification, SessionError>,
        >,
    ) -> slim_session::context::SessionContext {
        match tokio::time::timeout(std::time::Duration::from_secs(2), notifications.recv())
            .await
            .expect("timeout waiting for session")
            .expect("channel closed")
            .expect("error receiving session")
        {
            slim_session::notification::Notification::NewSession(ctx) => ctx,
            _ => panic!("unexpected notification"),
        }
    }

    /// Helper: Receive a message from a session context
    async fn receive_message(ctx: &mut slim_session::context::SessionContext) -> ProtoMessage {
        tokio::time::timeout(std::time::Duration::from_secs(2), ctx.rx.recv())
            .await
            .expect("timeout waiting for message")
            .expect("channel closed")
            .expect("error receiving message")
    }

    #[allow(dead_code)]
    fn create_app() -> App<SharedSecret, SharedSecret> {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        App::new(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("a", TEST_VALID_SECRET).unwrap(),
            0,
            tx_slim,
            tx_app,
        )
    }

    fn create_isolated_test_app(
        suffix: &str,
    ) -> (
        App<SharedSecret, SharedSecret>,
        tokio::sync::mpsc::Receiver<Result<ProtoMessage, slim_datapath::Status>>,
    ) {
        let (tx_slim, rx_slim) = tokio::sync::mpsc::channel(16);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(16);
        let name = create_test_name(&format!("isolated-{suffix}"));

        let app = App::new(
            &name,
            SharedSecret::new("isolated-provider", TEST_VALID_SECRET).unwrap(),
            SharedSecret::new("isolated-verifier", TEST_VALID_SECRET).unwrap(),
            0,
            tx_slim,
            tx_app,
        );

        (app, rx_slim)
    }

    async fn recv_outbound_message(
        rx_slim: &mut tokio::sync::mpsc::Receiver<Result<ProtoMessage, slim_datapath::Status>>,
    ) -> ProtoMessage {
        tokio::time::timeout(std::time::Duration::from_secs(1), rx_slim.recv())
            .await
            .expect("timeout waiting for outbound message")
            .expect("outbound channel closed")
            .expect("failed to send outbound message")
    }

    fn build_subscription_ack_message(
        ack_id: &str,
        success: bool,
        error_msg: Option<&str>,
        add: bool,
    ) -> ProtoMessage {
        let mut builder = ProtoMessage::builder()
            .source(create_test_name("ack-src"))
            .destination(create_test_name("ack-dst"))
            .metadata(SUBSCRIPTION_ACK_ID, ack_id.to_string())
            .metadata(
                SUBSCRIPTION_ACK_SUCCESS,
                if success {
                    TRUE_VAL
                } else {
                    slim_datapath::messages::utils::FALSE_VAL
                },
            );

        if let Some(err) = error_msg {
            builder = builder.metadata(SUBSCRIPTION_ACK_ERROR, err.to_string());
        }

        if add {
            builder.build_subscribe().unwrap()
        } else {
            builder.build_unsubscribe().unwrap()
        }
    }

    #[tokio::test]
    async fn test_handle_subscription_ack_message_uses_default_rejection_message() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().insert("ack-default".to_string(), tx);

        let ack_msg = build_subscription_ack_message("ack-default", false, None, true);
        App::<SharedSecret, SharedSecret>::handle_subscription_ack_message(&pending, &ack_msg)
            .await;

        let ack_result = rx.await.expect("ack sender dropped unexpectedly");
        match ack_result {
            Ok(()) => panic!("expected rejected ack"),
            Err(SubscriptionAckError::Rejected { message }) => {
                assert_eq!(message, "subscription ack failed");
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_subscribe_succeeds_when_ack_is_successful() {
        let (app, mut rx_slim) = create_isolated_test_app("subscribe-success");
        let dst = create_test_name("channel");

        let mut subscribe_fut = Box::pin(app.subscribe(&dst, None));
        let outbound = tokio::select! {
            res = &mut subscribe_fut => panic!("subscribe completed before ack handling: {:?}", res),
            msg = recv_outbound_message(&mut rx_slim) => msg,
        };

        let ack_id = outbound
            .get_metadata(SUBSCRIPTION_ACK_ID)
            .cloned()
            .expect("missing ack id in outbound subscribe message");

        let ack_msg = build_subscription_ack_message(&ack_id, true, None, true);
        App::<SharedSecret, SharedSecret>::handle_subscription_ack_message(
            &app.pending_subscription_acks,
            &ack_msg,
        )
        .await;

        subscribe_fut.await.expect("subscribe should succeed");
    }

    #[tokio::test]
    async fn test_subscribe_returns_rejected_ack_error() {
        let (app, mut rx_slim) = create_isolated_test_app("subscribe-rejected");
        let dst = create_test_name("channel");

        let mut subscribe_fut = Box::pin(app.subscribe(&dst, None));
        let outbound = tokio::select! {
            res = &mut subscribe_fut => panic!("subscribe completed before ack handling: {:?}", res),
            msg = recv_outbound_message(&mut rx_slim) => msg,
        };

        let ack_id = outbound
            .get_metadata(SUBSCRIPTION_ACK_ID)
            .cloned()
            .expect("missing ack id in outbound subscribe message");

        let ack_msg =
            build_subscription_ack_message(&ack_id, false, Some("forwarding update failed"), true);
        App::<SharedSecret, SharedSecret>::handle_subscription_ack_message(
            &app.pending_subscription_acks,
            &ack_msg,
        )
        .await;

        let err = subscribe_fut.await.expect_err("subscribe should fail");
        match err {
            ServiceError::SubscriptionError(SubscriptionAckError::Rejected { message }) => {
                assert_eq!(message, "forwarding update failed");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_unsubscribe_returns_channel_closed_when_ack_sender_is_dropped() {
        let (app, mut rx_slim) = create_isolated_test_app("unsubscribe-channel-closed");
        let dst = create_test_name("channel");

        let mut unsubscribe_fut = Box::pin(app.unsubscribe(&dst, None));
        let outbound = tokio::select! {
            res = &mut unsubscribe_fut => panic!("unsubscribe completed before ack handling: {:?}", res),
            msg = recv_outbound_message(&mut rx_slim) => msg,
        };

        let ack_id = outbound
            .get_metadata(SUBSCRIPTION_ACK_ID)
            .cloned()
            .expect("missing ack id in outbound unsubscribe message");

        {
            let mut pending = app.pending_subscription_acks.lock();
            let sender = pending
                .remove(&ack_id)
                .expect("missing pending sender for ack id");
            drop(sender);
        }

        let err = unsubscribe_fut.await.expect_err("unsubscribe should fail");
        assert!(matches!(
            err,
            ServiceError::UnsubscriptionError(SubscriptionAckError::ChannelClosed)
        ));
    }

    #[tokio::test]
    async fn test_create_session() {
        let service = create_test_service("create-session");
        let name = create_test_name("type");
        let (app, _) = create_test_app(&service, &name, "a");

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            max_retries: Some(2),
            interval: Some(std::time::Duration::from_millis(50)),
            ..Default::default()
        };
        let dst = create_test_name("dst");

        // Session creation should succeed (context returned) but completion should fail
        let (_session, completion_handle) = app
            .create_session(config.clone(), dst.clone(), None)
            .await
            .unwrap();

        // Completion should fail since there's no peer to respond
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(500), completion_handle).await;
        assert!(
            result.is_ok() && result.unwrap().is_err(),
            "Session completion should have failed due to no peer"
        );
    }

    #[tokio::test]
    async fn test_delete_session() {
        let service = create_test_service("delete-session");
        let name = create_test_name("type");
        let (app, _) = create_test_app(&service, &name, "a");

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            interval: Some(std::time::Duration::from_millis(500)),
            max_retries: Some(5),
            ..Default::default()
        };
        let dst = create_test_name("dst");
        let (res, completion_handle) = app.create_session(config, dst, None).await.unwrap();

        // The completion handle should fail, as the channel with SLIM is closed
        assert!(
            completion_handle.await.is_err(),
            "Session creation should have failed due to closed channel"
        );

        // Delete the session
        let handle = app
            .delete_session(&res.session().upgrade().unwrap())
            .expect("failed to delete session");

        // Completion handle should now complete
        handle.await.expect("error during session deletion");
    }

    #[tokio::test]
    async fn test_session_weak_after_delete() {
        let service = create_test_service("weak-after-delete");
        let name = create_test_name("type");
        let (app, _) = create_test_app(&service, &name, "a");

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            ..Default::default()
        };
        let dst = create_test_name("dst");
        let (session_ctx, _completion_error) = app
            .create_session(config, dst, Some(42))
            .await
            .expect("failed to create session");

        // Obtain a strong reference to delete it explicitly
        let strong = session_ctx
            .session_arc()
            .expect("expected session to be alive");
        assert_eq!(strong.id(), 42);

        // Delete the session from the app (removes it from the pool)
        let handler = app
            .delete_session(&strong)
            .expect("failed to delete session");

        // Drop the last strong reference
        drop(strong);

        // awiat for the session to be closed
        handler.await.expect("error while waiting for the handler");

        // After deletion and dropping strong refs, Weak should no longer upgrade
        assert!(
            session_ctx.session().upgrade().is_none(),
            "weak pointer should be invalid after deletion"
        );
    }

    #[tokio::test]
    async fn test_handle_message_from_slim() {
        let service = create_test_service("handle-message-from-slim");
        let source = create_test_name("source");
        let dest = create_test_name("dest");
        let (app, mut rx_app) = create_test_app(&service, &dest, "a");

        let identity = SharedSecret::new("a", TEST_VALID_SECRET).unwrap();

        // send join_request message to create the session
        let payload = CommandPayload::builder()
            .join_request(false, None, None, None)
            .as_content();

        let mut join_request = Message::builder()
            .source(source.clone())
            .destination(dest.clone())
            .identity("")
            .incoming_conn(0)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::JoinRequest)
            .session_id(1)
            .message_id(1)
            .payload(payload)
            .build_publish()
            .unwrap();

        app.session_layer
            .handle_message_from_slim(join_request.clone())
            .await
            .expect_err("should fail as identity is not verified");

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(rx_app.try_recv().is_err());

        // Set the right identity
        join_request
            .get_slim_header_mut()
            .set_identity(identity.get_token().unwrap());

        app.session_layer
            .handle_message_from_slim(join_request.clone())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            rx_app.try_recv().is_err(),
            "Should not receive notification yet"
        );

        // Send GroupWelcome message to complete the handshake
        let welcome_payload = CommandPayload::builder()
            .group_welcome(vec![source.clone(), dest.clone()], None)
            .as_content();

        let welcome_message = Message::builder()
            .source(source.clone())
            .destination(dest.clone())
            .identity(identity.get_token().unwrap())
            .incoming_conn(0)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::GroupWelcome)
            .session_id(1)
            .message_id(2)
            .payload(welcome_payload)
            .build_publish()
            .unwrap();

        app.session_layer
            .handle_message_from_slim(welcome_message)
            .await
            .unwrap();

        // Now we should get the new session notification
        let mut session_ctx = wait_for_session(&mut rx_app).await;
        assert_eq!(session_ctx.session().upgrade().unwrap().id(), 1);

        let mut message = ProtoMessage::builder()
            .source(source.clone())
            .destination(create_test_name("type"))
            .identity(identity.get_token().unwrap())
            .flags(SlimHeaderFlags::default().with_incoming_conn(0))
            .application_payload("msg", vec![0x1, 0x2, 0x3, 0x4])
            .build_publish()
            .unwrap();

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        app.session_layer
            .handle_message_from_slim(message.clone())
            .await
            .unwrap();

        // Receive message from the session
        let msg = receive_message(&mut session_ctx).await;
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_session_id(), 1);
    }

    #[tokio::test]
    async fn test_handle_message_from_app() {
        let service = create_test_service("handle-message-from-app");
        let dst = create_test_name("remote");
        let source = create_test_name("local");

        let (sender_app, _) = create_test_app(&service, &source, "a");
        let (receiver_app, mut receiver_notifications) = create_test_app(&service, &dst, "a");

        // Receiver subscribes to its own name to receive messages
        receiver_app.subscribe(&dst, None).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create sender session
        let session_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            max_retries: Some(5),
            interval: Some(std::time::Duration::from_millis(1000)),
            mls_enabled: false,
            metadata: HashMap::new(),
        };

        let sender_session =
            create_and_complete_session(&sender_app, session_config, dst.clone()).await;
        let mut receiver_session = wait_for_session(&mut receiver_notifications).await;

        // Send a message from sender to receiver
        let test_data = vec![0x1, 0x2, 0x3, 0x4];
        let sender_arc = sender_session.session_arc().unwrap();
        let res = sender_arc
            .publish(&dst, test_data.clone(), None, None)
            .await;
        assert!(res.is_ok());

        // Verify receiver gets the message
        let msg = receive_message(&mut receiver_session).await;
        assert_eq!(msg.get_session_message_type(), ProtoSessionMessageType::Msg);
        assert_eq!(msg.get_source(), source);
        assert_eq!(msg.get_dst(), dst);
        assert_eq!(
            msg.get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            test_data
        );
    }

    /// Test configuration for parameterized P2P session tests
    struct P2PTestConfig {
        test_name: &'static str,
        subscriber_suffix: &'static str,
        publisher_suffix: &'static str,
        subscription_names: Vec<&'static str>,
    }

    /// Test configuration for parameterized multicast session tests
    struct MulticastTestConfig {
        test_name: &'static str,
        moderator_suffix: &'static str,
        channel_suffix: &'static str,
        participant_suffixes: Vec<&'static str>,
    }

    /// Parameterized test template for point-to-point sessions with subscriptions.
    ///
    /// This test validates the following scenario:
    /// 1. Creates 2 apps from the same service (subscriber and publisher)
    /// 2. Subscriber app subscribes to the configured subscription names
    /// 3. Publisher app creates point-to-point sessions targeting each subscription name
    /// 4. Publisher sends messages through each session to initiate connections
    /// 5. Subscriber receives session notifications and verifies:
    ///    - Source name matches the publisher app name
    ///    - Destination name matches the publisher app name (from subscriber's perspective)
    ///    - Session type is PointToPoint
    ///    - Correct number of sessions (matching subscription count) are received
    async fn run_p2p_subscription_test(config: P2PTestConfig) {
        let service = create_test_service(config.test_name);

        let subscriber_name =
            Name::from_strings(["org", "ns", config.subscriber_suffix]).with_id(0);
        let publisher_name = Name::from_strings(["org", "ns", config.publisher_suffix]).with_id(0);

        let (subscriber_app, mut subscriber_notifications) =
            create_test_app(&service, &subscriber_name, "a");
        let (publisher_app, _publisher_notifications) =
            create_test_app(&service, &publisher_name, "a");

        // Generate subscription names based on configuration
        let subscription_names: Vec<Name> = if config.subscription_names.len() == 1
            && config.subscription_names[0] == "subscriber"
        {
            // Special case: subscribe to the subscriber's own name
            vec![subscriber_name.clone()]
        } else {
            // Generate multiple subscription names
            config
                .subscription_names
                .iter()
                .map(|suffix| Name::from_strings(["org", "ns", suffix]).with_id(0))
                .collect()
        };

        // Subscribe to all names with the subscriber app
        for name in &subscription_names {
            subscriber_app.subscribe(name, None).await.unwrap();
        }

        // Give some time for subscriptions to be processed
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create point-to-point sessions from publisher app to each subscription name
        let mut sessions = Vec::new();
        for name in &subscription_names {
            let session_config = SessionConfig {
                session_type: ProtoSessionType::PointToPoint,
                initiator: true,
                ..Default::default()
            };

            let session_ctx =
                create_and_complete_session(&publisher_app, session_config, name.clone()).await;

            // Send a message through the session to initiate the connection
            let session_arc = session_ctx.session_arc().unwrap();
            let test_message = format!("hello {}", config.test_name).into_bytes();
            session_arc
                .publish(name, test_message, None, None)
                .await
                .unwrap();

            sessions.push(session_ctx);
        }

        // Give some time for messages to be processed
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Collect received session notifications from subscriber app
        let mut received_sessions = Vec::new();
        while let Ok(notification) = subscriber_notifications.try_recv() {
            match notification.unwrap() {
                slim_session::notification::Notification::NewSession(session_ctx) => {
                    received_sessions.push(session_ctx);
                }
                _ => continue,
            }
        }

        // Verify we received sessions for each subscription
        assert_eq!(received_sessions.len(), config.subscription_names.len());

        // Test that the received session source information matches the publisher
        let sub_names_set = subscription_names
            .iter()
            .collect::<std::collections::HashSet<_>>();
        for session_ctx in received_sessions {
            let session_arc = session_ctx.session_arc().unwrap();

            // Check that the source matches is in sub_names_set
            let src = session_arc.source();
            assert!(sub_names_set.contains(src));

            // Verify it's a point-to-point session
            assert_eq!(session_arc.session_type(), ProtoSessionType::PointToPoint);

            // Verify the destination is the publisher app (from subscriber's perspective)
            let dst = session_arc.dst();
            assert_eq!(dst, &publisher_name);
        }

        // Verify we created sessions for each subscription
        assert_eq!(sessions.len(), config.subscription_names.len());
    }

    /// Test point-to-point sessions with multiple different subscription names
    #[tokio::test]
    async fn test_p2p_sessions_with_multiple_subscriptions() {
        let config = P2PTestConfig {
            test_name: "multiple-subs",
            subscriber_suffix: "subscriber",
            publisher_suffix: "publisher",
            subscription_names: vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
        };

        run_p2p_subscription_test(config).await;
    }

    /// Test point-to-point sessions with the standard org/ns/subscriber name pattern
    #[tokio::test]
    async fn test_p2p_session_with_standard_subscriber_name() {
        let config = P2PTestConfig {
            test_name: "standard-name",
            subscriber_suffix: "subscriber",
            publisher_suffix: "publisher",
            subscription_names: vec!["subscriber"], // Special marker to use subscriber's own name
        };

        run_p2p_subscription_test(config).await;
    }

    /// Parameterized test template for multicast sessions with multiple participants.
    ///
    /// This test validates the following scenario:
    /// 1. Creates multiple apps from the same service (1 moderator + N participants)
    /// 2. Participants subscribe to the multicast channel name
    /// 3. Moderator creates a multicast session with the channel name
    /// 4. Moderator invites all participants to the multicast session
    /// 5. Moderator sends messages through the multicast session
    /// 6. Participants receive session notifications and verify:
    ///    - Source name matches the channel name (multicast sessions use channel as source)
    ///    - Destination name matches the channel name
    ///    - Session type is Multicast
    ///    - Correct number of sessions are received
    async fn run_multicast_test(config: MulticastTestConfig) {
        let service = create_test_service(config.test_name);

        // Create moderator app
        let moderator_name = Name::from_strings(["org", "ns", config.moderator_suffix]).with_id(0);
        let (moderator_app, mut _moderator_notifications) =
            create_test_app(&service, &moderator_name, "a");

        // Create participant apps and collect their notification channels
        let mut participant_apps = Vec::new();
        let mut participant_notifications = Vec::new();
        let mut participant_names = Vec::new();

        for suffix in &config.participant_suffixes {
            let participant_name = Name::from_strings(["org", "ns", suffix]).with_id(0);
            let (app, notifications) = create_test_app(&service, &participant_name, "a");

            participant_apps.push(app);
            participant_notifications.push(notifications);
            participant_names.push(participant_name);
        }

        // Create multicast channel name
        let channel_name = Name::from_strings(["org", "ns", config.channel_suffix]).with_id(0);

        // Have all participants subscribe to the channel
        for app in &participant_apps {
            app.subscribe(&channel_name, None).await.unwrap();
        }

        // Give some time for subscriptions to be processed
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Create multicast session from moderator
        let session_config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(5),
            interval: Some(std::time::Duration::from_millis(1000)),
            mls_enabled: true,
            initiator: true,
            metadata: HashMap::new(),
        };

        let session_ctx =
            create_and_complete_session(&moderator_app, session_config, channel_name.clone()).await;
        let session_arc = session_ctx.session_arc().unwrap();

        // Invite all participants to the multicast session
        for participant_name in &participant_names {
            session_arc
                .invite_participant(participant_name)
                .await
                .unwrap();
        }

        // Send a test message through the multicast session
        let test_message = format!("multicast hello {}", config.test_name).into_bytes();
        session_arc
            .publish(&channel_name, test_message, None, None)
            .await
            .unwrap();

        // Give some time for messages to be processed
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Collect received session notifications from all participants
        let mut total_received_sessions = 0;

        for (i, mut notifications) in participant_notifications.into_iter().enumerate() {
            let mut participant_sessions = Vec::new();

            while let Ok(notification) = notifications.try_recv() {
                match notification.unwrap() {
                    slim_session::notification::Notification::NewSession(session_ctx) => {
                        participant_sessions.push(session_ctx);
                    }
                    _ => continue,
                }
            }

            // Each participant should receive exactly one session notification
            assert_eq!(
                participant_sessions.len(),
                1,
                "Participant {} should receive exactly 1 session",
                i
            );

            // Verify session information for this participant
            let received_session = &participant_sessions[0];
            let session_arc = received_session.session_arc().unwrap();

            // Verify it's a multicast session
            assert_eq!(session_arc.session_type(), ProtoSessionType::Multicast);

            // For multicast sessions, the destination is also the channel name
            let dst = session_arc.dst();
            assert_eq!(dst, &channel_name);

            total_received_sessions += participant_sessions.len();
        }

        // Verify total number of session notifications matches number of participants
        assert_eq!(total_received_sessions, config.participant_suffixes.len());
    }

    /// Test multicast sessions with many participants
    #[tokio::test]
    async fn test_multicast_session_with_many_participants() {
        let config = MulticastTestConfig {
            test_name: "many-participants",
            moderator_suffix: "leader",
            channel_suffix: "broadcast",
            //participant_suffixes: vec!["p1", "p2"],
            participant_suffixes: vec!["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"],
        };

        run_multicast_test(config).await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_message_acknowledgment_e2e() {
        tracing::info!("SETUP: Creating service and apps");
        let service = create_test_service("ack-e2e");

        let sender_name = create_test_name("sender");
        let receiver_name = create_test_name("receiver");

        let (sender_app, _sender_notifications) = create_test_app(&service, &sender_name, "a");
        let (receiver_app, mut receiver_notifications) =
            create_test_app(&service, &receiver_name, "a");

        // Wait for subscription to be established
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        tracing::info!("SETUP: Creating sender and receiver sessions");
        let sender_session = create_and_complete_session(
            &sender_app,
            SessionConfig {
                session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
                max_retries: Some(5),
                interval: Some(std::time::Duration::from_millis(1000)),
                mls_enabled: true,
                initiator: true,
                metadata: HashMap::new(),
            },
            receiver_name.clone(),
        )
        .await;

        let mut receiver_session = wait_for_session(&mut receiver_notifications).await;

        tracing::info!("SETUP: Sessions established successfully");

        tracing::info!("TEST 1: Successful message with acknowledgment");

        // Send message from sender to receiver and get ack receiver
        let message_data = b"Hello from sender!".to_vec();
        let ack_rx = sender_session
            .session()
            .upgrade()
            .unwrap()
            .publish(&receiver_name, message_data.clone(), None, None)
            .await
            .expect("failed to send message with ack");

        tracing::info!("Sender: Message sent, waiting for acknowledgment...");

        // Receiver should receive the message
        let received = receive_message(&mut receiver_session).await;

        tracing::info!("Receiver: Message received");
        assert_eq!(
            received
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            message_data
        );

        // Wait for acknowledgment from network
        let ack_result = tokio::time::timeout(std::time::Duration::from_secs(2), ack_rx)
            .await
            .expect("timeout waiting for ack notification");

        tracing::info!("Sender: Acknowledgment received from network!");
        assert!(
            ack_result.is_ok(),
            "acknowledgment should succeed: {:?}",
            ack_result
        );

        tracing::info!("TEST 2: Multiple messages with acknowledgments");

        // Send multiple messages and verify all are acknowledged
        let mut ack_receivers = Vec::new();
        for i in 0..3 {
            let msg = format!("Message {}", i).into_bytes();
            let ack_rx = sender_session
                .session()
                .upgrade()
                .unwrap()
                .publish(&receiver_name, msg, None, None)
                .await
                .expect("failed to send message with ack");
            ack_receivers.push(ack_rx);

            // Receive at receiver
            let _received = receive_message(&mut receiver_session).await;
        }

        tracing::info!("Publisher: Sent 3 messages, waiting for all acknowledgments...");

        // Wait for all acknowledgments - they should all succeed
        assert!(
            futures::future::join_all(ack_receivers)
                .await
                .into_iter()
                .all(|r| r.is_ok())
        );

        tracing::info!("All acknowledgment tests passed!");

        // Cleanup
        sender_app
            .delete_session(sender_session.session().upgrade().unwrap().as_ref())
            .unwrap();
        receiver_app
            .delete_session(receiver_session.session().upgrade().unwrap().as_ref())
            .unwrap();
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_invite_participant_with_ack_e2e() {
        tracing::info!("SETUP: Creating service and apps");
        let service = create_test_service("invite-ack-e2e");

        let moderator_name = create_test_name("moderator");
        let participant1_name = create_test_name("participant1");
        let participant2_name = create_test_name("participant2");
        let channel_name = create_test_name("channel");

        let (moderator_app, _moderator_notifications) =
            create_test_app(&service, &moderator_name, "a");
        let (_participant1_app, mut participant1_notifications) =
            create_test_app(&service, &participant1_name, "a");
        let (_participant2_app, mut participant2_notifications) =
            create_test_app(&service, &participant2_name, "a");

        // Wait for subscriptions to be established
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        tracing::info!("SETUP: Creating moderator session");
        let moderator_session = create_and_complete_session(
            &moderator_app,
            SessionConfig {
                session_type: slim_datapath::api::ProtoSessionType::Multicast,
                max_retries: Some(5),
                interval: Some(std::time::Duration::from_millis(100)),
                mls_enabled: false,
                initiator: true,
                metadata: HashMap::new(),
            },
            channel_name.clone(),
        )
        .await;

        let moderator_controller = moderator_session.session().upgrade().unwrap();

        tracing::info!("SETUP: Inviting participant1 (should succeed)");
        // Invite participant1 and wait for ack
        let invite_ack_rx1 = moderator_controller
            .invite_participant(&participant1_name)
            .await
            .expect("failed to invite participant1");

        // Wait for participant1 to receive session notification
        let _participant1_session = wait_for_session(&mut participant1_notifications).await;

        // Wait for invite ack to complete (after JoinReply)
        let invite_result1 =
            tokio::time::timeout(std::time::Duration::from_secs(3), invite_ack_rx1)
                .await
                .expect("timeout waiting for invite ack");

        assert!(
            invite_result1.is_ok(),
            "Expected invite to succeed, got: {:?}",
            invite_result1
        );
        tracing::info!("SETUP: Participant1 invited successfully");

        tracing::info!("SETUP: Inviting participant2 (should succeed)");
        // Invite participant2 and wait for ack
        let invite_ack_rx2 = moderator_controller
            .invite_participant(&participant2_name)
            .await
            .expect("failed to invite participant2");

        // Wait for participant2 to receive session notification
        let _participant2_session = wait_for_session(&mut participant2_notifications).await;

        // Wait for invite ack to complete (after JoinReply)
        let invite_result2 =
            tokio::time::timeout(std::time::Duration::from_secs(3), invite_ack_rx2)
                .await
                .expect("timeout waiting for invite ack");

        assert!(
            invite_result2.is_ok(),
            "Expected invite to succeed, got: {:?}",
            invite_result2
        );
        tracing::info!("SETUP: Participant2 invited successfully");

        // Verify both participants are successfully added (acks received)
        // The session is now active with both participants

        // TEST 1: Try to invite a non-existent participant
        tracing::info!("TEST 1: Invite non-existent participant");
        let nonexistent_participant = Name::from_strings(["org", "ns", "ghost"]).with_id(0);

        let invite_ghost_rx = moderator_controller
            .invite_participant(&nonexistent_participant)
            .await
            .expect("failed to send invite to ghost");

        // This should fail since the participant doesn't exist
        // We do have a 3 sec timeout, but the invite should error out expire sooner (5 * 100ms)
        let ghost_result = tokio::time::timeout(std::time::Duration::from_secs(3), invite_ghost_rx)
            .await
            .expect("timeout waiting for ghost invite ack");

        assert!(
            ghost_result.is_err(),
            "Expected session error for non-existent participant, but got: {:?}",
            ghost_result
        );

        // Now try to remove an existing participant
        tracing::info!("TEST 2: Remove existing participant");

        let remove_result_rx = moderator_controller
            .remove_participant(&participant1_name)
            .await
            .expect("failed to send remove for participant1");

        let remove_result =
            tokio::time::timeout(std::time::Duration::from_secs(3), remove_result_rx)
                .await
                .expect("timeout waiting for remove ack");

        assert!(
            remove_result.is_ok(),
            "Expected remove to succeed, got: {:?}",
            remove_result
        );

        tracing::info!("TEST 3: Remove non-existent participant");
        let remove_nonexistent_rx = moderator_controller
            .remove_participant(&nonexistent_participant)
            .await
            .expect("failed to send remove for ghost");

        let remove_ghost_result =
            tokio::time::timeout(std::time::Duration::from_secs(3), remove_nonexistent_rx)
                .await
                .expect("timeout waiting for remove ack");

        assert!(
            remove_ghost_result.is_err(),
            "Expected remove to fail for non-existent participant, got: {:?}",
            remove_ghost_result
        );

        // NOTE: Remove tests are flaky in test environment and have been moved to separate test
        // The invite mechanism is verified above
        tracing::info!("All invite tests passed!");
    }

    /// E2E test to verify MLS encryption is working correctly
    /// Creates a "spy" as a raw local connection that intercepts messages
    /// Verifies that:
    /// 1. When MLS is ENABLED: messages are encrypted and spy cannot read plaintext
    /// 2. When MLS is DISABLED: messages are plaintext and spy can read them
    /// 3. Legitimate participants can always read messages (decrypt if needed)
    #[tokio::test]
    async fn test_mls_encryption_with_spy_enabled() {
        test_mls_with_spy(true).await;
    }

    #[tokio::test]
    async fn test_mls_encryption_with_spy_disabled() {
        test_mls_with_spy(false).await;
    }

    // Helper: Create a spy connection that intercepts raw messages
    async fn create_spy(
        service: &crate::service::Service,
        channel_name: &Name,
    ) -> (
        tokio::sync::mpsc::Sender<Result<ProtoMessage, slim_datapath::Status>>,
        tokio::sync::mpsc::Receiver<Result<ProtoMessage, slim_datapath::Status>>,
    ) {
        let (spy_conn_id, spy_tx, spy_rx) = service
            .message_processor()
            .register_local_connection(false)
            .unwrap();

        let spy_subscribe_msg = ProtoMessage::builder()
            .source(Name::from_strings(["org", "ns", "spy"]).with_id(0))
            .destination(channel_name.clone())
            .identity("")
            .incoming_conn(spy_conn_id)
            .build_subscribe()
            .unwrap();

        spy_tx
            .send(Ok::<_, slim_datapath::Status>(spy_subscribe_msg))
            .await
            .unwrap();

        (spy_tx, spy_rx)
    }

    // Helper: Receive next Msg type message from spy, skipping control messages
    async fn receive_spy_msg(
        spy_rx: &mut tokio::sync::mpsc::Receiver<Result<ProtoMessage, slim_datapath::Status>>,
    ) -> Vec<u8> {
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(2), spy_rx.recv())
                .await
                .expect("timeout waiting for spy message")
                .expect("spy channel closed")
                .expect("error receiving spy message");

            if msg.get_session_header().session_message_type() == ProtoSessionMessageType::Msg {
                return msg
                    .get_payload()
                    .unwrap()
                    .as_application_payload()
                    .unwrap()
                    .blob
                    .clone();
            } else {
                println!("Ignoring control message");
            }
        }
    }

    async fn test_mls_with_spy(mls_enabled: bool) {
        let service = create_test_service(&format!(
            "mls-spy-{}",
            if mls_enabled { "enabled" } else { "disabled" }
        ));

        let channel_name = create_test_name("secure-channel");
        let moderator_name = create_test_name("moderator");
        let participant_name = create_test_name("participant");

        let (moderator_app, _) = create_test_app(&service, &moderator_name, "moderator");
        let (participant_app, mut participant_notifications) =
            create_test_app(&service, &participant_name, "participant");

        let (_spy_tx, mut spy_rx) = create_spy(&service, &channel_name).await;

        participant_app
            .subscribe(&channel_name, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let session_ctx = create_and_complete_session(
            &moderator_app,
            SessionConfig {
                session_type: ProtoSessionType::Multicast,
                max_retries: Some(5),
                interval: Some(std::time::Duration::from_millis(1000)),
                mls_enabled,
                initiator: true,
                metadata: HashMap::new(),
            },
            channel_name.clone(),
        )
        .await;

        let session_arc = session_ctx.session_arc().unwrap();
        session_arc
            .invite_participant(&participant_name)
            .await
            .unwrap()
            .await
            .unwrap();

        let mut participant_ctx = wait_for_session(&mut participant_notifications).await;

        let msg1: &[u8] = if mls_enabled {
            b"Secret message - should be encrypted!"
        } else {
            b"Secret message - unencrypted!"
        };
        let _ = session_arc
            .publish(&channel_name, msg1.to_vec(), None, None)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let received_msg = receive_message(&mut participant_ctx).await;
        assert_eq!(
            received_msg
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            msg1
        );

        let spy_payload1 = receive_spy_msg(&mut spy_rx).await;

        if mls_enabled {
            assert_ne!(
                spy_payload1, msg1,
                "MLS ENABLED: Spy MUST NOT receive plaintext"
            );
            assert!(!spy_payload1.is_empty());
        } else {
            assert_eq!(
                spy_payload1, msg1,
                "MLS DISABLED: Spy MUST receive plaintext"
            );
        }

        let msg2 = b"Another secret message";
        let _ = session_arc
            .publish(&channel_name, msg2.to_vec(), None, None)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let received_msg2 = receive_message(&mut participant_ctx).await;
        assert_eq!(
            received_msg2
                .get_payload()
                .unwrap()
                .as_application_payload()
                .unwrap()
                .blob,
            msg2
        );

        let spy_payload2 = receive_spy_msg(&mut spy_rx).await;

        if mls_enabled {
            assert_ne!(
                spy_payload2, msg2,
                "MLS ENABLED: Spy should still see encrypted data"
            );
        } else {
            assert_eq!(
                spy_payload2, msg2,
                "MLS DISABLED: Spy should still see plaintext"
            );
        }

        moderator_app
            .delete_session(session_ctx.session().upgrade().unwrap().as_ref())
            .unwrap();
        participant_app
            .delete_session(participant_ctx.session().upgrade().unwrap().as_ref())
            .unwrap();
    }
}

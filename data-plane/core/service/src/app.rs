// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
// Standard library imports
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use tokio::sync::mpsc;
use tracing::{debug, error};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::MessageType;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_session::{SessionConfig, session_controller::SessionController};

// Local crate
use crate::ServiceError;
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
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: mpsc::Sender<Result<Notification, SessionError>>,
        storage_path: std::path::PathBuf,
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
            interceptors: Arc::new(SyncRwLock::new(Vec::new())),
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
            storage_path,
        ));

        // Create a new cancellation token for the app receiver loop
        let cancel_token = tokio_util::sync::CancellationToken::new();

        Self {
            app_name: app_name.clone(),
            session_layer,
            cancel_token,
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
        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(ServiceError::SessionError)?;

        // Add the identity to the message metadata
        msg.get_slim_header_mut().set_identity(identity);

        self.session_layer
            .tx_slim()
            .send(Ok(msg))
            .await
            .map_err(|e| {
                error!("error sending message {}", e);
                ServiceError::MessageSendingError(e.to_string())
            })
    }

    /// Subscribe the app to receive messages for a name
    pub async fn subscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!("subscribe {} - conn {:?}", name, conn);

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

        let msg = builder.build_subscribe().unwrap();

        // Subscribe
        self.send_message_without_context(msg).await?;

        // Register the subscription
        self.session_layer.add_app_name(name);

        Ok(())
    }

    /// Unsubscribe the app
    pub async fn unsubscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!("unsubscribe from {} - {:?}", name, conn);

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

        let msg = builder.build_unsubscribe().unwrap();

        // Unsubscribe
        self.send_message_without_context(msg).await?;

        // Remove the subscription
        self.session_layer.remove_app_name(name);

        Ok(())
    }

    /// Set a route towards another app
    pub async fn set_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!("set route: {} - {:?}", name, conn);

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
        debug!("remove route: {} - {:?}", name, conn);

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
            "clearing all sessions for app {} (returning handles)",
            self.app_name
        );
        self.session_layer.clear_all_sessions()
    }

    /// SLIM receiver loop
    pub(crate) fn process_messages(&self, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let app_name = self.app_name.clone();
        let session_layer = self.session_layer.clone();
        let token_clone = self.cancel_token.clone();

        tokio::spawn(async move {
            debug!("starting message processing loop for {}", app_name);

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

                                        tracing::trace!("received message from SLIM {} {}", msg.get_session_message_type().as_str_name(), msg.get_id());

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
                                            error!("error handling message: {}", e);
                                        }
                                    }
                                    Err(e) => {
                                        error!("error: {}", e);

                                        // if internal error, forward it to application
                                        let tx_app = session_layer.tx_app();
                                        if let Err(send_err) = tx_app.send(Err(SessionError::Forward(e.to_string()))).await {
                                            // Channel closed, likely during shutdown - log but don't panic
                                            debug!("failed to send error to application (channel closed): {:?}", send_err);
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

    #[allow(dead_code)]
    fn create_app() -> App<SharedSecret, SharedSecret> {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        App::new(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        )
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_slim, _rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, _rx_app) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let app = App::new(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            ..Default::default()
        };
        let dst = Name::from_strings(["org", "ns", "dst"]);

        // Session creation should hang as there is no peer side to respond
        let (_session, completion_handle) = app
            .create_session(config.clone(), dst.clone(), None)
            .await
            .unwrap();

        // Session is created but completion shouhld hangs (times out)
        let timeout_result =
            tokio::time::timeout(std::time::Duration::from_millis(100), completion_handle).await;
        assert!(
            timeout_result.is_err(),
            "Session creation should have timed out"
        );
    }

    #[tokio::test]
    async fn test_delete_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let app = App::new(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            interval: Some(std::time::Duration::from_millis(500)),
            max_retries: Some(5),
            ..Default::default()
        };
        let dst = Name::from_strings(["org", "ns", "dst"]);
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
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let app = App::new(
            &name,
            SharedSecret::new("a", TEST_VALID_SECRET),
            SharedSecret::new("a", TEST_VALID_SECRET),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            ..Default::default()
        };
        let dst = Name::from_strings(["org", "ns", "dst"]);
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
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, mut rx_app) = tokio::sync::mpsc::channel(1);
        let source = Name::from_strings(["org", "ns", "source"]).with_id(0);
        let dest = Name::from_strings(["org", "ns", "dest"]).with_id(0);

        let identity = SharedSecret::new("a", TEST_VALID_SECRET);

        let app = App::new(
            &dest,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        );

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
            .message_id(1) // this id will be changed by the session controller
            .payload(payload)
            .build_publish()
            .unwrap();

        app.session_layer
            .handle_message_from_slim(join_request.clone())
            .await
            .expect_err("should fail as identity is not verified");

        // sleep to allow the message to be processed
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // As there is no identity, we should not get any message in the app
        assert!(rx_app.try_recv().is_err());

        // Set the right identity
        join_request
            .get_slim_header_mut()
            .set_identity(identity.get_token().unwrap());

        // Try again
        app.session_layer
            .handle_message_from_slim(join_request.clone())
            .await
            .unwrap();

        // We should get a new session notification
        let new_session = rx_app
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        let mut session_ctx = match new_session {
            Notification::NewSession(ctx) => ctx,
            _ => panic!("unexpected notification"),
        };
        assert_eq!(session_ctx.session().upgrade().unwrap().id(), 1);

        let mut message = ProtoMessage::builder()
            .source(source.clone())
            .destination(Name::from_strings(["org", "ns", "type"]).with_id(0))
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
        let msg = session_ctx
            .rx
            .recv()
            .await
            .expect("no message received")
            .expect("error");
        assert_eq!(msg, message);
        assert_eq!(msg.get_session_header().get_session_id(), 1);
    }

    #[tokio::test]
    async fn test_handle_message_from_app() {
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(16);
        let (tx_app, _) = tokio::sync::mpsc::channel(10);
        let dst = Name::from_strings(["cisco", "default", "remote"]).with_id(0);
        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let identity = SharedSecret::new("a", TEST_VALID_SECRET);

        let app = App::new(
            &source,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let mut session_config =
            SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
        session_config.initiator = true;

        // create a new session
        let (res, _completion_handle) = app
            .create_session(session_config, dst.clone(), Some(1))
            .await
            .unwrap();

        // Do not await on the completion error as there is no peer to complete the handshake

        // a discovery request should be generated by the session just created
        // try to read it on slim
        let discovery_req = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        assert_eq!(
            discovery_req.get_session_message_type(),
            ProtoSessionMessageType::DiscoveryRequest
        );

        // create a discovery reply with the right id
        let payload = CommandPayload::builder().discovery_reply().as_content();

        let discovery_reply = Message::builder()
            .source(dst.clone())
            .destination(source.clone())
            .identity(identity.get_token().unwrap())
            .incoming_conn(0)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::DiscoveryReply)
            .session_id(1)
            .message_id(discovery_req.get_id())
            .payload(payload)
            .build_publish()
            .unwrap();

        // process the discovery reply
        app.session_layer
            .handle_message_from_slim(discovery_reply.clone())
            .await
            .expect("error receiving discovery reply");

        // the local node sets the route to the remote endpoint
        let route = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        assert!(route.is_subscribe(), "route should be a subscribe message");

        // a join request should be generated by the session
        let join_req = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        assert_eq!(
            join_req.get_session_message_type(),
            ProtoSessionMessageType::JoinRequest
        );

        // reply with the right id
        let payload = CommandPayload::builder().join_reply(None).as_content();

        let join_replay = Message::builder()
            .source(dst.clone())
            .destination(source.clone())
            .identity(identity.get_token().unwrap())
            .incoming_conn(0)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::JoinReply)
            .session_id(1)
            .message_id(join_req.get_id())
            .payload(payload)
            .build_publish()
            .unwrap();

        app.session_layer
            .handle_message_from_slim(join_replay.clone())
            .await
            .expect("error receiving join reply");

        // now the local node should send a welcome message
        let welcome = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        assert_eq!(
            welcome.get_session_message_type(),
            ProtoSessionMessageType::GroupWelcome
        );

        let payload = CommandPayload::builder().group_ack().as_content();

        let ack = Message::builder()
            .source(dst.clone())
            .destination(source.clone())
            .identity(identity.get_token().unwrap())
            .incoming_conn(0)
            .session_type(slim_datapath::api::ProtoSessionType::PointToPoint)
            .session_message_type(slim_datapath::api::ProtoSessionMessageType::GroupAck)
            .session_id(1)
            .message_id(welcome.get_id())
            .payload(payload)
            .build_publish()
            .unwrap();

        app.session_layer
            .handle_message_from_slim(ack.clone())
            .await
            .expect("error receiving join reply");

        // now we can finally send a message
        let mut message = ProtoMessage::builder()
            .source(source.clone())
            .destination(dst.clone())
            .application_payload("msg", vec![0x1, 0x2, 0x3, 0x4])
            .build_publish()
            .unwrap();

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        let res = res
            .session()
            .upgrade()
            .unwrap()
            .on_message_from_app(message.clone())
            .await;

        assert!(res.is_ok());

        // message should have been delivered to the app
        let msg = rx_slim
            .recv()
            .await
            .expect("no message received")
            .expect("error");

        assert_eq!(msg.get_session_message_type(), ProtoSessionMessageType::Msg);
        assert_eq!(msg.get_source(), source);
        assert_eq!(msg.get_dst(), dst);
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
        use crate::service::Service;
        use slim_config::component::id::{ID, Kind};

        // Create a service instance with unique name
        let service_name = format!("test-service-{}", config.test_name);
        let id = ID::new_with_name(Kind::new("slim").unwrap(), &service_name).unwrap();
        let service = Service::new(id);

        // Create two apps from the same service
        let subscriber_name =
            Name::from_strings(["org", "ns", config.subscriber_suffix]).with_id(0);
        let publisher_name = Name::from_strings(["org", "ns", config.publisher_suffix]).with_id(0);

        let (subscriber_app, mut subscriber_notifications) = service
            .create_app(
                &subscriber_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        let (publisher_app, _publisher_notifications) = service
            .create_app(
                &publisher_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

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
            // Create session with the subscription name as peer
            let mut session_config =
                SessionConfig::default().with_session_type(ProtoSessionType::PointToPoint);
            session_config.initiator = true;
            let (session_ctx, completion_error) = publisher_app
                .create_session(session_config, name.clone(), None)
                .await
                .unwrap();

            // Wait for session establishment
            completion_error.await.unwrap();

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
        use crate::service::Service;
        use slim_config::component::id::{ID, Kind};

        // Create a service instance with unique name
        let service_name = format!("test-service-{}", config.test_name);
        let id = ID::new_with_name(Kind::new("slim").unwrap(), &service_name).unwrap();
        let service = Service::new(id);

        // Create moderator app
        let moderator_name = Name::from_strings(["org", "ns", config.moderator_suffix]).with_id(0);
        let (moderator_app, mut _moderator_notifications) = service
            .create_app(
                &moderator_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        // Create participant apps and collect their notification channels
        let mut participant_apps = Vec::new();
        let mut participant_notifications = Vec::new();
        let mut participant_names = Vec::new();

        for suffix in &config.participant_suffixes {
            let participant_name = Name::from_strings(["org", "ns", suffix]).with_id(0);
            let (app, notifications) = service
                .create_app(
                    &participant_name,
                    SharedSecret::new("a", TEST_VALID_SECRET),
                    SharedSecret::new("a", TEST_VALID_SECRET),
                )
                .unwrap();

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

        let (session_ctx, completion_handle) = moderator_app
            .create_session(session_config, channel_name.clone(), None)
            .await
            .unwrap();

        // Wait for session establishment
        completion_handle.await.unwrap();

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
        use crate::service::Service;
        use slim_config::component::id::{ID, Kind};

        tracing::info!("SETUP: Creating service and apps");
        // Create a service instance
        let service_name = "test-service-ack-e2e";
        let id = ID::new_with_name(Kind::new("slim").unwrap(), service_name).unwrap();
        let service = Service::new(id);

        // Create two apps
        let sender_name = Name::from_strings(["org", "ns", "sender"]).with_id(0);
        let receiver_name = Name::from_strings(["org", "ns", "receiver"]).with_id(0);

        let (sender_app, _sender_notifications) = service
            .create_app(
                &sender_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        let (receiver_app, mut receiver_notifications) = service
            .create_app(
                &receiver_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        // Wait for subscription to be established
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        tracing::info!("SETUP: Creating sender and receiver sessions");
        // Create sessions
        let (sender_session, sender_completion_handle) = sender_app
            .create_session(
                SessionConfig {
                    session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
                    max_retries: Some(5),
                    interval: Some(std::time::Duration::from_millis(1000)),
                    mls_enabled: true,
                    initiator: true,
                    metadata: HashMap::new(),
                },
                receiver_name.clone(),
                None,
            )
            .await
            .expect("failed to create sender session");

        // Wait for session on receiver side
        let receiver_session = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            receiver_notifications.recv(),
        )
        .await
        .expect("timeout waiting for session at receiver")
        .expect("receiver channel closed")
        .expect("error receiving session notification");

        // The sender session should complete successfully
        sender_completion_handle
            .await
            .expect("sender session failed to establish");

        let mut receiver_session = match receiver_session {
            slim_session::notification::Notification::NewSession(ctx) => ctx,
            _ => panic!("unexpected notification"),
        };

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
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            receiver_session.rx.recv(),
        )
        .await
        .expect("timeout waiting for message at subscriber")
        .expect("subscriber channel closed")
        .expect("error receiving message");

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
            let _received = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                receiver_session.rx.recv(),
            )
            .await
            .expect("timeout waiting for message")
            .expect("channel closed");
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
        use crate::service::Service;
        use slim_config::component::id::{ID, Kind};

        tracing::info!("SETUP: Creating service and apps");
        // Create a service instance
        let service_name = "test-service-invite-ack-e2e";
        let id = ID::new_with_name(Kind::new("slim").unwrap(), service_name).unwrap();
        let service = Service::new(id);

        // Create moderator and participant apps
        let moderator_name = Name::from_strings(["org", "ns", "moderator"]).with_id(0);
        let participant1_name = Name::from_strings(["org", "ns", "participant1"]).with_id(0);
        let participant2_name = Name::from_strings(["org", "ns", "participant2"]).with_id(0);
        let channel_name = Name::from_strings(["org", "ns", "channel"]).with_id(0);

        let (moderator_app, _moderator_notifications) = service
            .create_app(
                &moderator_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        let (_participant1_app, mut participant1_notifications) = service
            .create_app(
                &participant1_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        let (_participant2_app, mut participant2_notifications) = service
            .create_app(
                &participant2_name,
                SharedSecret::new("a", TEST_VALID_SECRET),
                SharedSecret::new("a", TEST_VALID_SECRET),
            )
            .unwrap();

        // Wait for subscriptions to be established
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        tracing::info!("SETUP: Creating moderator session");
        // Create moderator session (multicast)
        let (moderator_session, completion_handle) = moderator_app
            .create_session(
                SessionConfig {
                    session_type: slim_datapath::api::ProtoSessionType::Multicast,
                    max_retries: Some(5),
                    interval: Some(std::time::Duration::from_millis(100)),
                    mls_enabled: false,
                    initiator: true,
                    metadata: HashMap::new(),
                },
                channel_name.clone(),
                None,
            )
            .await
            .expect("failed to create moderator session");

        // Wait for session establishment
        completion_handle
            .await
            .expect("moderator session failed to establish");

        // Extract the session controller
        let moderator_controller = moderator_session.session().upgrade().unwrap();

        tracing::info!("SETUP: Inviting participant1 (should succeed)");
        // Invite participant1 and wait for ack
        let invite_ack_rx1 = moderator_controller
            .invite_participant(&participant1_name)
            .await
            .expect("failed to invite participant1");

        // Wait for participant1 to receive session notification
        let _participant1_session = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            participant1_notifications.recv(),
        )
        .await
        .expect("timeout waiting for session at participant1")
        .expect("participant1 channel closed");

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
        let _participant2_session = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            participant2_notifications.recv(),
        )
        .await
        .expect("timeout waiting for session at participant2")
        .expect("participant2 channel closed");

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
}

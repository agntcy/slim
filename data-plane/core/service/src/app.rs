// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

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
use slim_datapath::messages::utils::{SLIM_IDENTITY, SlimHeaderFlags};

// Local crate
use crate::ServiceError;
use slim_session::Session;
use slim_session::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
use slim_session::notification::Notification;
use slim_session::transmitter::AppTransmitter;
use slim_session::{Id, SessionConfig, SlimChannelSender};
use slim_session::{SessionError, SessionLayer, context::SessionContext};

#[derive(Clone)]
pub struct App<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// App name provided when creating the app
    app_name: Name,

    /// Session layer that manages sessions
    session_layer: Arc<SessionLayer<P, V>>,

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
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: mpsc::Sender<Result<Notification<P, V>, SessionError>>,
        storage_path: std::path::PathBuf,
    ) -> Self {
        // Create identity interceptor
        let identity_interceptor = Arc::new(IdentityInterceptor::new(
            identity_provider.clone(),
            identity_verifier.clone(),
        ));

        // Create the transmitter
        let transmitter = AppTransmitter::<P, V> {
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
        id: Option<Id>,
    ) -> Result<SessionContext<P, V>, SessionError> {
        let ret = self
            .session_layer
            .create_session(session_config, self.app_name.clone(), id)
            .await?;

        // return the session info
        Ok(ret)
    }

    /// Delete a session.
    pub async fn delete_session(&self, session: &Session<P, V>) -> Result<(), SessionError> {
        // remove the session from the pool
        if self.session_layer.remove_session(session.id()).await {
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(session.id()))
        }
    }

    /// Set config for a session
    pub fn set_default_session_config(
        &self,
        session_config: &slim_session::SessionConfig,
    ) -> Result<(), SessionError> {
        // set the session config
        self.session_layer
            .set_default_session_config(session_config)
    }

    /// Get default session config
    pub fn get_default_session_config(
        &self,
        session_type: slim_session::SessionType,
    ) -> Result<slim_session::SessionConfig, SessionError> {
        // get the default session config
        self.session_layer.get_default_session_config(session_type)
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
        msg.insert_metadata(SLIM_IDENTITY.to_string(), identity);

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

        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(ServiceError::SessionError)?;

        // Set the ID in the name to be the one of this app
        let name = name.clone().with_id(self.session_layer.app_id());

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };

        let msg = Message::new_subscribe(&self.app_name, &name, &identity, header);

        // Subscribe
        self.send_message_without_context(msg).await?;

        // Register the subscription
        self.session_layer.add_app_name(name);

        Ok(())
    }

    /// Unsubscribe the app
    pub async fn unsubscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!("unsubscribe from {} - {:?}", name, conn);

        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(ServiceError::SessionError)?;

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };

        let msg = Message::new_subscribe(&self.app_name, name, &identity, header);

        // Unsubscribe
        self.send_message_without_context(msg).await?;

        // Remove the subscription
        self.session_layer.remove_app_name(name);

        Ok(())
    }

    /// Set a route towards another app
    pub async fn set_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!("set route: {} - {:?}", name, conn);

        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(ServiceError::SessionError)?;

        // send a message with subscription from
        let msg = Message::new_subscribe(
            &self.app_name,
            name,
            &identity,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message_without_context(msg).await
    }

    pub async fn remove_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!("unset route to {} - {}", name, conn);

        let identity = self
            .session_layer
            .get_identity_token()
            .map_err(ServiceError::SessionError)?;

        //  send a message with unsubscription from
        let msg = Message::new_unsubscribe(
            &self.app_name,
            name,
            &identity,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );

        self.send_message_without_context(msg).await
    }

    /// SLIM receiver loop
    pub(crate) fn process_messages(&self, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let app_name = self.app_name.clone();
        let session_layer = self.session_layer.clone();
        let token_clone = self.cancel_token.clone();
        let identity = self
            .session_layer
            .get_identity_token()
            .expect("error while getting local identiyt");

        tokio::spawn(async move {
            debug!("starting message processing loop for {}", app_name);

            // subscribe for local name running this loop
            let subscribe_msg = Message::new_subscribe(&app_name, &app_name, &identity, None);
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

#[cfg(test)]
mod tests {
    use super::*;
    use slim_session::point_to_point::PointToPointConfiguration;

    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::{
        api::{ApplicationPayload, ProtoMessage, ProtoSessionMessageType, ProtoSessionType},
        messages::{Name, utils::SLIM_IDENTITY},
    };
    
    #[allow(dead_code)]
    fn create_app() -> App<SharedSecret, SharedSecret> {
        let (tx_slim, _) = tokio::sync::mpsc::channel(128);
        let (tx_app, _) = tokio::sync::mpsc::channel(128);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        App::new(
            &name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        )
    }

    #[tokio::test]
    async fn test_remove_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let app = App::new(
            &name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );
        let session_config = PointToPointConfiguration::default();

        let ret = app
            .create_session(SessionConfig::PointToPoint(session_config), Some(1))
            .await;

        assert!(ret.is_ok());

        app.delete_session(&ret.unwrap().session().upgrade().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let session_layer = App::new(
            &name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let res = session_layer
            .create_session(
                SessionConfig::PointToPoint(PointToPointConfiguration::default()),
                None,
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let session_layer = App::new(
            &name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let res = session_layer
            .create_session(
                SessionConfig::PointToPoint(PointToPointConfiguration::default()),
                Some(1),
            )
            .await;
        assert!(res.is_ok());

        session_layer
            .delete_session(&res.unwrap().session().upgrade().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_session_weak_after_delete() {
        let (tx_slim, _) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let app = App::new(
            &name,
            SharedSecret::new("a", "group"),
            SharedSecret::new("a", "group"),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let session_ctx = app
            .create_session(
                SessionConfig::PointToPoint(PointToPointConfiguration::default()),
                Some(42),
            )
            .await
            .expect("failed to create session");

        // Obtain a strong reference to delete it explicitly
        let strong = session_ctx
            .session_arc()
            .expect("expected session to be alive");
        assert_eq!(strong.id(), 42);

        // Delete the session from the app (removes it from the pool)
        app.delete_session(&strong)
            .await
            .expect("failed to delete session");

        // Drop the last strong reference
        drop(strong);

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
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let identity = SharedSecret::new("a", "group");

        let app = App::new(
            &name,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let mut message = ProtoMessage::new_publish(
            &name,
            &Name::from_strings(["org", "ns", "type"]).with_id(0),
            "a",
            None,
            Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content()),
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        app.session_layer
            .handle_message_from_slim(message.clone())
            .await
            .expect_err("should fail as identity is not verified");

        // sleep to allow the message to be processed
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // As there is no identity, we should not get any message in the app
        assert!(rx_app.try_recv().is_err());

        // Add identity to message
        message.insert_metadata(SLIM_IDENTITY.to_string(), identity.get_token().unwrap());

        // Try again
        app.session_layer
            .handle_message_from_slim(message.clone())
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
        let (tx_slim, mut rx_slim) = tokio::sync::mpsc::channel(1);
        let (tx_app, _) = tokio::sync::mpsc::channel(1);
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let identity = SharedSecret::new("a", "group");

        let app = App::new(
            &name,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let session_config = PointToPointConfiguration::default();

        // create a new session
        let res = app
            .create_session(SessionConfig::PointToPoint(session_config), Some(1))
            .await
            .unwrap();

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let mut message = ProtoMessage::new_publish(
            &source,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            "local",
            None,
            Some(ApplicationPayload::new("msg", vec![0x1, 0x2, 0x3, 0x4]).as_content()),
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::PointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::Msg);

        let res = app
            .session_layer
            .handle_message_from_app(message.clone(), &res)
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
        use slim_session::point_to_point::PointToPointConfiguration;

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
                SharedSecret::new("a", "group"),
                SharedSecret::new("a", "group"),
            )
            .await
            .unwrap();

        let (publisher_app, _publisher_notifications) = service
            .create_app(
                &publisher_name,
                SharedSecret::new("a", "group"),
                SharedSecret::new("a", "group"),
            )
            .await
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

            println!("Creating session to peer: {}", name);
            let session_config = PointToPointConfiguration::default().with_peer_name(name.clone());
            let session_ctx = publisher_app
                .create_session(
                    slim_session::SessionConfig::PointToPoint(session_config),
                    None,
                )
                .await
                .unwrap();

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

            println!("Verifying session ID: {}", session_arc.source());

            // Check that the source matches is in sub_names_set
            let src = session_arc.source();
            println!("Session source: {}", src);
            assert!(sub_names_set.contains(src));

            // Verify it's a point-to-point session
            assert_eq!(session_arc.kind(), slim_session::SessionType::PointToPoint);

            // Verify the destination is the publisher app (from subscriber's perspective)
            let dst = session_arc.dst().unwrap();
            assert_eq!(dst, publisher_name);
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
        use slim_session::multicast::MulticastConfiguration;
        use std::collections::HashMap;

        // Create a service instance with unique name
        let service_name = format!("test-service-{}", config.test_name);
        let id = ID::new_with_name(Kind::new("slim").unwrap(), &service_name).unwrap();
        let service = Service::new(id);

        // Create moderator app
        let moderator_name = Name::from_strings(["org", "ns", config.moderator_suffix]).with_id(0);
        let (moderator_app, mut _moderator_notifications) = service
            .create_app(
                &moderator_name,
                SharedSecret::new("a", "group"),
                SharedSecret::new("a", "group"),
            )
            .await
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
                    SharedSecret::new("a", "group"),
                    SharedSecret::new("a", "group"),
                )
                .await
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
        let multicast_config = MulticastConfiguration::new(
            channel_name.clone(),
            Some(5),
            Some(std::time::Duration::from_millis(1000)),
            false,
            HashMap::new(),
        );

        let session_ctx = moderator_app
            .create_session(
                slim_session::SessionConfig::Multicast(multicast_config),
                None,
            )
            .await
            .unwrap();

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
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

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
            assert_eq!(session_arc.kind(), slim_session::SessionType::Multicast);

            // For multicast sessions, the destination is also the channel name
            let dst = session_arc.dst().unwrap();
            assert_eq!(dst, channel_name);

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
            participant_suffixes: vec!["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"],
        };

        run_multicast_test(config).await;
    }
}

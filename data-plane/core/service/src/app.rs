// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;
use std::sync::Arc;

// Third-party crates
use parking_lot::RwLock as SyncRwLock;
use tokio::sync::mpsc;
use tracing::{debug, error};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::Status;
use slim_datapath::api::ProtoMessage as Message;
use slim_datapath::api::{MessageType, SessionHeader, SlimHeader};
use slim_datapath::api::{ProtoSessionMessageType, ProtoSessionType};
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::{SLIM_IDENTITY, SlimHeaderFlags};

// Local crate
use crate::session::interceptor::{IdentityInterceptor, SessionInterceptorProvider};
use crate::session::transmitter::Transmitter;
use crate::session::{
    AppChannelSender, Id, Info, MessageDirection, SessionConfig, SessionMessage, SlimChannelSender,
};
use crate::session::{SessionError, SessionLayer};
use crate::{ServiceError, session};

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
        app_name: &Name,
        identity_provider: P,
        identity_verifier: V,
        conn_id: u64,
        tx_slim: SlimChannelSender,
        tx_app: AppChannelSender,
        storage_path: std::path::PathBuf,
    ) -> Self {
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
        mut msg: Message,
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
            None => {
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
        }
    }

    /// Invite a new participant to a session
    pub async fn invite_participant(
        &self,
        destination: &Name,
        session_info: session::Info,
    ) -> Result<(), ServiceError> {
        let slim_header = Some(SlimHeader::new(
            self.session_layer.app_name(),
            destination,
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
        destination: &Name,
        session_info: session::Info,
    ) -> Result<(), ServiceError> {
        let slim_header = Some(SlimHeader::new(
            self.session_layer.app_name(),
            destination,
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
    pub async fn subscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!("subscribe {} - conn {:?}", name, conn);

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };
        let msg = Message::new_subscribe(self.session_layer.app_name(), name, header);
        self.send_message(msg, None).await
    }

    /// Unsubscribe the app
    pub async fn unsubscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        debug!("unsubscribe from {} - {:?}", name, conn);

        let header = if let Some(c) = conn {
            Some(SlimHeaderFlags::default().with_forward_to(c))
        } else {
            Some(SlimHeaderFlags::default())
        };
        let msg = Message::new_subscribe(self.session_layer.app_name(), name, header);
        self.send_message(msg, None).await
    }

    /// Set a route towards another app
    pub async fn set_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!("set route: {} - {:?}", name, conn);

        // send a message with subscription from
        let msg = Message::new_subscribe(
            self.session_layer.app_name(),
            name,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(msg, None).await
    }

    pub async fn remove_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        debug!("unset route to {} - {}", name, conn);

        //  send a message with unsubscription from
        let msg = Message::new_unsubscribe(
            self.session_layer.app_name(),
            name,
            Some(SlimHeaderFlags::default().with_recv_from(conn)),
        );
        self.send_message(msg, None).await
    }

    /// Publish a message to a specific connection
    pub async fn publish_to(
        &self,
        session_info: session::Info,
        name: &Name,
        forward_to: u64,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            session_info,
            name,
            SlimHeaderFlags::default().with_forward_to(forward_to),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message to a specific app name
    pub async fn publish(
        &self,
        session_info: session::Info,
        name: &Name,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        self.publish_with_flags(
            session_info,
            name,
            SlimHeaderFlags::default(),
            blob,
            payload_type,
            metadata,
        )
        .await
    }

    /// Publish a message with specific flags
    pub async fn publish_with_flags(
        &self,
        session_info: session::Info,
        name: &Name,
        flags: SlimHeaderFlags,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        debug!("sending publication to {} - Flags: {}", name, flags);

        let ct = match payload_type {
            Some(ct) => ct,
            None => "msg".to_string(),
        };

        let mut msg =
            Message::new_publish(self.session_layer.app_name(), name, Some(flags), &ct, blob);

        if let Some(map) = metadata {
            msg.set_metadata_map(map);
        }

        self.send_message(msg, Some(session_info)).await
    }

    /// SLIM receiver loop
    pub(crate) fn process_messages(&self, mut rx: mpsc::Receiver<Result<Message, Status>>) {
        let app_name = self.session_layer.app_name().clone();
        let session_layer = self.session_layer.clone();
        let token_clone = self.cancel_token.clone();

        tokio::spawn(async move {
            debug!("starting message processing loop for {}", app_name);

            // subscribe for local name running this loop
            let subscribe_msg = Message::new_subscribe(&app_name, &app_name, None);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::point_to_point::PointToPointConfiguration;

    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::{
        api::ProtoMessage,
        messages::{Name, utils::SLIM_IDENTITY},
    };

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
    async fn test_create_app() {
        let app = create_app();

        assert!(app.session_layer.is_pool_empty().await);
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

        app.delete_session(1).await.unwrap();
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

        session_layer.delete_session(1).await.unwrap();

        // try to delete a non-existing session
        let res = session_layer.delete_session(1).await;
        assert!(res.is_err());
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
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let session_config = PointToPointConfiguration::default();

        // create a new session
        let res = app
            .create_session(SessionConfig::PointToPoint(session_config), Some(1))
            .await;
        assert!(res.is_ok());

        let mut message = ProtoMessage::new_publish(
            &name,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::SessionPointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::P2PMsg);

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
        let name = Name::from_strings(["org", "ns", "type"]).with_id(0);

        let identity = SharedSecret::new("a", "group");

        let app = App::new(
            &name,
            identity.clone(),
            identity.clone(),
            0,
            tx_slim.clone(),
            tx_app.clone(),
            std::path::PathBuf::from("/tmp/test_storage"),
        );

        let session_config = PointToPointConfiguration::default();

        // create a new session
        let res = app
            .create_session(SessionConfig::PointToPoint(session_config), Some(1))
            .await;
        assert!(res.is_ok());

        let source = Name::from_strings(["cisco", "default", "local"]).with_id(0);

        let mut message = ProtoMessage::new_publish(
            &source,
            &Name::from_strings(["cisco", "default", "remote"]).with_id(0),
            None,
            "msg",
            vec![0x1, 0x2, 0x3, 0x4],
        );

        // set the session id in the message
        let header = message.get_session_header_mut();
        header.session_id = 1;
        header.set_session_type(ProtoSessionType::SessionPointToPoint);
        header.set_session_message_type(ProtoSessionMessageType::P2PMsg);

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

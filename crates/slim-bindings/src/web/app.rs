// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Browser implementation of the shared UniFFI `App` API.
//!
//! Native bindings construct an app through `slim-service`.  That crate is
//! deliberately native-only, so browser builds assemble the equivalent
//! client-side pieces directly: a WebSocket `MessageProcessor`, a
//! `SessionLayer`, and its subscription manager.  The public objects returned
//! to TypeScript are still the common `Name`, `Session`, and
//! `CompletionHandle` bindings.

use std::sync::Arc;

use crate::{CompletionHandle, Name, Session, SessionConfig, SlimError};

use slim_session::Direction as CoreDirection;

/// Direction in which an application exchanges session data.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum Direction {
    Send,
    Recv,
    Bidirectional,
    None,
}

impl From<Direction> for CoreDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Send => CoreDirection::Send,
            Direction::Recv => CoreDirection::Recv,
            Direction::Bidirectional => CoreDirection::Bidirectional,
            Direction::None => CoreDirection::None,
        }
    }
}

/// A newly-created session and the handle that reports establishment.
#[derive(uniffi::Record)]
pub struct SessionWithCompletion {
    pub session: Arc<Session>,
    pub completion: Arc<CompletionHandle>,
}

/// Browser-backed SLIM application.
///
/// The native placeholder is used only while UniFFI extracts metadata for the
/// WASM generator.  `ubrn` then compiles the same exported API for wasm32,
/// where the real browser fields are enabled.
#[derive(uniffi::Object)]
pub struct App {
    #[cfg(target_arch = "wasm32")]
    processor: slim_datapath::message_processing::MessageProcessor,
    #[cfg(target_arch = "wasm32")]
    session_layer: Arc<
        slim_session::SessionLayer<
            slim_auth::shared_secret::SharedSecret,
            slim_auth::shared_secret::SharedSecret,
        >,
    >,
    #[cfg(target_arch = "wasm32")]
    subscription_manager: slim_session::subscription_manager::SubscriptionManager,
    #[cfg(target_arch = "wasm32")]
    app_name: slim_datapath::api::ProtoName,
    #[cfg(target_arch = "wasm32")]
    remote_connection: u64,
    #[cfg(target_arch = "wasm32")]
    notification_rx: tokio::sync::RwLock<
        tokio::sync::mpsc::Receiver<Result<slim_session::Notification, slim_session::SessionError>>,
    >,
    #[cfg(target_arch = "wasm32")]
    route_ids: parking_lot::Mutex<std::collections::HashMap<String, u64>>,

    #[cfg(not(target_arch = "wasm32"))]
    _metadata_only: (),
}

#[uniffi::export]
impl App {
    /// Connect a browser application to a SLIM WebSocket endpoint.
    ///
    /// `endpoint` must use `ws://` or `wss://`.  When supplied, `token` is
    /// appended as the `token` query parameter used by the SLIM WebSocket
    /// server.  Browser bindings currently support shared-secret identity,
    /// which is portable and backed by WebCrypto-compatible SLIM components.
    #[uniffi::constructor]
    pub async fn connect_with_secret(
        endpoint: String,
        token: Option<String>,
        name: Arc<Name>,
        secret: String,
        direction: Direction,
    ) -> Result<Arc<Self>, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            Self::connect_browser(endpoint, token, name, secret, direction).await
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (endpoint, token, name, secret, direction);
            Err(browser_only_error())
        }
    }

    /// Fully-qualified application name, including its derived identity ID.
    pub fn name(&self) -> Result<Arc<Name>, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(Arc::new(Name::from(&self.app_name)))
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(browser_only_error())
        }
    }

    /// Application ID formatted as a UUID string.
    pub fn id(&self) -> Result<String, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(self.app_name.string_id())
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(browser_only_error())
        }
    }

    /// Connection ID of the upstream browser WebSocket.
    pub fn remote_connection_id(&self) -> Result<u64, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(self.remote_connection)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(browser_only_error())
        }
    }

    /// Create a session without blocking the browser event loop.
    pub async fn create_session_async(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<SessionWithCompletion, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            let destination = destination.as_ref().into();
            let (context, completion) = self
                .session_layer
                .create_session(config.into(), self.app_name.clone(), destination, None)
                .await
                .map_err(session_error)?;

            Ok(SessionWithCompletion {
                session: Arc::new(Session::new(context)),
                completion: Arc::new(CompletionHandle::from(completion)),
            })
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (config, destination);
            Err(browser_only_error())
        }
    }

    /// Create a session and await its initial handshake.
    pub async fn create_session_and_wait_async(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<Arc<Session>, SlimError> {
        let result = self.create_session_async(config, destination).await?;
        result.completion.wait_async().await?;
        Ok(result.session)
    }

    /// Delete a session and return its completion handle.
    pub async fn delete_session_async(
        &self,
        session: Arc<Session>,
    ) -> Result<Arc<CompletionHandle>, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            let controller = session
                .session
                .upgrade()
                .ok_or_else(|| SlimError::SessionError {
                    message: "Session already closed or dropped".to_string(),
                })?;
            let completion = self
                .session_layer
                .remove_session(controller.id())
                .map_err(session_error)?;
            Ok(Arc::new(CompletionHandle::from(completion)))
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = session;
            Err(browser_only_error())
        }
    }

    /// Delete a session and await shutdown.
    pub async fn delete_session_and_wait_async(
        &self,
        session: Arc<Session>,
    ) -> Result<(), SlimError> {
        self.delete_session_async(session).await?.wait_async().await
    }

    /// Subscribe the local app to a name and wait for the acknowledgement.
    pub async fn subscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            self.subscribe_name(name.as_ref().into(), connection_id)
                .await
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (name, connection_id);
            Err(browser_only_error())
        }
    }

    /// Remove a previously-created subscription.
    pub async fn unsubscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};

            let name = name.as_ref().into();
            let subscription_id = self.session_layer.remove_app_name(&name).ok_or_else(|| {
                SlimError::InvalidArgument {
                    message: format!("no subscription exists for {name}"),
                }
            })?;
            let ack = self
                .subscription_manager
                .unsubscribe(&self.app_name, &name, subscription_id, connection_id)
                .await
                .map_err(subscription_error)?;
            SubscriptionManager::await_ack(ack)
                .await
                .map_err(subscription_error)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (name, connection_id);
            Err(browser_only_error())
        }
    }

    /// Route traffic for `name` through a SLIM connection.
    pub async fn set_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};

            let name = name.as_ref().into();
            let (subscription_id, ack) = self
                .subscription_manager
                .set_route(&self.app_name, &name, connection_id)
                .await
                .map_err(subscription_error)?;
            SubscriptionManager::await_ack(ack)
                .await
                .map_err(subscription_error)?;
            self.route_ids
                .lock()
                .insert(route_key(&name, connection_id), subscription_id);
            Ok(())
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (name, connection_id);
            Err(browser_only_error())
        }
    }

    /// Route traffic through the application's upstream WebSocket.
    pub async fn set_route_via_upstream_async(&self, name: Arc<Name>) -> Result<(), SlimError> {
        let connection_id = self.remote_connection_id()?;
        self.set_route_async(name, connection_id).await
    }

    /// Remove a route previously installed with `set_route_async`.
    pub async fn remove_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};

            let name = name.as_ref().into();
            let subscription_id = self
                .route_ids
                .lock()
                .remove(&route_key(&name, connection_id))
                .ok_or_else(|| SlimError::InvalidArgument {
                    message: format!("no route exists for {name} on connection {connection_id}"),
                })?;
            let ack = self
                .subscription_manager
                .remove_route(&self.app_name, &name, subscription_id, connection_id)
                .await
                .map_err(subscription_error)?;
            SubscriptionManager::await_ack(ack)
                .await
                .map_err(subscription_error)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (name, connection_id);
            Err(browser_only_error())
        }
    }

    /// Wait for an incoming session notification.
    pub async fn listen_for_session_async(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<Arc<Session>, SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            use slim_session::Notification;

            let mut receiver = self.notification_rx.write().await;
            let receive = receiver.recv();
            let notification = if let Some(timeout) = timeout {
                futures::pin_mut!(receive);
                let delay = tokio::time::sleep(timeout);
                futures::pin_mut!(delay);
                match futures::future::select(receive, delay).await {
                    futures::future::Either::Left((notification, _)) => notification,
                    futures::future::Either::Right(_) => return Err(SlimError::Timeout),
                }
            } else {
                receive.await
            }
            .ok_or_else(|| SlimError::ReceiveError {
                message: "application channel closed".to_string(),
            })?;

            match notification.map_err(session_error)? {
                Notification::NewSession(context) => Ok(Arc::new(Session::new(context))),
                Notification::NewMessage(_) => Err(SlimError::ReceiveError {
                    message: "received a message while waiting for a new session".to_string(),
                }),
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = timeout;
            Err(browser_only_error())
        }
    }

    /// Disconnect the upstream WebSocket.
    pub fn disconnect(&self) -> Result<(), SlimError> {
        #[cfg(target_arch = "wasm32")]
        {
            self.processor
                .disconnect(self.remote_connection)
                .map(|_| ())
                .map_err(|error| SlimError::ServiceError {
                    message: format!("failed to disconnect browser WebSocket: {error}"),
                })
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(browser_only_error())
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl App {
    async fn connect_browser(
        endpoint: String,
        token: Option<String>,
        name: Arc<Name>,
        secret: String,
        direction: Direction,
    ) -> Result<Arc<Self>, SlimError> {
        use slim_auth::shared_secret::SharedSecret;
        use slim_config::client::ClientConfig;
        use slim_datapath::api::ProtoName;
        use slim_datapath::message_processing::MessageProcessor;
        use slim_session::SessionLayer;

        validate_websocket_endpoint(&endpoint)?;
        let endpoint = build_endpoint(&endpoint, token.as_deref());
        let base_name: ProtoName = name.as_ref().into();
        let identity_name = name.to_string();
        let identity = SharedSecret::new(&identity_name, &secret).map_err(auth_error)?;
        let app_name = derive_app_name(&base_name, &identity)?;

        let processor = MessageProcessor::new();
        let (_connection_handle, remote_connection) = processor
            .connect(ClientConfig::with_endpoint(&endpoint), None, None)
            .await
            .map_err(|error| SlimError::ServiceError {
                message: format!("failed to connect browser WebSocket: {error}"),
            })?;
        let (_local_connection, tx_slim, rx_slim) = processor
            .register_local_connection(false)
            .map_err(|error| SlimError::ServiceError {
                message: format!("failed to register browser application: {error}"),
            })?;

        let (tx_app, notification_rx) = tokio::sync::mpsc::channel(128);
        let session_layer = Arc::new(SessionLayer::new(
            app_name.clone(),
            identity.clone(),
            identity,
            remote_connection,
            tx_slim,
            tx_app,
            direction.into(),
            "slim/web".to_string(),
        ));
        let subscription_manager = session_layer.subscription_manager();

        spawn_receive_loop(
            session_layer.clone(),
            subscription_manager.clone(),
            app_name.clone(),
            rx_slim,
        );

        let app = Arc::new(Self {
            processor,
            session_layer,
            subscription_manager,
            app_name,
            remote_connection,
            notification_rx: tokio::sync::RwLock::new(notification_rx),
            route_ids: parking_lot::Mutex::new(std::collections::HashMap::new()),
        });

        // Publish the app's own subscription through the upstream connection.
        app.subscribe_name(app.app_name.clone(), Some(remote_connection))
            .await?;
        Ok(app)
    }

    async fn subscribe_name(
        &self,
        name: slim_datapath::api::ProtoName,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        use slim_session::subscription_manager::{SubscriptionManager, SubscriptionOps};

        let name_with_id = name.clone().with_id(self.session_layer.app_id());
        let (subscription_id, ack) = self
            .subscription_manager
            .subscribe(&self.app_name, &name_with_id, connection_id)
            .await
            .map_err(subscription_error)?;
        SubscriptionManager::await_ack(ack)
            .await
            .map_err(subscription_error)?;
        self.session_layer
            .add_app_name(name_with_id, subscription_id);
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_receive_loop(
    session_layer: Arc<
        slim_session::SessionLayer<
            slim_auth::shared_secret::SharedSecret,
            slim_auth::shared_secret::SharedSecret,
        >,
    >,
    subscription_manager: slim_session::subscription_manager::SubscriptionManager,
    app_name: slim_datapath::api::ProtoName,
    mut receiver: tokio::sync::mpsc::Receiver<
        Result<slim_datapath::api::ProtoMessage, slim_datapath::Status>,
    >,
) {
    use slim_datapath::api::MessageType;
    use slim_session::subscription_manager::SubscriptionOps;

    wasm_bindgen_futures::spawn_local(async move {
        // Register the local delivery route.  The same receive loop resolves
        // the acknowledgement emitted by the local datapath.
        let _ = subscription_manager
            .subscribe(&app_name, &app_name, None)
            .await;

        while let Some(item) = receiver.recv().await {
            match item {
                Ok(message) => match message.message_type.as_ref() {
                    Some(MessageType::Publish(_)) => {
                        if let Err(error) = session_layer.handle_message_from_slim(message).await {
                            match error {
                                slim_session::SessionError::SubscriptionNotFound(_) => {}
                                other => {
                                    tracing::error!(%other, "failed to handle browser SLIM message");
                                }
                            }
                        }
                    }
                    Some(MessageType::SubscriptionAck(ack)) => {
                        subscription_manager.resolve_ack(ack);
                    }
                    _ => {}
                },
                Err(error) => {
                    tracing::error!(%error, "browser SLIM transport closed");
                    break;
                }
            }
        }
    });
}

#[cfg(any(target_arch = "wasm32", test))]
fn build_endpoint(endpoint: &str, token: Option<&str>) -> String {
    match token {
        Some(token) if !token.is_empty() => {
            let separator = if endpoint.contains('?') { '&' } else { '?' };
            format!("{endpoint}{separator}token={token}")
        }
        _ => endpoint.to_string(),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_websocket_endpoint(endpoint: &str) -> Result<(), SlimError> {
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        Ok(())
    } else {
        Err(SlimError::InvalidArgument {
            message: format!("browser endpoint must use ws:// or wss://, got {endpoint:?}"),
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn derive_app_name(
    base: &slim_datapath::api::ProtoName,
    identity: &slim_auth::shared_secret::SharedSecret,
) -> Result<slim_datapath::api::ProtoName, SlimError> {
    use slim_auth::traits::TokenProvider;
    use slim_datapath::api::NameId;

    let token_id = identity.get_id().map_err(auth_error)?;
    let mut id = twox_hash::XxHash3_128::oneshot(token_id.as_bytes());
    if NameId::is_reserved_id(id) {
        id %= u128::MAX - NameId::RESERVED_IDS;
    }
    Ok(base.clone().with_id(id))
}

#[cfg(target_arch = "wasm32")]
fn route_key(name: &slim_datapath::api::ProtoName, connection_id: u64) -> String {
    format!("{name}@{connection_id}")
}

#[cfg(target_arch = "wasm32")]
fn auth_error(error: impl std::fmt::Display) -> SlimError {
    SlimError::AuthError {
        message: error.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
fn session_error(error: impl std::fmt::Display) -> SlimError {
    SlimError::SessionError {
        message: error.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
fn subscription_error(error: impl std::fmt::Display) -> SlimError {
    SlimError::ServiceError {
        message: error.to_string(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_only_error() -> SlimError {
    SlimError::InternalError {
        message: "the web binding implementation is available only on wasm32".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_auth_token_to_websocket_endpoint() {
        assert_eq!(
            build_endpoint("wss://slim.example/ws", Some("jwt")),
            "wss://slim.example/ws?token=jwt"
        );
        assert_eq!(
            build_endpoint("wss://slim.example/ws?x=1", Some("jwt")),
            "wss://slim.example/ws?x=1&token=jwt"
        );
    }

    #[test]
    fn validates_browser_websocket_schemes() {
        assert!(validate_websocket_endpoint("ws://localhost:46357").is_ok());
        assert!(validate_websocket_endpoint("wss://slim.example").is_ok());
        assert!(validate_websocket_endpoint("http://slim.example").is_err());
    }
}

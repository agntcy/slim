// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Bindings adapter that bridges the core App API with language-bindings interface
//!
//! This module provides an adapter layer that converts between the core App API
//! and the interface expected by bindings, handling type conversions and
//! API differences.
//!
//! # Usage
//!
//! The `AppAdapter` wraps a core `App` instance and provides a bindings-compatible
//! interface. This allows Python bindings to use the core functionality without
//! directly depending on the lower-level App API.
//!
//! ## Basic Usage
//!
//! ```rust
//! # tokio_test::block_on(async {
//! use slim_service::{Service, ServiceBuilder};
//! use slim_config::component::ComponentBuilder;
//! use slim_service::bindings::AppAdapter;
//! use slim_auth::shared_secret::SharedSecret;
//! use slim_datapath::messages::Name;
//!
//! // Create authentication components
//! let provider = SharedSecret::new("myapp", "my_secret");
//! let verifier = SharedSecret::new("myapp", "my_secret");
//!
//! // Create service instance (handles message processing and connections)
//! let service = Service::builder().build("svc-0".to_string()).expect("failed to create service");
//!
//! // Create app name
//! let app_name = Name::from_strings(["org", "ns", "svc"]);
//!
//! // Create adapter using service
//! let adapter = AppAdapter::new_with_service(
//!     &service,
//!     app_name,
//!     provider,
//!     verifier,
//! ).await.expect("failed to create app adapter");
//!# })
//! ```
//!
//! ## Construction
//!
//! The adapter can be created in two ways:
//!
//! 1. **Direct Constructor**: `AppAdapter::create()` - Creates App using Service
//! 2. **Builder Pattern**: `AppAdapter::builder()` or `AppAdapterBuilder::new()` - More flexible configuration
//!
//! ## Key Differences from App API
//!
//! - **Type Adapters**: Converts between core types and PyService types
//! - **Session Management**: Provides async listening for new sessions
//! - **Message Handling**: Bridges message reception between layers
//! - **Error Mapping**: Maps core errors to service-level errors
//!
//! ## Features
//!
//! - **Session Creation/Deletion**: Create and manage communication sessions
//! - **Publish/Subscribe**: Message routing and delivery
//! - **Route Management**: Set up communication paths between endpoints
//! - **Multicast Support**: Group communication with invite/remove operations
//! - **Async Operations**: Full async/await support for all operations

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_session::context::SessionContext;
use slim_session::{Notification, Session, SessionConfig, SessionError};

use crate::app::App;
use crate::errors::ServiceError;
use crate::service::Service;

/// Adapter that bridges the App API with generic language-bindings interface
pub struct AppAdapter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// The underlying App instance
    app: Arc<App<P, V>>,

    /// Channel receiver for notifications from the app
    notification_rx: Arc<RwLock<mpsc::Receiver<Result<Notification<P, V>, SessionError>>>>,

    /// Cancellation token for cleanup
    cancel_token: CancellationToken,
}

impl<P, V> AppAdapter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new AppAdapter wrapping the given App
    pub fn new_with_app(
        app: App<P, V>,
        notification_rx: mpsc::Receiver<Result<Notification<P, V>, SessionError>>,
    ) -> Self {
        Self {
            app: Arc::new(app),
            notification_rx: Arc::new(RwLock::new(notification_rx)),
            cancel_token: CancellationToken::new(),
        }
    }

    /// Create a new AppAdapter using a Service instance to create the App
    pub async fn create(
        service: &Service,
        app_name: Name,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<Self, ServiceError> {
        // Use Service to create the App and get the notification receiver
        let (app, rx_app) = service
            .create_app(&app_name, identity_provider, identity_verifier)
            .await?;

        Ok(Self::new_with_app(app, rx_app))
    }

    /// Get the app ID (derived from name)
    pub fn id(&self) -> u64 {
        self.app.app_name().id()
    }

    /// Get the app name
    pub fn name(&self) -> &Name {
        self.app.app_name()
    }

    /// Create a new AppAdapterBuilder
    pub fn builder() -> AppAdapterBuilder<P, V> {
        AppAdapterBuilder::new()
    }

    /// Create a new session with the given configuration
    pub async fn create_session(
        &self,
        session_config: SessionConfig,
    ) -> Result<SessionContext<P, V>, SessionError> {
        self.app.create_session(session_config, None).await
    }

    /// Delete a session by its context
    pub async fn delete_session(&self, session: &Session<P, V>) -> Result<(), SessionError> {
        self.app.delete_session(session).await
    }

    /// Set the default session configuration
    pub fn set_default_session_config(
        &self,
        session_config: &slim_session::SessionConfig,
    ) -> Result<(), SessionError> {
        self.app.set_default_session_config(session_config)
    }

    /// Get the default session configuration for a given session type
    pub fn get_default_session_config(
        &self,
        session_type: slim_session::SessionType,
    ) -> Result<slim_session::SessionConfig, SessionError> {
        self.app.get_default_session_config(session_type)
    }

    /// Subscribe to a name with optional connection ID
    pub async fn subscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        self.app.subscribe(name, conn).await
    }

    /// Unsubscribe from a name with optional connection ID
    pub async fn unsubscribe(&self, name: &Name, conn: Option<u64>) -> Result<(), ServiceError> {
        self.app.unsubscribe(name, conn).await
    }

    /// Set a route to a name for a specific connection
    pub async fn set_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        self.app.set_route(name, conn).await
    }

    /// Remove a route to a name for a specific connection
    pub async fn remove_route(&self, name: &Name, conn: u64) -> Result<(), ServiceError> {
        self.app.remove_route(name, conn).await
    }

    /// Listen for new sessions from the app
    pub async fn listen_for_session(&self) -> Result<SessionContext<P, V>, ServiceError> {
        let mut rx = self.notification_rx.write().await;

        tokio::select! {
            notification = rx.recv() => {
                if notification.is_none() {
                    return Err(ServiceError::ReceiveError("application channel closed".to_string()));
                }

                let notification = notification.unwrap();
                match notification {
                    Ok(Notification::NewSession(ctx)) => {
                        Ok(ctx)
                    }
                    Ok(Notification::NewMessage(_)) => {
                        Err(ServiceError::ReceiveError("received unexpected message notification while listening for session".to_string()))
                    }
                    Err(e) => {
                        Err(ServiceError::ReceiveError(format!("failed to receive session notification: {}", e)))
                    }
                }
            }
            _ = self.cancel_token.cancelled() => {
                Err(ServiceError::ReceiveError("adapter was cancelled".to_string()))
            }
        }
    }

    /// Publish a message through a session
    pub async fn publish(
        &self,
        session: &Session<P, V>,
        name: &Name,
        fanout: u32,
        blob: Vec<u8>,
        conn_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), ServiceError> {
        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        session
            .publish_with_flags(name, flags, blob, payload_type, metadata)
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    /// Invite a peer to join a session
    pub async fn invite(
        &self,
        session: &Session<P, V>,
        destination: &Name,
    ) -> Result<(), SessionError> {
        session.invite_participant(destination).await
    }

    /// Remove a peer from a session
    pub async fn remove(
        &self,
        session: &Session<P, V>,
        destination: &Name,
    ) -> Result<(), SessionError> {
        session.remove_participant(destination).await
    }

    /// Get the underlying App instance (for advanced usage)
    pub fn app(&self) -> &App<P, V> {
        &self.app
    }

    /// Cancel all operations
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl<P, V> Drop for AppAdapter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Builder for creating AppAdapter instances
pub struct AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    app_name: Option<Name>,
    identity_provider: Option<P>,
    identity_verifier: Option<V>,
}

impl<P, V> AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            app_name: None,
            identity_provider: None,
            identity_verifier: None,
        }
    }

    /// Set the app name
    pub fn with_name(mut self, name: Name) -> Self {
        self.app_name = Some(name);
        self
    }

    /// Set the identity provider
    pub fn with_identity_provider(mut self, provider: P) -> Self {
        self.identity_provider = Some(provider);
        self
    }

    /// Set the identity verifier
    pub fn with_identity_verifier(mut self, verifier: V) -> Self {
        self.identity_verifier = Some(verifier);
        self
    }

    /// Build the AppAdapter using a Service instance
    pub async fn build(self, service: &Service) -> Result<AppAdapter<P, V>, ServiceError> {
        let app_name = self
            .app_name
            .ok_or_else(|| ServiceError::ConfigError("app name is required".to_string()))?;

        let identity_provider = self.identity_provider.ok_or_else(|| {
            ServiceError::ConfigError("identity provider is required".to_string())
        })?;

        let identity_verifier = self.identity_verifier.ok_or_else(|| {
            ServiceError::ConfigError("identity verifier is required".to_string())
        })?;

        // Use Service to create the App and get the notification receiver
        let (app, rx_app) = service
            .create_app(&app_name, identity_provider, identity_verifier)
            .await?;

        Ok(AppAdapter::new_with_app(app, rx_app))
    }
}

impl<P, V> Default for AppAdapterBuilder<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

// Tests are disabled to avoid complex dependency setup.
// In real usage, provide proper TokenProvider and Verifier implementations
// from slim_auth crate (e.g., SpiffeProvider, OidcVerifier, etc.)

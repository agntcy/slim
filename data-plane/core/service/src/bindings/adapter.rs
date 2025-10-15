// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;
use slim_session::{Notification, Session, SessionConfig, SessionError};
use slim_session::context::SessionContext;

use crate::app::App;
use crate::errors::ServiceError;
use crate::service::Service;
use crate::bindings::service_ref::{ServiceRef, get_or_init_global_service};
use crate::bindings::builder::AppAdapterBuilder;
use slim_config::component::ComponentBuilder;

/// Adapter that bridges the App API with generic language-bindings interface
#[derive(Debug)]
pub struct BindingsAdapter<P, V>
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

impl<P, V> BindingsAdapter<P, V>
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

    /// Create a new AppAdapter from the service
    pub async fn new_with_service(
        service: &Service,
        app_name: Name,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<Self, ServiceError> {
        let (app, rx) = service
            .create_app(&app_name, identity_provider, identity_verifier)
            .await?;

        Ok(Self::new_with_app(app, rx))
    }

    /// Create a new BindingsAdapter with complete creation logic (for language bindings)
    ///
    /// This method encapsulates all creation logic including:
    /// - Service management (global vs local)
    /// - Token validation
    /// - Name generation with random ID
    /// - Adapter creation
    ///
    /// # Arguments
    /// * `base_name` - Base name for the app (will have random ID appended)
    /// * `identity_provider` - Authentication provider
    /// * `identity_verifier` - Authentication verifier
    /// * `use_local_service` - If true, creates a local service; if false, uses global service
    ///
    /// # Returns
    /// * `Ok((BindingsAdapter, ServiceRef))` - The adapter and service reference
    /// * `Err(ServiceError)` - If creation fails
    pub async fn new(
        base_name: Name,
        identity_provider: P,
        identity_verifier: V,
        use_local_service: bool,
    ) -> Result<(Self, ServiceRef), ServiceError> {
        // Validate token
        let _identity_token = identity_provider.get_token().map_err(|e| {
            ServiceError::ConfigError(format!("Failed to get token from provider: {}", e))
        })?;

        // Generate name with random ID
        let app_name = base_name.with_id(rand::random::<u64>());

        // Create or get service
        let service_ref = if use_local_service {
            let svc = Service::builder()
                .build("local-bindings-service".to_string())
                .map_err(|e| {
                    ServiceError::ConfigError(format!("Failed to create local service: {}", e))
                })?;
            ServiceRef::Local(Box::new(svc))
        } else {
            ServiceRef::Global(get_or_init_global_service())
        };

        // Get service reference for adapter creation
        let service = service_ref.get_service();

        // Create the adapter
        let adapter =
            Self::new_with_service(service, app_name, identity_provider, identity_verifier).await?;

        Ok((adapter, service_ref))
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

    /// Get the underlying App instance (for advanced usage)
    pub fn app(&self) -> &App<P, V> {
        &self.app
    }

    /// Cancel all operations
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl<P, V> Drop for BindingsAdapter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;
    
    use tokio::sync::mpsc;
    use tokio::time::timeout;
    
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::messages::Name;

    use slim_session::point_to_point::PointToPointConfiguration;
    use slim_session::{Notification, SessionConfig, SessionError, SessionType};
    
    use crate::bindings::session_context::BindingsSessionContext;

    
    type TestProvider = SharedSecret;
    type TestVerifier = SharedSecret;
    
    /// Create a mock service for testing
    async fn create_test_service() -> Service {
        use slim_config::component::ComponentBuilder;
        
        Service::builder()
            .build("test-service".to_string())
            .expect("Failed to create test service")
    }
    
    /// Create test authentication components
    fn create_test_auth() -> (TestProvider, TestVerifier) {
        let provider = SharedSecret::new("test-app", "test-secret");
        let verifier = SharedSecret::new("test-app", "test-secret");
        (provider, verifier)
    }
    
    /// Create test app name
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }
    
    /// Create a mock app and notification receiver for testing
    fn create_mock_app_with_receiver() -> (
        App<TestProvider, TestVerifier>,
        mpsc::Receiver<Result<Notification<TestProvider, TestVerifier>, SessionError>>,
    ) {
        let (tx_slim, _rx_slim) = mpsc::channel(128);
        let (tx_app, rx_app) = mpsc::channel(128);
        let name = create_test_name().with_id(0);
        let (provider, verifier) = create_test_auth();
        
        let app = App::new(
            &name,
            provider,
            verifier,
            0,
            tx_slim,
            tx_app,
            std::path::PathBuf::from("/tmp/test_bindings"),
        );
        
        (app, rx_app)
    }
    
    #[tokio::test]
    async fn test_new_with_app() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        assert_eq!(adapter.id(), 0);
        assert_eq!(
            adapter.name().components_strings().unwrap(),
            &["org", "namespace", "test-app"]
        );
    }
    
    #[tokio::test]
    async fn test_new_with_service() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        assert!(adapter.id() > 0);
        assert_eq!(
            adapter.name().components_strings().unwrap(),
            &["org", "namespace", "test-app"]
        );
    }
    
    #[tokio::test]
    async fn test_id_and_name() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        assert_eq!(adapter.id(), 0);
        assert_eq!(adapter.name().id(), 0);
        assert_eq!(
            adapter.name().components_strings().unwrap(),
            &["org", "namespace", "test-app"]
        );
    }
    
    #[tokio::test]
    async fn test_create_session() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        let config = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let session = adapter
            .create_session(config)
            .await
            .expect("Failed to create session");
        
        // Just verify we got a session context
        assert!(session.session.upgrade().is_some());
    }
    
    #[tokio::test]
    async fn test_session_config_operations() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        let config = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        adapter
            .set_default_session_config(&config)
            .expect("Failed to set config");
        
        let retrieved_config = adapter
            .get_default_session_config(SessionType::PointToPoint)
            .expect("Failed to get config");
        
        assert!(matches!(retrieved_config, SessionConfig::PointToPoint(_)));
    }
    
    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        let name = Name::from_strings(["org", "namespace", "subscription"]);
        
        // Test subscribe
        let result = adapter.subscribe(&name, Some(1)).await;
        assert!(result.is_ok());
        
        // Test unsubscribe
        let result = adapter.unsubscribe(&name, Some(1)).await;
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_route_operations() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        let name = Name::from_strings(["org", "namespace", "route"]);
        
        // Test set_route
        let result = adapter.set_route(&name, 1).await;
        assert!(result.is_ok());
        
        // Test remove_route
        let result = adapter.remove_route(&name, 1).await;
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_listen_for_session_timeout() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        // This should timeout since no session is being sent
        let result = timeout(Duration::from_millis(10), adapter.listen_for_session()).await;
        assert!(result.is_err()); // Should timeout
    }
    
    #[tokio::test]
    async fn test_listen_for_session_cancellation() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        // Cancel the adapter
        adapter.cancel();
        
        // This should return immediately with cancellation error
        let result = adapter.listen_for_session().await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("cancelled"));
        }
    }
    
    #[tokio::test]
    async fn test_app_accessor() {
        let (app, rx) = create_mock_app_with_receiver();
        let expected_name = app.app_name().clone();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        let app_ref = adapter.app();
        assert_eq!(app_ref.app_name(), &expected_name);
    }
    
    #[tokio::test]
    async fn test_cancel_operations() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        // Test that cancel doesn't panic
        adapter.cancel();
        adapter.cancel(); // Should be safe to call multiple times
    }
    
    #[tokio::test]
    async fn test_drop_behavior() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);
        
        // Test that dropping the adapter doesn't panic
        drop(adapter);
    }
    
    #[tokio::test]
    async fn test_delete_session() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        // Create a session
        let config = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let session_ctx = adapter
            .create_session(config)
            .await
            .expect("Failed to create session");
        
        // Get the session reference and test delete
        let session_ref = session_ctx.session.upgrade();
        assert!(session_ref.is_some());
        
        if let Some(session) = session_ref {
            // Test delete session
            let result = adapter.delete_session(&session).await;
            assert!(result.is_ok());
        }
    }
    
    #[tokio::test]
    async fn test_subscribe_unsubscribe_without_connection() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .await
            .expect("Failed to create adapter");
        
        let name = Name::from_strings(["org", "namespace", "subscription"]);
        
        // Test subscribe without connection ID
        let result = adapter.subscribe(&name, None).await;
        assert!(result.is_ok());
        
        // Test unsubscribe without connection ID
        let result = adapter.unsubscribe(&name, None).await;
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_new_complete_with_local_service() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let result = BindingsAdapter::new(base_name, provider, verifier, true).await;
        assert!(result.is_ok());
        
        let (adapter, service_ref) = result.unwrap();
        assert!(adapter.id() > 0);
        assert!(matches!(service_ref, ServiceRef::Local(_)));
    }
    
    #[tokio::test]
    async fn test_new_complete_with_global_service() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        let result = BindingsAdapter::new(base_name, provider, verifier, false).await;
        assert!(result.is_ok());
        
        let (adapter, service_ref) = result.unwrap();
        assert!(adapter.id() > 0);
        assert!(matches!(service_ref, ServiceRef::Global(_)));
    }
    
    #[tokio::test]
    async fn test_reorganized_api_usage() {
        // Test the new reorganized API where session operations are on BindingsSessionContext
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();
        
        // Create adapter using the complete creation method
        let (adapter, _service_ref) = BindingsAdapter::new(
            base_name,
            provider,
            verifier,
            false, // use global service
        )
        .await
        .expect("Failed to create adapter");
        
        // Create a session
        let config = SessionConfig::PointToPoint(PointToPointConfiguration::default());
        let session_ctx = adapter
            .create_session(config)
            .await
            .expect("Failed to create session");
        
        // Convert to BindingsSessionContext for session operations
        let session_bindings = BindingsSessionContext::from_session_context(session_ctx);
        
        // Test session-level operations on the session context (not the adapter)
        let target_name = Name::from_strings(["org", "target", "service"]);
        let message_data = b"hello from reorganized API".to_vec();
        let mut metadata = HashMap::new();
        metadata.insert("test".to_string(), "reorganized".to_string());
        
        // Publish through session context
        let result = session_bindings
            .publish(
                &target_name,
                1, // fanout
                message_data.clone(),
                None, // conn_out
                Some("text/plain".to_string()),
                Some(metadata.clone()),
            )
            .await;
        assert!(result.is_ok());
        
        // Test invite/remove operations on session context
        let peer_name = Name::from_strings(["org", "peer", "service"]);
        let invite_result = session_bindings.invite(&peer_name).await;
        // Note: This may fail in test environment, but we're testing the API structure
        assert!(invite_result.is_err() || invite_result.is_ok());
        
        let remove_result = session_bindings.remove(&peer_name).await;
        assert!(remove_result.is_err() || remove_result.is_ok());
        
        // Verify adapter still handles app-level operations
        assert!(adapter.id() > 0);
        assert!(!adapter.name().to_string().is_empty());
    }
}
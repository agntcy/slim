// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::messages::Name;
use slim_session::context::SessionContext;
use slim_session::{Notification, SessionError};
use slim_session::{SessionConfig, session_controller::SessionController};

use crate::app::App;
use crate::bindings::builder::AppAdapterBuilder;
use crate::bindings::service_ref::{ServiceRef, get_or_init_global_service};
use crate::errors::ServiceError;
use crate::service::Service;
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
    notification_rx: Arc<RwLock<mpsc::Receiver<Result<Notification, SessionError>>>>,
}

impl<P, V> BindingsAdapter<P, V>
where
    P: TokenProvider + Send + Sync + Clone + 'static,
    V: Verifier + Send + Sync + Clone + 'static,
{
    /// Create a new AppAdapter wrapping the given App
    pub fn new_with_app(
        app: App<P, V>,
        notification_rx: mpsc::Receiver<Result<Notification, SessionError>>,
    ) -> Self {
        Self {
            app: Arc::new(app),
            notification_rx: Arc::new(RwLock::new(notification_rx)),
        }
    }

    /// Create a new AppAdapter from the service
    pub fn new_with_service(
        service: &Service,
        app_name: Name,
        identity_provider: P,
        identity_verifier: V,
    ) -> Result<Self, ServiceError> {
        let (app, rx) = service.create_app(&app_name, identity_provider, identity_verifier)?;

        Ok(Self::new_with_app(app, rx))
    }

    /// Create a new BindingsAdapter with complete creation logic (for language bindings)
    ///
    /// This method encapsulates all creation logic including:
    /// - Service management (global vs local)
    /// - Token validation
    /// - Name generation with token ID
    /// - Adapter creation
    ///
    /// # Arguments
    /// * `base_name` - Base name for the app (will have token ID appended)
    /// * `identity_provider` - Authentication provider
    /// * `identity_verifier` - Authentication verifier
    /// * `use_local_service` - If true, creates a local service; if false, uses global service
    ///
    /// # Returns
    /// * `Ok((BindingsAdapter, ServiceRef))` - The adapter and service reference
    /// * `Err(ServiceError)` - If creation fails
    pub fn new(
        base_name: Name,
        identity_provider: P,
        identity_verifier: V,
        use_local_service: bool,
    ) -> Result<(Self, ServiceRef), ServiceError> {
        // Validate token
        let _identity_token = identity_provider.get_token().map_err(|e| {
            ServiceError::ConfigError(format!("Failed to get token from provider: {}", e))
        })?;

        // Get ID from token and generate name with token ID
        let token_id = identity_provider.get_id().map_err(|e| {
            ServiceError::ConfigError(format!("Failed to get ID from token: {}", e))
        })?;

        // Use a hash of the token ID to convert to u64 for name generation
        let id_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };
        let app_name = base_name.with_id(id_hash);

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
            Self::new_with_service(service, app_name, identity_provider, identity_verifier)?;

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
        destination: Name,
    ) -> Result<(SessionContext, slim_session::CompletionHandle), SessionError> {
        self.app
            .create_session(session_config, destination, None)
            .await
    }

    /// Delete a session by its context and return a completion handle to await on
    pub fn delete_session(
        &self,
        session: &SessionController,
    ) -> Result<slim_session::CompletionHandle, SessionError> {
        self.app.delete_session(session)
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
    ///
    /// If `timeout` is `Some(duration)`, waits up to that duration for a new session
    /// before returning a timeout error. If `None`, waits indefinitely.
    pub async fn listen_for_session(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<SessionContext, ServiceError> {
        let mut rx = self.notification_rx.write().await;

        let recv_fut = rx.recv();
        let notification_opt = if let Some(dur) = timeout {
            match tokio::time::timeout(dur, recv_fut).await {
                Ok(n) => n,
                Err(_) => {
                    return Err(ServiceError::ReceiveError(
                        "listen_for_session timed out".to_string(),
                    ));
                }
            }
        } else {
            recv_fut.await
        };

        if notification_opt.is_none() {
            return Err(ServiceError::ReceiveError(
                "application channel closed".to_string(),
            ));
        }

        match notification_opt.unwrap() {
            Ok(Notification::NewSession(ctx)) => Ok(ctx),
            Ok(Notification::NewMessage(_)) => Err(ServiceError::ReceiveError(
                "received unexpected message notification while listening for session".to_string(),
            )),
            Err(e) => Err(ServiceError::ReceiveError(format!(
                "failed to receive session notification: {}",
                e
            ))),
        }
    }

    /// Get the underlying App instance (for advanced usage)
    pub fn app(&self) -> &App<P, V> {
        &self.app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use tokio::sync::mpsc;

    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::{api::ProtoSessionType, messages::Name};

    use slim_session::{Notification, SessionConfig, SessionError};
    use slim_testing::utils::TEST_VALID_SECRET;

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
        let provider = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let verifier = SharedSecret::new("test-app", TEST_VALID_SECRET);
        (provider, verifier)
    }

    /// Create test app name
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }

    /// Create a mock app and notification receiver for testing
    fn create_mock_app_with_receiver() -> (
        App<TestProvider, TestVerifier>,
        mpsc::Receiver<Result<Notification, SessionError>>,
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
            adapter.name().components_strings(),
            &["org", "namespace", "test-app"]
        );
    }

    #[tokio::test]
    async fn test_new_with_service() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        assert!(adapter.id() > 0);
        assert_eq!(
            adapter.name().components_strings(),
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
            adapter.name().components_strings(),
            &["org", "namespace", "test-app"]
        );
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
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
        let result = adapter
            .listen_for_session(Some(Duration::from_millis(10)))
            .await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("timed out"));
        }
    }

    #[tokio::test]
    async fn test_listen_for_session_no_timeout() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);

        // Test that None timeout waits indefinitely (but we'll wrap it in a timeout for testing)
        // Use a timeout wrapper to prevent the test from hanging indefinitely
        let result =
            tokio::time::timeout(Duration::from_millis(100), adapter.listen_for_session(None))
                .await;

        // The operation should timeout since no session is being sent and we're not providing a timeout
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_listen_for_session_various_timeouts() {
        let (app, rx) = create_mock_app_with_receiver();
        let adapter = BindingsAdapter::new_with_app(app, rx);

        // Test very short timeout
        let result = adapter
            .listen_for_session(Some(Duration::from_nanos(1)))
            .await;
        assert!(result.is_err());

        // Test zero timeout
        let result = adapter.listen_for_session(Some(Duration::ZERO)).await;
        assert!(result.is_err());

        // Test reasonable timeout
        let start = std::time::Instant::now();
        let result = adapter
            .listen_for_session(Some(Duration::from_millis(100)))
            .await;
        let elapsed = start.elapsed();

        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(90)); // Allow some variance
        assert!(elapsed <= Duration::from_millis(200)); // But not too much
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
            .expect("Failed to create adapter");

        // Create a session
        let session_config = SessionConfig {
            session_type: ProtoSessionType::PointToPoint,
            initiator: true,
            ..Default::default()
        };
        let dst = Name::from_strings(["org", "ns", "dst"]);
        let (session_ctx, _completion_handle) = adapter
            .create_session(session_config, dst)
            .await
            .expect("Failed to create session");

        // Get the session reference and test delete
        let session_ref = session_ctx.session.upgrade();
        assert!(session_ref.is_some());

        if let Some(session) = session_ref {
            // Test delete session
            let result = adapter.delete_session(&session);
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe_without_connection() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
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

        let result = BindingsAdapter::new(base_name, provider, verifier, true);
        assert!(result.is_ok());

        let (adapter, service_ref) = result.unwrap();
        assert!(adapter.id() > 0);
        assert!(matches!(service_ref, ServiceRef::Local(_)));
    }

    #[tokio::test]
    async fn test_new_complete_with_global_service() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let result = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result.is_ok());

        let (adapter, service_ref) = result.unwrap();
        assert!(adapter.id() > 0);
        assert!(matches!(service_ref, ServiceRef::Global(_)));
    }

    #[tokio::test]
    async fn test_new_uses_token_id_for_name_generation() {
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get the token ID from the same provider instance before using it
        let token_id = provider.get_id().expect("Failed to get token ID");
        let expected_id = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };

        let result = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result.is_ok());

        let (adapter, _service_ref) = result.unwrap();

        // The ID should be derived from the token, not random
        let app_id = adapter.id();
        assert!(app_id > 0);
        assert_eq!(app_id, expected_id);
    }

    #[tokio::test]
    async fn test_deterministic_name_generation_same_token() {
        let base_name = create_test_name();
        // Use the same SharedSecret instances to ensure same token IDs
        let provider = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let verifier = SharedSecret::new("test-app", TEST_VALID_SECRET);

        // Create two adapters with the same authentication (should produce same ID)
        let result1 =
            BindingsAdapter::new(base_name.clone(), provider.clone(), verifier.clone(), false);
        assert!(result1.is_ok());
        let (adapter1, _) = result1.unwrap();

        let result2 = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result2.is_ok());
        let (adapter2, _) = result2.unwrap();

        // Both adapters should have the same ID since they use the same token
        assert_eq!(adapter1.id(), adapter2.id());
        assert_eq!(adapter1.name().id(), adapter2.name().id());
    }

    #[tokio::test]
    async fn test_different_tokens_produce_different_ids() {
        let base_name = create_test_name();

        // Create two different authentication providers
        let provider1 = SharedSecret::new("app1", "secret1-shared-secret-value-0123456789abcdef");
        let verifier1 = SharedSecret::new("app1", "secret1-shared-secret-value-0123456789abcdef");

        let provider2 = SharedSecret::new("app2", "secret2-shared-secret-value-0123456789abcdef");
        let verifier2 = SharedSecret::new("app2", "secret2-shared-secret-value-0123456789abcdef");

        let result1 = BindingsAdapter::new(base_name.clone(), provider1, verifier1, false);
        assert!(result1.is_ok());
        let (adapter1, _) = result1.unwrap();

        let result2 = BindingsAdapter::new(base_name, provider2, verifier2, false);
        assert!(result2.is_ok());
        let (adapter2, _) = result2.unwrap();

        // Different tokens should produce different IDs
        assert_ne!(adapter1.id(), adapter2.id());
        assert_ne!(adapter1.name().id(), adapter2.name().id());
    }

    #[tokio::test]
    async fn test_consistent_id_generation_multiple_calls() {
        // Test that multiple calls with the same SharedSecret instance produce consistent results
        let base_name = create_test_name();
        let provider = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let verifier = SharedSecret::new("test-app", TEST_VALID_SECRET);

        // Since SharedSecret instances are created separately, they will have different random suffixes
        // But we can test that the same instance produces consistent results
        let token_id = provider.get_id().expect("Failed to get ID");
        let expected_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };

        let result1 =
            BindingsAdapter::new(base_name.clone(), provider.clone(), verifier.clone(), false);
        let result2 = BindingsAdapter::new(base_name, provider, verifier, false);

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let (adapter1, _) = result1.unwrap();
        let (adapter2, _) = result2.unwrap();

        // Both should have the same computed hash ID since we used the same provider instance
        assert_eq!(adapter1.id(), expected_hash);
        assert_eq!(adapter2.id(), expected_hash);
        assert_eq!(adapter1.id(), adapter2.id());
    }

    #[tokio::test]
    async fn test_hash_id_generation() {
        // Test that the hash generation produces expected results
        let base_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        // Get the token ID and compute expected hash manually
        let token_id = provider.get_id().expect("Failed to get token ID");
        let expected_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token_id.hash(&mut hasher);
            hasher.finish()
        };

        let result = BindingsAdapter::new(base_name, provider, verifier, false);
        assert!(result.is_ok());

        let (adapter, _) = result.unwrap();
        assert_eq!(adapter.id(), expected_hash);
        assert_eq!(adapter.name().id(), expected_hash);
    }
}

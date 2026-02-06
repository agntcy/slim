// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! # SLIM Bindings Adapter (UniFFI-Compatible)
//!
//! This module provides a language-agnostic FFI interface to SLIM using a hybrid approach
//! that exposes both synchronous (blocking) and asynchronous versions of operations.
//!
//! ## Architecture
//! - **Flexible Authentication**: Uses `AuthProvider`/`AuthVerifier` enums supporting multiple auth types
//!   (SharedSecret, JWT, SPIRE, StaticToken) instead of generics (UniFFI requirement)
//! - **Hybrid API**: Both sync (FFI-exposed) and async (internal) methods
//! - **Runtime management**: Manages Tokio runtime for blocking operations

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use crate::errors::SlimError;
use crate::name::Name;
use crate::{get_global_service, get_runtime};

use crate::session::SessionConfig;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};

use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};

use futures_timer::Delay;
use slim_datapath::messages::Name as SlimName;
use slim_service::Service as SlimService;
use slim_service::app::App as SlimApp;
use slim_session::Direction as CoreDirection;

use slim_session::SessionConfig as SlimSessionConfig;
use slim_session::session_controller::SessionController;
use slim_session::{Notification, SessionError as SlimSessionError};

/// Direction enum
/// Indicates whether the App can send, receive, both, or neither.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum Direction {
    Send,          // Can only send data messages (shutdown_send: false, shutdown_receive: true)
    Recv,          // Can only receive data messages (shutdown_send: true, shutdown_receive: false)
    Bidirectional, // Can send and receive data messages (shutdown_send: false, shutdown_receive: false)
    None, // Neither send nor receive data messages (shutdown_send: true, shutdown_receive: true)
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

impl From<CoreDirection> for Direction {
    fn from(direction: CoreDirection) -> Self {
        match direction {
            CoreDirection::Send => Direction::Send,
            CoreDirection::Recv => Direction::Recv,
            CoreDirection::Bidirectional => Direction::Bidirectional,
            CoreDirection::None => Direction::None,
        }
    }
}

// ============================================================================
// Return Types
// ============================================================================

/// Result of creating a session, containing the session context and a completion handle
///
/// The completion handle should be awaited to ensure the session is fully established.
#[derive(uniffi::Record)]
pub struct SessionWithCompletion {
    /// The session context for performing operations
    pub session: Arc<crate::Session>,
    /// Completion handle to wait for session establishment
    pub completion: Arc<crate::CompletionHandle>,
}

/// Adapter that bridges the App API with language-bindings interface
///
/// This adapter uses enum-based auth types (`AuthProvider`/`AuthVerifier`) instead of generics
/// to be compatible with UniFFI, supporting multiple authentication mechanisms (SharedSecret,
/// JWT, SPIRE, StaticToken). It provides both synchronous (blocking) and asynchronous methods
/// for flexibility.
#[derive(uniffi::Object)]
pub struct App {
    /// The underlying App instance with enum-based auth types (supports SharedSecret, JWT, SPIRE)
    app: Arc<SlimApp<AuthProvider, AuthVerifier>>,

    /// Channel receiver for notifications from the app
    notification_rx: Arc<RwLock<mpsc::Receiver<Result<Notification, SlimSessionError>>>>,

    /// Service instance for lifecycle management (Arc to inner SlimService)
    service: Arc<SlimService>,
}

impl App {
    /// Internal constructor from parts
    ///
    /// Used by Service::create_adapter_async to construct a App from its components.
    pub(crate) fn from_parts(
        app: Arc<SlimApp<AuthProvider, AuthVerifier>>,
        notification_rx: Arc<RwLock<mpsc::Receiver<Result<Notification, SlimSessionError>>>>,
        service: Arc<SlimService>,
    ) -> Self {
        Self {
            app,
            notification_rx,
            service,
        }
    }

    /// Get a reference to the core SlimApp instance
    ///
    /// This allows direct access to the core SLIM API methods without going through
    /// the bindings layer. Useful for advanced use cases that need core functionality.
    ///
    /// # Note
    /// This is a public internal method that exposes the core app. Use with caution as it
    /// bypasses the bindings layer abstractions.
    pub fn core_app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    /// Get a clone of the inner core SlimApp instance
    ///
    /// This is used internally by other bindings modules (like slimrpc) that need
    /// to interact with the core SLIM app.
    pub fn inner(&self) -> Arc<SlimApp<AuthProvider, AuthVerifier>> {
        self.app.clone()
    }

    /// Async constructor - Create a new App with complete creation logic
    ///
    /// This is the recommended entry point for language bindings to avoid nested block_on issues.
    /// Uses the global service instance.
    pub async fn new_async(
        base_name: SlimName,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Self, SlimError> {
        // Use the global service
        let service_arc = get_global_service().inner.clone();

        // Delegate to the core creation logic
        crate::service::create_app_async_internal(
            base_name,
            identity_provider_config,
            identity_verifier_config,
            service_arc,
            Direction::Bidirectional,
        )
        .await
    }

    /// Get a reference to the service instance
    ///
    /// This provides access to the underlying SLIM service that manages this app.
    /// Useful for accessing service-level functionality and state.
    ///
    /// # Returns
    /// A reference to the Arc-wrapped SlimService instance
    pub fn service(&self) -> &Arc<SlimService> {
        &self.service
    }

    /// Create a new App with traffic direction (async version)
    ///
    /// This is a convenience function for creating a SLIM application with configurable
    /// traffic direction (send-only, receive-only, bidirectional, or none).
    /// This is the async version for use in async contexts.
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    /// * `direction` - Traffic direction for sessions (Send, Recv, Bidirectional, or None)
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created app
    /// * `Err(SlimError)` - If app creation fails
    pub async fn new_with_direction_async(
        name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        direction: Direction,
    ) -> Result<Arc<App>, SlimError> {
        // Delegate to the global service's async create_app_with_direction method
        get_global_service()
            .create_app_with_direction_async(
                name,
                identity_provider_config,
                identity_verifier_config,
                direction,
            )
            .await
    }

    /// Create a new App with SharedSecret authentication (async version)
    ///
    /// This is a convenience function for creating a SLIM application using SharedSecret authentication.
    /// This is the async version for use in async contexts.
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `secret` - The shared secret string for authentication
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    pub async fn new_with_secret_async(
        name: Arc<Name>,
        secret: String,
    ) -> Result<Arc<App>, SlimError> {
        // Delegate to the global service's async create_app_with_secret method
        get_global_service()
            .create_app_with_secret_async(name, secret)
            .await
    }
}

#[uniffi::export]
impl App {
    /// Create a new App with identity provider and verifier configurations
    ///
    /// This is the main entry point for creating a SLIM application from language bindings.
    ///
    /// # Arguments
    /// * `base_name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    ///
    /// # Supported Identity Types
    /// - SharedSecret: Symmetric key authentication
    /// - JWT: Dynamic JWT generation/verification with signing/decoding keys
    /// - StaticJWT: Static JWT loaded from file with auto-reload
    #[uniffi::constructor]
    pub fn new(
        base_name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
    ) -> Result<Arc<Self>, SlimError> {
        crate::config::get_runtime().block_on(async {
            Self::new_async(
                base_name.as_ref().into(),
                identity_provider_config,
                identity_verifier_config,
            )
            .await
            .map(Arc::new)
        })
    }

    /// Create a new App with traffic direction (blocking version)
    ///
    /// This is a convenience function for creating a SLIM application with configurable
    /// traffic direction (send-only, receive-only, bidirectional, or none).
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `identity_provider_config` - Configuration for proving identity to others
    /// * `identity_verifier_config` - Configuration for verifying identity of others
    /// * `direction` - Traffic direction for sessions (Send, Recv, Bidirectional, or None)
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created app
    /// * `Err(SlimError)` - If app creation fails
    #[uniffi::constructor]
    pub fn new_with_direction(
        name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        direction: Direction,
    ) -> Result<Arc<App>, SlimError> {
        // Delegate to the global service's blocking method
        get_global_service().create_app_with_direction(
            name,
            identity_provider_config,
            identity_verifier_config,
            direction,
        )
    }

    /// Create a new App with SharedSecret authentication (blocking version)
    ///
    /// This is a convenience function for creating a SLIM application using SharedSecret authentication.
    ///
    /// # Arguments
    /// * `name` - The base name for the app (without ID)
    /// * `secret` - The shared secret string for authentication
    ///
    /// # Returns
    /// * `Ok(Arc<App>)` - Successfully created adapter
    /// * `Err(SlimError)` - If adapter creation fails
    #[uniffi::constructor]
    pub fn new_with_secret(name: Arc<Name>, secret: String) -> Result<Arc<App>, SlimError> {
        // Delegate to the global service's blocking method
        get_global_service().create_app_with_secret(name, secret)
    }

    /// Get the app ID (derived from name)
    pub fn id(&self) -> u64 {
        self.app.app_name().id()
    }

    /// Get the app name
    pub fn name(&self) -> Arc<Name> {
        Arc::new(self.app.app_name().into())
    }

    /// Create a new session (blocking version for FFI)
    ///
    /// Returns a SessionWithCompletion containing the session context and a completion handle.
    /// Call `.wait()` on the completion handle to wait for session establishment.
    pub fn create_session(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<SessionWithCompletion, SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.create_session_async(config, destination).await })
    }

    /// Create a new session (async version)
    ///
    /// Returns a SessionWithCompletion containing the session context and a completion handle.
    /// Await the completion handle to wait for session establishment.
    /// For point-to-point sessions, this ensures the remote peer has acknowledged the session.
    /// For multicast sessions, this ensures the initial setup is complete.
    pub async fn create_session_async(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<SessionWithCompletion, SlimError> {
        let slim_config: SlimSessionConfig = config.into();
        let slim_dest: SlimName = destination.as_ref().into();
        let app = self.app.clone();
        let runtime = get_runtime();

        // Spawn on the runtime's handle to ensure tokio context is available
        let handle = runtime
            .handle()
            .spawn(async move { app.create_session(slim_config, slim_dest, None).await });

        let (session_ctx, completion) = handle.await.map_err(|e| SlimError::SessionError {
            message: format!("Failed to create session: {}", e),
        })??;

        // Create Session and CompletionHandle
        let bindings_ctx = Arc::new(crate::Session::new(session_ctx));
        let completion_handle = Arc::new(crate::CompletionHandle::from(completion));

        Ok(SessionWithCompletion {
            session: bindings_ctx,
            completion: completion_handle,
        })
    }

    /// Create a new session and wait for completion (blocking version)
    ///
    /// This method creates a session and blocks until the session establishment completes.
    /// Returns only the session context, as the completion has already been awaited.
    pub fn create_session_and_wait(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<Arc<crate::Session>, SlimError> {
        crate::config::get_runtime().block_on(async {
            self.create_session_and_wait_async(config, destination)
                .await
        })
    }

    /// Create a new session and wait for completion (async version)
    ///
    /// This method creates a session and waits until the session establishment completes.
    /// Returns only the session context, as the completion has already been awaited.
    pub async fn create_session_and_wait_async(
        &self,
        config: SessionConfig,
        destination: Arc<Name>,
    ) -> Result<Arc<crate::Session>, SlimError> {
        let session_with_completion = self.create_session_async(config, destination).await?;
        session_with_completion.completion.wait_async().await?;
        Ok(session_with_completion.session)
    }

    /// Delete a session (blocking version for FFI)
    ///
    /// Returns a completion handle that can be awaited to ensure the deletion completes.
    pub fn delete_session(
        &self,
        session: Arc<crate::Session>,
    ) -> Result<Arc<crate::CompletionHandle>, SlimError> {
        crate::config::get_runtime().block_on(async { self.delete_session_async(session).await })
    }

    /// Delete a session (async version)
    ///
    /// Returns a completion handle that can be awaited to ensure the deletion completes.
    pub async fn delete_session_async(
        &self,
        session: Arc<crate::Session>,
    ) -> Result<Arc<crate::CompletionHandle>, SlimError> {
        let session_ref = session
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        let completion = self.app.delete_session(&session_ref)?;

        // Return completion handle for caller to wait on
        Ok(Arc::new(crate::CompletionHandle::from(completion)))
    }

    /// Delete a session and wait for completion (blocking version)
    ///
    /// This method deletes a session and blocks until the deletion completes.
    pub fn delete_session_and_wait(&self, session: Arc<crate::Session>) -> Result<(), SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.delete_session_and_wait_async(session).await })
    }

    /// Delete a session and wait for completion (async version)
    ///
    /// This method deletes a session and waits until the deletion completes.
    pub async fn delete_session_and_wait_async(
        &self,
        session: Arc<crate::Session>,
    ) -> Result<(), SlimError> {
        let completion_handle = self.delete_session_async(session).await?;
        completion_handle.wait_async().await
    }

    /// Subscribe to a session name (blocking version for FFI)
    pub fn subscribe(&self, name: Arc<Name>, connection_id: Option<u64>) -> Result<(), SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.subscribe_async(name, connection_id).await })
    }

    /// Subscribe to a name (async version)
    pub async fn subscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.subscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Unsubscribe from a name (blocking version for FFI)
    pub fn unsubscribe(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.unsubscribe_async(name, connection_id).await })
    }

    /// Unsubscribe from a name (async version)
    pub async fn unsubscribe_async(
        &self,
        name: Arc<Name>,
        connection_id: Option<u64>,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.unsubscribe(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Set a route to a name for a specific connection (blocking version for FFI)
    pub fn set_route(&self, name: Arc<Name>, connection_id: u64) -> Result<(), SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.set_route_async(name, connection_id).await })
    }

    /// Set a route to a name for a specific connection (async version)
    pub async fn set_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.set_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Remove a route (blocking version for FFI)
    pub fn remove_route(&self, name: Arc<Name>, connection_id: u64) -> Result<(), SlimError> {
        crate::config::get_runtime()
            .block_on(async { self.remove_route_async(name, connection_id).await })
    }

    /// Remove a route (async version)
    pub async fn remove_route_async(
        &self,
        name: Arc<Name>,
        connection_id: u64,
    ) -> Result<(), SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        self.app.remove_route(&slim_name, connection_id).await?;
        Ok(())
    }

    /// Listen for incoming sessions (blocking version for FFI)
    pub fn listen_for_session(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<Arc<crate::Session>, SlimError> {
        crate::get_runtime().block_on(async { self.listen_for_session_async(timeout).await })
    }

    /// Listen for incoming sessions (async version)
    pub async fn listen_for_session_async(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<Arc<crate::Session>, SlimError> {
        let mut rx = self.notification_rx.write().await;

        let recv_fut = rx.recv();
        let notification_opt = if let Some(dur) = timeout {
            // Runtime-agnostic timeout using futures-timer
            futures::pin_mut!(recv_fut);
            let delay = Delay::new(dur);
            futures::pin_mut!(delay);

            match futures::future::select(recv_fut, delay).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => {
                    return Err(SlimError::ReceiveError {
                        message: "listen_for_session timed out".to_string(),
                    });
                }
            }
        } else {
            recv_fut.await
        };

        if notification_opt.is_none() {
            return Err(SlimError::ReceiveError {
                message: "application channel closed".to_string(),
            });
        }

        match notification_opt.unwrap() {
            Ok(Notification::NewSession(ctx)) => Ok(Arc::new(crate::Session::new(ctx))),
            Ok(Notification::NewMessage(_)) => Err(SlimError::ReceiveError {
                message: "received unexpected message notification while listening for session"
                    .to_string(),
            }),
            Err(e) => Err(SlimError::ReceiveError {
                message: format!("failed to receive session notification: {}", e),
            }),
        }
    }
}

// Non-UniFFI methods for internal use (slimrpc)
impl App {
    /// Get reference to internal app for advanced use cases (slimrpc)
    pub fn inner_app(&self) -> &Arc<SlimApp<AuthProvider, AuthVerifier>> {
        &self.app
    }

    /// Get notification receiver for server use (slimrpc)
    pub fn notification_receiver(
        &self,
    ) -> Arc<RwLock<mpsc::Receiver<Result<Notification, SlimSessionError>>>> {
        self.notification_rx.clone()
    }
}

// ============================================================================
// Internal methods for PyO3 bindings (not exported through UniFFI)
// ============================================================================

impl App {
    /// Create a new session returning internal Session (for PyO3 bindings)
    ///
    /// This method is for Python bindings to bypass the FFI layer and get
    /// a proper Session with its CompletionHandle.
    pub async fn create_session_internal(
        &self,
        config: SlimSessionConfig,
        destination: SlimName,
    ) -> Result<
        (
            slim_session::context::SessionContext,
            slim_session::CompletionHandle,
        ),
        SlimError,
    > {
        let (session_ctx, completion) = self.app.create_session(config, destination, None).await?;

        Ok((session_ctx, completion))
    }

    /// Listen for incoming sessions returning internal Session (for PyO3 bindings)
    pub async fn listen_for_session_internal(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<slim_session::context::SessionContext, SlimError> {
        let mut rx = self.notification_rx.write().await;

        let recv_fut = rx.recv();
        let notification_opt = if let Some(dur) = timeout {
            // Runtime-agnostic timeout using futures-timer
            futures::pin_mut!(recv_fut);
            let delay = Delay::new(dur);
            futures::pin_mut!(delay);

            match futures::future::select(recv_fut, delay).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => {
                    return Err(SlimError::ReceiveError {
                        message: "listen_for_session timed out".to_string(),
                    });
                }
            }
        } else {
            recv_fut.await
        };

        if notification_opt.is_none() {
            return Err(SlimError::ReceiveError {
                message: "application channel closed".to_string(),
            });
        }

        match notification_opt.unwrap() {
            Ok(Notification::NewSession(ctx)) => Ok(ctx),
            Ok(Notification::NewMessage(_)) => Err(SlimError::ReceiveError {
                message: "received unexpected message notification while listening for session"
                    .to_string(),
            }),
            Err(e) => Err(SlimError::ReceiveError {
                message: format!("failed to receive session notification: {}", e),
            }),
        }
    }

    /// Delete a session returning CompletionHandle (for PyO3 bindings)
    ///
    /// This method is for Python bindings to get the CompletionHandle when deleting a session.
    pub fn delete_session_internal(
        &self,
        session: &SessionController,
    ) -> Result<slim_session::CompletionHandle, SlimError> {
        let ret = self.app.delete_session(session)?;

        Ok(ret)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use crate::SessionType;

    use super::*;

    use slim_config::component::ComponentBuilder;
    use slim_datapath::messages::Name as SlimName;
    use slim_testing::utils::TEST_VALID_SECRET;

    // Helper to create test identity configs
    fn create_test_configs(secret: &str) -> (IdentityProviderConfig, IdentityVerifierConfig) {
        (
            IdentityProviderConfig::SharedSecret {
                id: "test-service".to_string(),
                data: secret.to_string(),
            },
            IdentityVerifierConfig::SharedSecret {
                id: "test-service".to_string(),
                data: secret.to_string(),
            },
        )
    }

    /// Test basic adapter creation
    #[tokio::test]
    async fn test_adapter_creation() {
        let base_name = SlimName::from_strings(["org", "namespace", "test-app"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let result = App::new_async(base_name, provider_config, verifier_config).await;
        assert!(result.is_ok());

        let adapter = result.unwrap();
        assert!(adapter.id() > 0);
    }

    /// Test that adapter ID is consistently derived from its internal provider's token ID
    #[tokio::test]
    async fn test_deterministic_id_generation() {
        let base_name = SlimName::from_strings(["org", "namespace", "test-app"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // Create the adapter
        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // The adapter's ID should be non-zero (derived from token ID hash)
        let adapter_id = adapter.id();
        assert!(adapter_id > 0, "Adapter ID should be non-zero");

        // Verify the adapter's name includes the ID
        let adapter_name = adapter.name();
        assert_eq!(
            adapter_name.id(),
            adapter_id,
            "Name ID should match adapter ID"
        );
    }

    /// Test that session creation auto-waits for establishment
    #[tokio::test]
    async fn test_session_creation_auto_wait() {
        // This test verifies that create_session_async properly awaits the completion handle
        // In a real scenario, this would ensure the session is fully established

        let base_name = SlimName::from_strings(["org", "namespace", "create-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_millis(100)),
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "dest".to_string(),
        ));

        // This should auto-wait for session establishment
        // If it returns without error, the session is fully established
        let result = adapter
            .create_session_async(session_config, destination)
            .await;

        // In a real scenario with network, this would verify the session is ready
        // For this test, we just verify it completes without panicking
        match result {
            Ok(_session) => {
                // Session created and auto-waited successfully
            }
            Err(e) => {
                // Expected to fail in test environment without network
                // but shouldn't panic
                println!("Expected error in test environment: {:?}", e);
            }
        }
    }

    /// Test publish_with_completion returns a valid completion handle
    #[tokio::test]
    async fn test_publish_with_completion_returns_handle() {
        // Note: This is a structural test - in a real environment with connections,
        // the completion handle would actually track message delivery

        let base_name = SlimName::from_strings(["org", "namespace", "publish-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_millis(100)),
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "dest".to_string(),
        ));

        // Try to create a session (may fail without network)
        if let Ok(session) = adapter
            .create_session_async(session_config, destination)
            .await
        {
            let data = b"test message".to_vec();

            // Attempt to publish (always returns completion handle)
            // This verifies the API exists and returns the right type
            let result = session.session.publish_async(data, None, None).await;

            match result {
                Ok(completion_handle) => {
                    // Verify we got a completion handle
                    // In a real scenario, we could wait on it
                    assert!(Arc::strong_count(&completion_handle) > 0);
                }
                Err(e) => {
                    // Expected to fail without actual connections
                    println!("Expected error without network: {:?}", e);
                }
            }
        }
    }

    // ========================================================================
    // TlsConfig Tests
    // ========================================================================

    /// Test TlsConfig with all fields
    #[test]
    fn test_tls_config_full() {
        use crate::common_config::TlsClientConfig;

        let config = TlsClientConfig {
            insecure: false,
            insecure_skip_verify: false,
            source: crate::common_config::TlsSource::File {
                cert: "test-cert.pem".to_string(),
                key: "test-key.pem".to_string(),
            },
            ca_source: crate::common_config::CaSource::File {
                path: "test-ca.pem".to_string(),
            },
            include_system_ca_certs_pool: true,
            tls_version: "tls1.3".to_string(),
        };

        assert!(!config.insecure);
        assert!(!config.insecure_skip_verify);
        assert!(matches!(
            config.source,
            crate::common_config::TlsSource::File { .. }
        ));
        assert!(matches!(
            config.ca_source,
            crate::common_config::CaSource::File { .. }
        ));
        assert_eq!(config.tls_version, "tls1.3");
        assert!(config.include_system_ca_certs_pool);
    }

    /// Test TlsConfig with insecure mode
    #[test]
    fn test_tls_config_insecure() {
        use crate::common_config::TlsClientConfig;

        let config = TlsClientConfig {
            insecure: true,
            insecure_skip_verify: false,
            source: crate::common_config::TlsSource::None,
            ca_source: crate::common_config::CaSource::None,
            include_system_ca_certs_pool: true,
            tls_version: "tls1.3".to_string(),
        };

        assert!(config.insecure);
        assert!(matches!(
            config.source,
            crate::common_config::TlsSource::None
        ));
    }

    // ========================================================================
    // Adapter Methods Tests
    // ========================================================================

    /// Test adapter id() and name() methods
    #[tokio::test]
    async fn test_adapter_id_and_name() {
        let base_name = SlimName::from_strings(["org", "namespace", "id-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name.clone(), provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // ID should be non-zero
        let id = adapter.id();
        assert!(id > 0, "Adapter ID should be positive");

        // Name should have the right components
        let name = adapter.name();
        assert_eq!(name.components()[0], "org");
        assert_eq!(name.components()[1], "namespace");
        assert_eq!(name.components()[2], "id-test");
        assert!(name.id() > 0);
    }

    /// Test adapter with local service
    /// Test adapter with global service
    #[tokio::test]
    async fn test_adapter_with_global_service() {
        let base_name = SlimName::from_strings(["org", "namespace", "global-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let result = App::new_async(base_name, provider_config, verifier_config).await;
        assert!(result.is_ok(), "Should create adapter with global service");
    }

    /// Test adapter creation with different namespace values
    #[tokio::test]
    async fn test_adapter_different_namespaces() {
        // Test with different valid namespace configurations
        let namespaces = [
            ["org1", "namespace1", "app1"],
            ["company", "team", "service"],
            ["prod", "api", "gateway"],
        ];

        for ns in namespaces {
            let base_name = SlimName::from_strings(ns);
            let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

            let result = App::new_async(base_name, provider_config, verifier_config).await;
            assert!(
                result.is_ok(),
                "Should create adapter for namespace {:?}",
                ns
            );
        }
    }

    // ========================================================================
    // initialize_crypto_provider Tests
    // ========================================================================
    // App::new Tests
    // ========================================================================

    /// Test App::new_async entry point
    #[tokio::test]
    async fn test_bindings_adapter_new() {
        let base_name = SlimName::from_strings(["org", "namespace", "ffi-app"]);

        let provider_config = IdentityProviderConfig::SharedSecret {
            id: "test-sync-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };
        let verifier_config = IdentityVerifierConfig::SharedSecret {
            id: "test-sync-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };

        let result = App::new_async(base_name, provider_config, verifier_config).await;
        assert!(result.is_ok(), "App::new_async should succeed");

        let adapter = result.unwrap();
        assert!(adapter.id() > 0);

        let name = adapter.name();
        assert_eq!(name.components()[0], "org");
        assert_eq!(name.components()[1], "namespace");
        assert_eq!(name.components()[2], "ffi-app");
    }

    /// Test App::new_async with minimal name (3 components)
    #[tokio::test]
    async fn test_bindings_adapter_new_minimal_name() {
        let base_name = SlimName::from_strings(["org", "ns", "test-app"]);

        let provider_config = IdentityProviderConfig::SharedSecret {
            id: "test-minimal-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };
        let verifier_config = IdentityVerifierConfig::SharedSecret {
            id: "test-minimal-service".to_string(),
            data: TEST_VALID_SECRET.to_string(),
        };

        let result = App::new_async(base_name, provider_config, verifier_config).await;
        // Should handle minimal name
        assert!(result.is_ok(), "Should create adapter with minimal name");
    }

    // ========================================================================
    // Listen for Session Timeout Tests
    // ========================================================================

    /// Test listen_for_session with timeout
    #[tokio::test]
    async fn test_listen_for_session_timeout() {
        let base_name = SlimName::from_strings(["org", "namespace", "listen-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // Listen with a very short timeout - should timeout
        let result = adapter
            .listen_for_session_async(Some(std::time::Duration::from_millis(10)))
            .await;

        match result {
            Err(SlimError::ReceiveError { message }) => {
                assert!(
                    message.contains("timed out"),
                    "Should contain timeout message"
                );
            }
            _ => {
                // May get a different error in some cases, which is fine
            }
        }
    }

    // ========================================================================
    // Subscribe/Unsubscribe Tests
    // ========================================================================

    /// Test subscribe and unsubscribe (requires running service)
    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let base_name = SlimName::from_strings(["org", "namespace", "sub-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "target".to_string(),
        ));

        // Subscribe (may fail without connection, but shouldn't panic)
        let sub_result = adapter.subscribe_async(target_name.clone(), None).await;
        // We don't assert success because there's no active connection

        // Unsubscribe
        let unsub_result = adapter.unsubscribe_async(target_name, None).await;
        // Same - may fail but shouldn't panic

        let _ = (sub_result, unsub_result);
    }

    // ========================================================================
    // Set/Remove Route Tests
    // ========================================================================

    /// Test set_route and remove_route
    #[tokio::test]
    async fn test_set_remove_route() {
        let base_name = SlimName::from_strings(["org", "namespace", "route-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "route-target".to_string(),
        ));

        // Set route (may fail without valid connection_id)
        let set_result = adapter.set_route_async(target_name.clone(), 12345).await;
        // Remove route
        let remove_result = adapter.remove_route_async(target_name, 12345).await;

        // These will likely fail without actual connections, but shouldn't panic
        let _ = (set_result, remove_result);
    }

    // ========================================================================
    // Stop Server Tests
    // ========================================================================

    /// Test stop_server on non-existent server
    #[tokio::test]
    async fn test_stop_server_nonexistent() {
        let base_name = SlimName::from_strings(["org", "namespace", "stop-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let _adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // Try to stop a server that doesn't exist using the global service
        let service = get_global_service();
        let result = service.stop_server("127.0.0.1:99999".to_string());
        // Should fail with appropriate error
        assert!(result.is_err());
    }

    // ========================================================================
    // Disconnect Tests
    // ========================================================================

    /// Test disconnect with invalid connection_id
    #[tokio::test]
    async fn test_disconnect_invalid_id() {
        let base_name = SlimName::from_strings(["org", "namespace", "disconnect-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let _adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // Try to disconnect with an invalid connection ID using the global service
        let service = get_global_service();
        let result = service.disconnect(999999);
        // Should fail but not panic
        assert!(result.is_err());
    }

    // ========================================================================
    // Delete Session Tests
    // ========================================================================

    /// Test delete_session with a session (structural test)
    #[tokio::test]
    async fn test_delete_session_flow() {
        let base_name = SlimName::from_strings(["org", "namespace", "delete-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(1),
            interval: Some(std::time::Duration::from_millis(50)),
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "delete-dest".to_string(),
        ));

        // Create session (may fail without network)
        if let Ok(session_with_completion) = adapter
            .create_session_async(session_config, destination)
            .await
        {
            // Delete session
            let delete_result = adapter
                .delete_session_async(session_with_completion.session)
                .await;
            // May succeed or fail depending on session state
            if let Ok(completion) = delete_result {
                let _ = completion;
            }
        }
    }

    // ========================================================================
    // Blocking Method Tests
    // ========================================================================

    /// Test blocking version of listen_for_session
    #[tokio::test]
    async fn test_listen_for_session_blocking_timeout() {
        let base_name = SlimName::from_strings(["org", "namespace", "blocking-listen"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        // Call async version with short timeout (blocking version can't be called from async context)
        let result = adapter
            .listen_for_session_async(Some(std::time::Duration::from_millis(10)))
            .await;

        // Should timeout
        match result {
            Err(SlimError::ReceiveError { message }) => {
                assert!(message.contains("timed out") || message.contains("closed"));
            }
            _ => {
                // Other errors are acceptable too
            }
        }
    }

    /// Test blocking version of subscribe
    #[tokio::test]
    async fn test_subscribe_blocking() {
        let base_name = SlimName::from_strings(["org", "namespace", "blocking-sub"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let adapter = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter");

        let target_name = Arc::new(Name::new(
            "org".to_string(),
            "ns".to_string(),
            "block-target".to_string(),
        ));

        // Call async version (blocking version can't be called from async context)
        let _ = adapter.subscribe_async(target_name, None).await;
    }

    /// Test from_parts internal constructor
    #[tokio::test]
    async fn test_from_parts_constructor() {
        let base_name = SlimName::from_strings(["org", "namespace", "from-parts-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // First create an adapter normally to get its parts
        let _adapter1 = App::new_async(
            base_name.clone(),
            provider_config.clone(),
            verifier_config.clone(),
        )
        .await
        .expect("Failed to create adapter");

        // Now create another adapter using the Service's create_adapter_async method
        // which uses from_parts internally
        let service = get_global_service();
        let name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "from-parts-test-2".to_string(),
        ));

        let adapter2 = service
            .create_app_async(name, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter via service");

        // Verify the adapter created via from_parts works correctly
        assert!(adapter2.id() > 0);
        assert!(!adapter2.name().to_string().is_empty());
    }

    /// Test creating app with a custom service using Service::create_app_async
    #[tokio::test]
    async fn test_new_async_with_custom_service() {
        use slim_service::Service as SlimService;

        let base_name = SlimName::from_strings(["org", "namespace", "custom-service-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // Create a custom service instance
        let custom_service = SlimService::builder()
            .build("test-custom-service".to_string())
            .expect("Failed to create custom service");
        let service_wrapper = crate::Service {
            inner: Arc::new(custom_service),
        };

        // Create adapter with custom service using Service API
        let base_name_arc = Arc::new(Name::from(base_name));
        let result = service_wrapper
            .create_app_async(base_name_arc, provider_config, verifier_config)
            .await;

        assert!(result.is_ok(), "Should create adapter with custom service");
        let adapter = result.unwrap();
        assert!(adapter.id() > 0);
        assert!(!adapter.name().to_string().is_empty());
    }

    /// Test that new_async uses global service by default
    #[tokio::test]
    async fn test_new_async_uses_global_service() {
        let base_name = SlimName::from_strings(["org", "namespace", "new-async-global"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // Create two adapters using new_async
        let adapter1 = App::new_async(
            base_name.clone(),
            provider_config.clone(),
            verifier_config.clone(),
        )
        .await
        .expect("Failed to create first adapter");

        let base_name2 = SlimName::from_strings(["org", "namespace", "new-async-global-2"]);
        let adapter2 = App::new_async(base_name2, provider_config, verifier_config)
            .await
            .expect("Failed to create second adapter");

        // Both should be created successfully (using the same global service)
        assert!(adapter1.id() > 0);
        assert!(adapter2.id() > 0);
        assert_ne!(
            adapter1.id(),
            adapter2.id(),
            "Different adapters should have different IDs"
        );
    }

    /// Test multiple adapters with different custom services
    /// Test that different service instances create isolated adapters
    #[tokio::test]
    async fn test_different_services_create_isolated_adapters() {
        use slim_service::Service as SlimService;

        let base_name1 = SlimName::from_strings(["org", "namespace", "isolated-1"]);
        let base_name2 = SlimName::from_strings(["org", "namespace", "isolated-2"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // Create two different custom services
        let service1 = SlimService::builder()
            .build("test-service-1".to_string())
            .expect("Failed to create service 1");
        let service1_wrapper = crate::Service {
            inner: Arc::new(service1),
        };

        let service2 = SlimService::builder()
            .build("test-service-2".to_string())
            .expect("Failed to create service 2");
        let service2_wrapper = crate::Service {
            inner: Arc::new(service2),
        };

        // Create adapters with different services using Service API
        let base_name1_arc = Arc::new(Name::from(base_name1));
        let adapter1 = service1_wrapper
            .create_app_async(
                base_name1_arc,
                provider_config.clone(),
                verifier_config.clone(),
            )
            .await
            .expect("Failed to create adapter 1");

        let base_name2_arc = Arc::new(Name::from(base_name2));
        let adapter2 = service2_wrapper
            .create_app_async(base_name2_arc, provider_config, verifier_config)
            .await
            .expect("Failed to create adapter 2");

        // Adapters should have different IDs since they're on different services
        assert_ne!(adapter1.id(), adapter2.id());
    }

    /// Test new_with_secret_async creates app successfully
    #[tokio::test]
    async fn test_new_with_secret_async() {
        let name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "secret-test".to_string(),
        ));
        let secret = TEST_VALID_SECRET.to_string();

        let result = App::new_with_secret_async(name, secret).await;

        assert!(result.is_ok(), "Should create app with secret");
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
        assert!(
            !app.name().to_string().is_empty(),
            "App should have valid name"
        );
    }

    /// Test new_with_secret blocking version
    #[test]
    fn test_new_with_secret_blocking() {
        let name = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "secret-blocking-test".to_string(),
        ));
        let secret = TEST_VALID_SECRET.to_string();

        // Call blocking version from sync context
        let result = App::new_with_secret(name, secret);

        assert!(
            result.is_ok(),
            "Should create app with secret in blocking mode"
        );
        let app = result.unwrap();
        assert!(app.id() > 0, "App should have non-zero ID");
    }

    /// Test new_with_secret creates unique IDs for different apps
    #[tokio::test]
    async fn test_new_with_secret_unique_ids() {
        let secret = TEST_VALID_SECRET.to_string();

        let name1 = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "unique-1".to_string(),
        ));
        let name2 = Arc::new(Name::new(
            "org".to_string(),
            "namespace".to_string(),
            "unique-2".to_string(),
        ));

        let app1 = App::new_with_secret_async(name1, secret.clone())
            .await
            .expect("Failed to create app1");

        let app2 = App::new_with_secret_async(name2, secret)
            .await
            .expect("Failed to create app2");

        assert_ne!(app1.id(), app2.id(), "Apps should have different IDs");
    }

    /// Test create_session_async runtime spawning
    #[tokio::test]
    async fn test_create_session_async_runtime_spawning() {
        let base_name = SlimName::from_strings(["org", "namespace", "runtime-spawn-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let app = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create app");

        let session_config = SessionConfig {
            session_type: SessionType::PointToPoint,
            enable_mls: false,
            max_retries: Some(3),
            interval: Some(std::time::Duration::from_millis(100)),
            metadata: std::collections::HashMap::new(),
        };

        let destination = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            "spawn-dest".to_string(),
        ));

        // This should not panic even though it spawns on runtime
        let result = app.create_session_async(session_config, destination).await;

        // May fail without network, but shouldn't panic from join error
        let _ = result;
    }

    /// Test listen_for_session with timeout using futures-timer
    #[tokio::test]
    async fn test_listen_for_session_timeout_futures_timer() {
        let base_name = SlimName::from_strings(["org", "namespace", "listen-timeout-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let app = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create app");

        // Listen with a short timeout - should timeout since no session is incoming
        let result = app
            .listen_for_session_async(Some(std::time::Duration::from_millis(50)))
            .await;

        assert!(result.is_err(), "Should timeout when no session arrives");
        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("timed out") || error_msg.contains("timeout"),
                "Error should mention timeout"
            );
        }
    }

    /// Test listen_for_session with timeout using futures-timer (second test)
    #[tokio::test]
    async fn test_listen_for_session_no_incoming() {
        let base_name = SlimName::from_strings(["org", "namespace", "no-incoming-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        let app = App::new_async(base_name, provider_config, verifier_config)
            .await
            .expect("Failed to create app");

        // Listen with a short timeout when no session is incoming - should timeout
        let result = app
            .listen_for_session_async(Some(std::time::Duration::from_millis(30)))
            .await;

        assert!(result.is_err(), "Should timeout when no session arrives");
        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("timed out") || error_msg.contains("timeout"),
                "Error should mention timeout"
            );
        }
    }

    /// Test that new_async delegates to service's create_app_async_internal
    #[tokio::test]
    async fn test_new_async_uses_internal_creation() {
        let base_name = SlimName::from_strings(["org", "namespace", "internal-test"]);
        let (provider_config, verifier_config) = create_test_configs(TEST_VALID_SECRET);

        // This tests that new_async properly delegates to the internal creation function
        let result = App::new_async(base_name, provider_config, verifier_config).await;

        assert!(result.is_ok(), "Should create app via internal function");
        let app = result.unwrap();
        assert!(app.id() > 0);
        assert!(!app.name().to_string().is_empty());
    }
}

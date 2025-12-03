// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_session::CompletionHandle;
use slim_session::session_controller::SessionController;
use std::collections::HashMap;
use tokio::sync::RwLock;

use slim_datapath::messages::Name;
use slim_datapath::messages::utils::{PUBLISH_TO, SlimHeaderFlags, TRUE_VAL};
use slim_service::errors::ServiceError;
use slim_session::SessionError;
use slim_session::context::SessionContext;

use crate::message_context::MessageContext;

/// Generic session context wrapper for language bindings
///
/// Wraps the session context with proper async access patterns for message reception.
#[derive(Debug)]
pub struct BindingsSessionContext {
    /// Weak reference to the underlying session
    pub session: std::sync::Weak<SessionController>,
    /// Message receiver wrapped in RwLock for concurrent access
    pub rx: RwLock<slim_session::AppChannelReceiver>,
}

impl From<SessionContext> for BindingsSessionContext {
    /// Create a new BindingsSessionContext from a SessionContext
    fn from(ctx: SessionContext) -> Self {
        let (session, rx) = ctx.into_parts();
        Self {
            session,
            rx: RwLock::new(rx),
        }
    }
}

impl BindingsSessionContext {
    /// Publish a message through this session
    pub async fn publish(
        &self,
        name: &Name,
        fanout: u32,
        blob: Vec<u8>,
        conn_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, ServiceError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| ServiceError::SessionError("Session has been dropped".to_string()))?;

        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        session
            .publish_with_flags(name, flags, blob, payload_type, metadata)
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    /// Publish a message as a reply to a received message (reply semantics)
    ///
    /// This method publishes a message back to the source of a previously received
    /// message, using the routing information from the original message context.
    ///
    /// # Arguments
    /// * `message_ctx` - Context from the original received message (provides routing info)
    /// * `blob` - The message payload bytes
    /// * `payload_type` - Optional content type for the payload
    /// * `metadata` - Optional key-value metadata pairs
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(ServiceError)` if publishing fails
    pub async fn publish_to(
        &self,
        message_ctx: &MessageContext,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, ServiceError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| ServiceError::SessionError("Session has been dropped".to_string()))?;

        let flags = SlimHeaderFlags::new(
            1, // fanout = 1 for reply semantics
            None,
            Some(message_ctx.input_connection), // reply to the same connection
            None,
            None,
        );

        let mut final_metadata = metadata.unwrap_or_default();
        final_metadata.insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        session
            .publish_with_flags(
                &message_ctx.source_name, // reply to the original source
                flags,
                blob,
                payload_type,
                Some(final_metadata),
            )
            .await
            .map_err(|e| ServiceError::SessionError(e.to_string()))
    }

    /// Invite a peer to join this session
    pub async fn invite(&self, destination: &Name) -> Result<CompletionHandle, SessionError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SessionError::Processing("Session has been dropped".to_string()))?;

        session.invite_participant(destination).await
    }

    /// Remove a peer from this session
    pub async fn remove(&self, destination: &Name) -> Result<CompletionHandle, SessionError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SessionError::Processing("Session has been dropped".to_string()))?;

        session.remove_participant(destination).await
    }

    /// Receive a message from this session with optional timeout
    ///
    /// This method blocks until a message is available on this session's channel
    /// or the timeout expires. All message reception in SLIM is session-specific.
    ///
    /// # Arguments
    /// * `timeout` - Optional timeout for the operation
    ///
    /// # Returns
    /// * `Ok((MessageContext, Vec<u8>))` - Message context and raw payload bytes
    /// * `Err(ServiceError)` - If the session channel is closed or timeout expires
    pub async fn get_session_message(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<(MessageContext, Vec<u8>), ServiceError> {
        let mut rx = self.rx.write().await;

        let recv_future = async {
            let msg = rx
                .recv()
                .await
                .ok_or_else(|| ServiceError::ReceiveError("session channel closed".to_string()))?;

            let msg = msg.map_err(|e| {
                ServiceError::ReceiveError(format!("failed to decode message: {}", e))
            })?;
            MessageContext::from_proto_message(msg)
        };

        if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, recv_future)
                .await
                .map_err(|_| {
                    ServiceError::ReceiveError("timeout waiting for message".to_string())
                })?
        } else {
            recv_future.await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    use slim_auth::shared_secret::SharedSecret;
    use slim_config::component::ComponentBuilder;
    use slim_datapath::messages::Name;
    use slim_testing::utils::TEST_VALID_SECRET;

    use crate::adapter::BindingsAdapter;
    use slim_service::Service;

    /// Create a mock service for testing
    async fn create_test_service() -> Service {
        Service::builder()
            .build("test-service".to_string())
            .expect("Failed to create test service")
    }

    /// Create test authentication components
    fn create_test_auth() -> (AuthProvider, AuthVerifier) {
        let shared_secret = SharedSecret::new("test-app", TEST_VALID_SECRET);
        let provider = AuthProvider::SharedSecret(shared_secret.clone());
        let verifier = AuthVerifier::SharedSecret(shared_secret);
        (provider, verifier)
    }

    /// Create test app name
    fn create_test_name() -> Name {
        Name::from_strings(["org", "namespace", "test-app"])
    }

    #[tokio::test]
    async fn test_bindings_session_context_creation() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        // Verify session reference is valid through the FFI wrapper
        assert!(session_ffi.inner.session.upgrade().is_some());
    }

    #[tokio::test]
    async fn test_get_session_message_timeout() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        // Create a session using FFI types
        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        // Test that get_session_message times out when no messages are sent
        // Use the async version since we're in an async test
        let result = session_ffi
            .get_message_async(Some(50)) // 50ms timeout
            .await;
        assert!(result.is_err()); // Should timeout
        if let Err(e) = result {
            assert!(e.to_string().contains("timeout"));
        }
    }

    #[tokio::test]
    async fn test_get_session_message_no_timeout() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        // Create a session using FFI types
        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        // Test with None timeout - should wait indefinitely until channel is closed
        // Use a timeout wrapper to prevent the test from hanging indefinitely
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            session_ffi.get_message_async(None),
        )
        .await;

        // The operation should timeout since no message is being sent and we're not providing a timeout
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_session_message_various_timeouts() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        // Create a session using FFI types
        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        // Test very short timeout (1 nanosecond = 0 milliseconds, rounds down)
        let result = session_ffi.get_message_async(Some(0)).await;
        assert!(result.is_err());

        // Test zero timeout
        let result = session_ffi.get_message_async(Some(0)).await;
        assert!(result.is_err());

        // Test reasonable timeout with timing verification
        let start = std::time::Instant::now();
        let result = session_ffi.get_message_async(Some(100)).await;
        let elapsed = start.elapsed();

        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(90)); // Allow some variance
        assert!(elapsed <= Duration::from_millis(200)); // But not too much
    }

    #[tokio::test]
    async fn test_publish_method() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        // Create a session using FFI types
        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        let message = b"test payload".to_vec();
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        // Test the simplified publish method - this should work without errors
        let result = session_ffi
            .publish_async(message, Some("text/plain".to_string()), Some(metadata))
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_publish_with_params_method() {
        let service = create_test_service().await;
        let app_name = create_test_name();
        let (provider, verifier) = create_test_auth();

        let adapter = BindingsAdapter::new_with_service(&service, app_name, provider, verifier)
            .expect("Failed to create adapter");

        // Create a session using FFI types
        let config = crate::adapter::SessionConfig {
            session_type: crate::adapter::SessionType::PointToPoint,
            enable_mls: false,
            max_retries: None,
            interval_ms: None,
            initiator: true,
            metadata: std::collections::HashMap::new(),
        };
        let dst = crate::adapter::Name {
            components: vec!["org".to_string(), "ns".to_string(), "dst".to_string()],
            id: None,
        };
        let session_ffi = adapter
            .create_session_async(config, dst)
            .await
            .expect("Failed to create session");

        // Create a custom destination name for publishing
        let destination = crate::adapter::Name {
            components: vec![
                "sender".to_string(),
                "org".to_string(),
                "service".to_string(),
            ],
            id: None,
        };

        let message = b"advanced payload".to_vec();
        let mut metadata = HashMap::new();
        metadata.insert("custom_header".to_string(), "custom_value".to_string());

        // Test the advanced publish_with_params method with full control
        let result = session_ffi
            .publish_with_params_async(
                destination,
                1,
                message,
                None,
                Some("application/json".to_string()),
                Some(metadata),
            )
            .await;

        assert!(result.is_ok());
    }
}


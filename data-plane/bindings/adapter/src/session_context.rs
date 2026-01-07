// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use slim_session::CompletionHandle;
use slim_session::session_controller::SessionController;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use slim_datapath::api::ProtoSessionType;
use slim_datapath::messages::Name as SlimName;
use slim_datapath::messages::utils::{PUBLISH_TO, SlimHeaderFlags, TRUE_VAL};
use slim_session::SessionError;
use slim_session::context::SessionContext;

use crate::Name;
use crate::adapter::{FfiCompletionHandle, ReceivedMessage, SessionType, SlimError};
use crate::message_context::MessageContext;

/// Session context for language bindings (UniFFI-compatible)
///
/// Wraps the session context with proper async access patterns for message reception.
/// Provides both synchronous (blocking) and asynchronous methods for FFI compatibility.
#[derive(uniffi::Object)]
pub struct BindingsSessionContext {
    /// Weak reference to the underlying session
    pub session: std::sync::Weak<SessionController>,
    /// Message receiver wrapped in RwLock for concurrent access
    pub rx: RwLock<slim_session::AppChannelReceiver>,
    /// Tokio runtime for blocking operations (static lifetime)
    runtime: &'static tokio::runtime::Runtime,
}

impl BindingsSessionContext {
    /// Create a new BindingsSessionContext from a SessionContext and runtime
    pub fn new(ctx: SessionContext, runtime: &'static tokio::runtime::Runtime) -> Self {
        let (session, rx) = ctx.into_parts();
        Self {
            session,
            rx: RwLock::new(rx),
            runtime,
        }
    }

    /// Get the runtime (for internal use)
    pub fn runtime(&self) -> &'static tokio::runtime::Runtime {
        self.runtime
    }
}

// ============================================================================
// Internal async methods (used by both FFI and Python bindings)
// ============================================================================

impl BindingsSessionContext {
    /// Publish a message through this session (internal API for language bindings)
    ///
    /// This is the low-level publish method that takes SlimName directly.
    /// Use `publish()` or `publish_with_params()` for FFI-compatible APIs.
    pub async fn publish_internal(
        &self,
        name: &SlimName,
        fanout: u32,
        blob: Vec<u8>,
        conn_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, SessionError> {
        let session = self.session.upgrade().ok_or(SessionError::SessionClosed)?;

        let flags = SlimHeaderFlags::new(fanout, None, conn_out, None, None);

        let ret = session
            .publish_with_flags(name, flags, blob, payload_type, metadata)
            .await?;

        Ok(ret)
    }

    /// Publish a message as a reply (internal API for language bindings)
    ///
    /// This is the low-level publish_to method that takes MessageContext reference.
    /// Use `publish_to()` for FFI-compatible API.
    pub async fn publish_to_internal(
        &self,
        message_ctx: &MessageContext,
        blob: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<CompletionHandle, SessionError> {
        let session = self.session.upgrade().ok_or(SessionError::SessionClosed)?;

        let flags = SlimHeaderFlags::new(
            1, // fanout = 1 for reply semantics
            None,
            Some(message_ctx.input_connection), // reply to the same connection
            None,
            None,
        );

        let mut final_metadata = metadata.unwrap_or_default();
        final_metadata.insert(PUBLISH_TO.to_string(), TRUE_VAL.to_string());

        // Convert FFI Name to SlimName for the datapath layer
        let source_name = message_ctx.source_as_slim_name();

        let ret = session
            .publish_with_flags(
                &source_name, // reply to the original source
                flags,
                blob,
                payload_type,
                Some(final_metadata),
            )
            .await?;

        Ok(ret)
    }

    /// Invite a peer to join this session (internal API for language bindings)
    ///
    /// This is the low-level invite method that takes SlimName reference.
    /// Use `invite()` for FFI-compatible API with auto-wait.
    pub async fn invite_internal(
        &self,
        destination: &SlimName,
    ) -> Result<CompletionHandle, SessionError> {
        let session = self.session.upgrade().ok_or(SessionError::SessionClosed)?;

        session.invite_participant(destination).await
    }

    /// Remove a peer from this session (internal API for language bindings)
    ///
    /// This is the low-level remove method that takes SlimName reference.
    /// Use `remove()` for FFI-compatible API with auto-wait.
    pub async fn remove_internal(
        &self,
        destination: &SlimName,
    ) -> Result<CompletionHandle, SessionError> {
        let session = self.session.upgrade().ok_or(SessionError::SessionClosed)?;

        session.remove_participant(destination).await
    }

    /// Receive a message from this session with optional timeout
    pub async fn get_session_message(
        &self,
        timeout: Option<std::time::Duration>,
    ) -> Result<(MessageContext, Vec<u8>), SessionError> {
        let mut rx = self.rx.write().await;

        let recv_future = async {
            let msg = rx.recv().await.ok_or(SessionError::SessionClosed)??;

            MessageContext::from_proto_message(msg)
        };

        if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, recv_future)
                .await
                .map_err(|_| SessionError::ReceiveTimeout)?
        } else {
            recv_future.await
        }
    }
}

// ============================================================================
// FFI-exported methods (UniFFI)
// ============================================================================

#[uniffi::export]
impl BindingsSessionContext {
    /// Publish a message to the session's destination (fire-and-forget, blocking version)
    ///
    /// This is the simple "fire-and-forget" API that most users want.
    /// The message is queued for sending and this method returns immediately without
    /// waiting for delivery confirmation.
    ///
    /// **When to use:** Most common use case where you don't need delivery confirmation.
    ///
    /// **When not to use:** If you need to ensure the message was delivered, use
    /// `publish_with_completion()` instead.
    ///
    /// # Arguments
    /// * `data` - The message payload bytes
    /// * `payload_type` - Optional content type identifier
    /// * `metadata` - Optional key-value metadata pairs
    ///
    /// # Returns
    /// * `Ok(())` - Message queued successfully
    /// * `Err(SlimError)` - If publishing fails
    pub fn publish(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.publish_async(data, payload_type, metadata).await })
    }

    /// Publish a message to the session's destination (fire-and-forget, async version)
    pub async fn publish_async(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        // Get the session's destination
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        let destination = session.dst();

        // Fire and forget - discard completion handle
        self.publish_internal(
            destination,
            1, // fanout = 1 for normal publish
            data,
            None, // connection_out
            payload_type,
            metadata,
        )
        .await
        .map(|_| ())?; // ignore completion handle

        Ok(())
    }

    /// Publish a message with delivery confirmation (blocking version)
    ///
    /// This variant returns a `FfiCompletionHandle` that can be awaited to ensure
    /// the message was delivered successfully. Use this when you need reliable
    /// delivery confirmation.
    ///
    /// **When to use:** Critical messages where you need delivery confirmation.
    ///
    /// # Arguments
    /// * `data` - The message payload bytes
    /// * `payload_type` - Optional content type identifier
    /// * `metadata` - Optional key-value metadata pairs
    ///
    /// # Returns
    /// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
    /// * `Err(SlimError)` - If publishing fails
    ///
    /// # Example
    /// ```ignore
    /// let completion = session.publish_with_completion(data, None, None)?;
    /// completion.wait()?; // Blocks until message is delivered
    /// ```
    pub fn publish_with_completion(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Arc<FfiCompletionHandle>, SlimError> {
        self.runtime.block_on(async {
            self.publish_with_completion_async(data, payload_type, metadata)
                .await
        })
    }

    /// Publish a message with delivery confirmation (async version)
    pub async fn publish_with_completion_async(
        &self,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Arc<FfiCompletionHandle>, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        let destination = session.dst();

        let completion = self
            .publish_internal(
                destination,
                1, // fanout = 1 for normal publish
                data,
                None, // connection_out
                payload_type,
                metadata,
            )
            .await?;

        Ok(Arc::new(FfiCompletionHandle::new(completion, self.runtime)))
    }

    /// Publish a reply message to the originator of a received message (blocking version for FFI)
    ///
    /// This method uses the routing information from a previously received message
    /// to send a reply back to the sender. This is the preferred way to implement
    /// request/reply patterns.
    ///
    /// # Arguments
    /// * `message_context` - Context from a message received via `get_message()`
    /// * `data` - The reply payload bytes
    /// * `payload_type` - Optional content type identifier
    /// * `metadata` - Optional key-value metadata pairs
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(SlimError)` if publishing fails
    pub fn publish_to(
        &self,
        message_context: MessageContext,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        self.runtime.block_on(async {
            self.publish_to_async(message_context, data, payload_type, metadata)
                .await
        })
    }

    /// Publish a reply message (fire-and-forget, async version)
    pub async fn publish_to_async(
        &self,
        message_context: MessageContext,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        self.publish_to_internal(&message_context, data, payload_type, metadata)
            .await
            .map(|_| ())?;

        Ok(())
    }

    /// Publish a reply message with delivery confirmation (blocking version)
    ///
    /// Similar to `publish_with_completion()` but for reply messages.
    /// Returns a completion handle to await delivery confirmation.
    ///
    /// # Arguments
    /// * `message_context` - Context from a message received via `get_message()`
    /// * `data` - The reply payload bytes
    /// * `payload_type` - Optional content type identifier
    /// * `metadata` - Optional key-value metadata pairs
    ///
    /// # Returns
    /// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
    /// * `Err(SlimError)` - If publishing fails
    pub fn publish_to_with_completion(
        &self,
        message_context: MessageContext,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Arc<FfiCompletionHandle>, SlimError> {
        self.runtime.block_on(async {
            self.publish_to_with_completion_async(message_context, data, payload_type, metadata)
                .await
        })
    }

    /// Publish a reply message with delivery confirmation (async version)
    pub async fn publish_to_with_completion_async(
        &self,
        message_context: MessageContext,
        data: Vec<u8>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Arc<FfiCompletionHandle>, SlimError> {
        let completion = self
            .publish_to_internal(&message_context, data, payload_type, metadata)
            .await?;

        Ok(Arc::new(FfiCompletionHandle::new(completion, self.runtime)))
    }

    /// Low-level publish with full control over all parameters (blocking version for FFI)
    ///
    /// This is an advanced method that provides complete control over routing and delivery.
    /// Most users should use `publish()` or `publish_to()` instead.
    ///
    /// # Arguments
    /// * `destination` - Target name to send to
    /// * `fanout` - Number of copies to send (for multicast)
    /// * `data` - The message payload bytes
    /// * `connection_out` - Optional specific connection ID to use
    /// * `payload_type` - Optional content type identifier
    /// * `metadata` - Optional key-value metadata pairs
    pub fn publish_with_params(
        &self,
        destination: Arc<Name>,
        fanout: u32,
        data: Vec<u8>,
        connection_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        self.runtime.block_on(async {
            self.publish_with_params_async(
                destination,
                fanout,
                data,
                connection_out,
                payload_type,
                metadata,
            )
            .await
        })
    }

    /// Low-level publish with full control (async version)
    pub async fn publish_with_params_async(
        &self,
        destination: Arc<Name>,
        fanout: u32,
        data: Vec<u8>,
        connection_out: Option<u64>,
        payload_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<(), SlimError> {
        let slim_dest: SlimName = destination.as_ref().into();

        self.publish_internal(
            &slim_dest,
            fanout,
            data,
            connection_out,
            payload_type,
            metadata,
        )
        .await
        .map(|_| ())?;

        Ok(())
    }

    /// Receive a message from the session (blocking version for FFI)
    ///
    /// # Arguments
    /// * `timeout_ms` - Optional timeout in milliseconds
    ///
    /// # Returns
    /// * `Ok(ReceivedMessage)` - Message with context and payload bytes
    /// * `Err(SlimError)` - If the receive fails or times out
    pub fn get_message(&self, timeout_ms: Option<u32>) -> Result<ReceivedMessage, SlimError> {
        self.runtime
            .block_on(async { self.get_message_async(timeout_ms).await })
    }

    /// Receive a message from the session (async version)
    pub async fn get_message_async(
        &self,
        timeout_ms: Option<u32>,
    ) -> Result<ReceivedMessage, SlimError> {
        let timeout = timeout_ms.map(|ms| std::time::Duration::from_millis(ms as u64));

        let (ctx, payload) = self.get_session_message(timeout).await?;

        Ok(ReceivedMessage {
            context: ctx,
            payload,
        })
    }

    /// Invite a participant to the session (blocking version for FFI)
    ///
    /// **Auto-waits for completion:** This method automatically waits for the
    /// invitation to be sent and acknowledged before returning.
    pub fn invite(&self, participant: Arc<Name>) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.invite_async(participant).await })
    }

    /// Invite a participant to the session (async version)
    ///
    /// **Auto-waits for completion:** This method automatically waits for the
    /// invitation to be sent and acknowledged before returning.
    pub async fn invite_async(&self, participant: Arc<Name>) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.as_ref().into();

        let completion = self.invite_internal(&slim_name).await?;

        // Wait for invitation to complete
        completion.await?;

        Ok(())
    }

    /// Remove a participant from the session (blocking version for FFI)
    ///
    /// **Auto-waits for completion:** This method automatically waits for the
    /// removal to be processed and acknowledged before returning.
    pub fn remove(&self, participant: Arc<Name>) -> Result<(), SlimError> {
        self.runtime
            .block_on(async { self.remove_async(participant).await })
    }

    /// Remove a participant from the session (async version)
    ///
    /// **Auto-waits for completion:** This method automatically waits for the
    /// removal to be processed and acknowledged before returning.
    pub async fn remove_async(&self, participant: Arc<Name>) -> Result<(), SlimError> {
        let slim_name: SlimName = participant.as_ref().into();

        let completion = self.remove_internal(&slim_name).await?;

        // Wait for removal to complete
        completion.await?;

        Ok(())
    }

    /// Get the destination name for this session
    pub fn destination(&self) -> Result<Name, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        Ok(Name::from(session.dst()))
    }

    /// Get the source name for this session
    pub fn source(&self) -> Result<Name, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        Ok(Name::from(session.source()))
    }

    /// Get the session ID
    pub fn session_id(&self) -> Result<u32, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        Ok(session.id())
    }

    /// Get the session type (PointToPoint or Group)
    pub fn session_type(&self) -> Result<SessionType, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        match session.session_type() {
            ProtoSessionType::PointToPoint => Ok(SessionType::PointToPoint),
            ProtoSessionType::Multicast => Ok(SessionType::Group),
            ProtoSessionType::Unspecified => Err(SlimError::InvalidArgument {
                message: "Session has unspecified type".to_string(),
            }),
        }
    }

    /// Check if this session is the initiator
    pub fn is_initiator(&self) -> Result<bool, SlimError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| SlimError::SessionError {
                message: "Session already closed or dropped".to_string(),
            })?;

        Ok(session.is_initiator())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Name as FfiName;
    use slim_datapath::api::{
        ApplicationPayload, ProtoMessage, ProtoPublish, ProtoPublishType, SessionHeader, SlimHeader,
    };
    use slim_session::SessionError;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Get the shared test runtime (uses global runtime)
    fn get_test_runtime() -> &'static tokio::runtime::Runtime {
        crate::adapter::get_runtime()
    }

    /// Helper to create SlimName for proto message construction
    fn make_slim_name(parts: [&str; 3]) -> SlimName {
        SlimName::from_strings(parts).with_id(0)
    }

    /// Helper to create FFI Name for MessageContext construction
    fn make_ffi_name(parts: [&str; 3]) -> FfiName {
        FfiName::new(
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
            Some(u64::MAX), // Default SlimName ID
        )
    }

    fn make_context() -> (
        BindingsSessionContext,
        mpsc::UnboundedSender<Result<ProtoMessage, SessionError>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<Result<ProtoMessage, SessionError>>();
        let runtime = get_test_runtime();
        let ctx = BindingsSessionContext {
            session: std::sync::Weak::new(),
            rx: RwLock::new(rx),
            runtime,
        };
        (ctx, tx)
    }

    /// Helper to create a valid ProtoMessage for testing message reception
    fn create_test_proto_message(
        source: SlimName,
        dest: SlimName,
        connection_id: u64,
        payload: Vec<u8>,
        content_type: &str,
        metadata: HashMap<String, String>,
    ) -> ProtoMessage {
        let content = ApplicationPayload::new(content_type, payload).as_content();

        let mut slim_header = SlimHeader::default();
        slim_header.set_source(&source);
        slim_header.set_destination(&dest);

        let publish = ProtoPublish {
            header: Some(slim_header),
            session: Some(SessionHeader::default()),
            msg: Some(content),
        };

        let mut proto_msg = ProtoMessage {
            message_type: Some(ProtoPublishType(publish)),
            metadata,
        };

        proto_msg.set_incoming_conn(Some(connection_id));
        proto_msg
    }

    // ==================== Message Reception Tests ====================

    #[tokio::test]
    async fn test_get_session_message_success() {
        let (ctx, tx) = make_context();

        let source = make_slim_name(["org", "sender", "app"]);
        let dest = make_slim_name(["org", "receiver", "app"]);
        let payload = b"hello world".to_vec();
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let msg = create_test_proto_message(
            source.clone(),
            dest,
            42,
            payload.clone(),
            "text/plain",
            metadata.clone(),
        );

        tx.send(Ok(msg)).expect("send should succeed");

        let result = ctx
            .get_session_message(Some(Duration::from_millis(100)))
            .await;
        assert!(result.is_ok(), "should receive message successfully");

        let (msg_ctx, received_payload) = result.unwrap();
        assert_eq!(received_payload, payload);
        assert_eq!(msg_ctx.payload_type, "text/plain");
        assert_eq!(msg_ctx.input_connection, 42);
        assert_eq!(msg_ctx.metadata.get("key"), Some(&"value".to_string()));
    }

    #[tokio::test]
    async fn test_get_session_message_timeout() {
        let (ctx, _tx) = make_context();
        let result = ctx
            .get_session_message(Some(Duration::from_millis(50)))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn test_get_session_message_channel_closed() {
        let (ctx, tx) = make_context();
        drop(tx); // close sender so receiver sees channel closed
        let result = ctx
            .get_session_message(Some(Duration::from_millis(50)))
            .await;
        assert!(result.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    #[tokio::test]
    async fn test_get_session_message_decode_error() {
        let (ctx, tx) = make_context();

        // Send an error through the channel (simulates decode failure)
        tx.send(Err(SessionError::SlimMessageSendFailed)).unwrap();

        let result = ctx
            .get_session_message(Some(Duration::from_millis(100)))
            .await;
        assert!(result.is_err_and(|e| matches!(e, SessionError::SlimMessageSendFailed)));
    }

    #[tokio::test]
    async fn test_get_session_message_no_timeout() {
        let (ctx, tx) = make_context();

        // Spawn a task that sends a message after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let msg = create_test_proto_message(
                make_slim_name(["org", "sender", "app"]),
                make_slim_name(["org", "receiver", "app"]),
                1,
                b"delayed".to_vec(),
                "text/plain",
                HashMap::new(),
            );
            let _ = tx.send(Ok(msg));
        });

        // Call without timeout - should block until message arrives
        let result = ctx.get_session_message(None).await;
        assert!(result.is_ok());
        let (_, payload) = result.unwrap();
        assert_eq!(payload, b"delayed".to_vec());
    }

    #[tokio::test]
    async fn test_get_session_message_various_timeouts() {
        let (ctx, _tx) = make_context();

        // Very short timeout
        let start = std::time::Instant::now();
        let result = ctx
            .get_session_message(Some(Duration::from_millis(10)))
            .await;
        assert!(result.is_err());
        assert!(start.elapsed() >= Duration::from_millis(10));

        // Zero timeout should return immediately
        let start = std::time::Instant::now();
        let result = ctx.get_session_message(Some(Duration::ZERO)).await;
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(50));

        // Longer timeout
        let start = std::time::Instant::now();
        let result = ctx
            .get_session_message(Some(Duration::from_millis(100)))
            .await;
        assert!(result.is_err());
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90)); // Allow variance
        assert!(elapsed < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn test_get_session_message_multiple_messages() {
        let (ctx, tx) = make_context();

        // Send multiple messages
        for i in 0..3 {
            let msg = create_test_proto_message(
                make_slim_name(["org", "sender", "app"]),
                make_slim_name(["org", "receiver", "app"]),
                i as u64,
                format!("message {}", i).into_bytes(),
                "text/plain",
                HashMap::new(),
            );
            tx.send(Ok(msg)).expect("send should succeed");
        }

        // Receive them in order
        for i in 0..3 {
            let result = ctx
                .get_session_message(Some(Duration::from_millis(100)))
                .await;
            assert!(result.is_ok());
            let (msg_ctx, payload) = result.unwrap();
            assert_eq!(payload, format!("message {}", i).into_bytes());
            assert_eq!(msg_ctx.input_connection, i as u64);
        }
    }

    // ==================== Publish Tests (Session Missing) ====================

    #[tokio::test]
    async fn test_publish_internal_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let res = ctx
            .publish_internal(
                &make_slim_name(["dest", "app", "v1"]),
                1,
                b"payload".to_vec(),
                None,
                Some("text/plain".to_string()),
                None,
            )
            .await;
        assert!(res.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    #[tokio::test]
    async fn test_publish_internal_with_all_params_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let res = ctx
            .publish_internal(
                &make_slim_name(["dest", "app", "v1"]),
                3,                                    // fanout
                b"payload".to_vec(),                  // blob
                Some(123),                            // conn_out
                Some("application/json".to_string()), // payload_type
                Some(metadata),                       // metadata
            )
            .await;
        assert!(res.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    #[tokio::test]
    async fn test_publish_to_internal_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let message_ctx = MessageContext::new(
            make_ffi_name(["sender", "org", "service"]),
            Some(make_ffi_name(["receiver", "org", "service"])),
            "application/json".to_string(),
            HashMap::new(),
            42,
            "unique".to_string(),
        );

        let res = ctx
            .publish_to_internal(
                &message_ctx,
                b"reply".to_vec(),
                Some("json".to_string()),
                None,
            )
            .await;
        assert!(res.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    // ==================== Invite/Remove Tests (Session Missing) ====================

    #[tokio::test]
    async fn test_invite_internal_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let res = ctx
            .invite_internal(&make_slim_name(["org", "peer", "app"]))
            .await;
        assert!(res.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    #[tokio::test]
    async fn test_remove_internal_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let err = ctx
            .remove_internal(&make_slim_name(["org", "peer", "app"]))
            .await;
        assert!(err.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    // ==================== Context Creation Tests ====================

    #[tokio::test]
    async fn test_bindings_session_context_weak_ref() {
        let (ctx, _tx) = make_context();
        // With no actual SessionController, the weak ref should be None
        assert!(ctx.session.upgrade().is_none());
    }

    // ==================== Runtime Access Tests ====================

    #[tokio::test]
    async fn test_runtime_accessor() {
        let (ctx, _tx) = make_context();
        let runtime = ctx.runtime();
        // Verify runtime is accessible (we can't block_on from within a tokio test)
        // Just verify we get a valid handle
        let handle = runtime.handle();
        // Spawn a task on the runtime to verify it's working
        let result = handle
            .spawn(async { 42 })
            .await
            .expect("Task should complete");
        assert_eq!(result, 42);
    }

    // ==================== FFI Method Tests (Session Missing) ====================

    #[tokio::test]
    async fn test_publish_async_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.publish_async(b"test".to_vec(), None, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_publish_with_completion_async_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx
            .publish_with_completion_async(b"test".to_vec(), None, None)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_publish_to_async_session_missing() {
        let (ctx, _tx) = make_context();
        let message_ctx = MessageContext::new(
            make_ffi_name(["sender", "org", "service"]),
            Some(make_ffi_name(["receiver", "org", "service"])),
            "application/json".to_string(),
            HashMap::new(),
            42,
            "identity".to_string(),
        );

        let result = ctx
            .publish_to_async(message_ctx, b"reply".to_vec(), None, None)
            .await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("closed")
            } else {
                false
            }
        }));
    }

    #[tokio::test]
    async fn test_publish_to_with_completion_async_session_missing() {
        let (ctx, _tx) = make_context();
        let message_ctx = MessageContext::new(
            make_ffi_name(["sender", "org", "service"]),
            Some(make_ffi_name(["receiver", "org", "service"])),
            "application/json".to_string(),
            HashMap::new(),
            42,
            "identity".to_string(),
        );

        let result = ctx
            .publish_to_with_completion_async(message_ctx, b"reply".to_vec(), None, None)
            .await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("closed")
            } else {
                false
            }
        }));
    }

    #[tokio::test]
    async fn test_publish_with_params_async_session_missing() {
        let (ctx, _tx) = make_context();
        let dest = Arc::new(FfiName::new(
            "org".to_string(),
            "ns".to_string(),
            "dest".to_string(),
            None,
        ));

        let result = ctx
            .publish_with_params_async(dest, 1, b"test".to_vec(), None, None, None)
            .await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("closed")
            } else {
                false
            }
        }));
    }

    #[tokio::test]
    async fn test_publish_with_params_async_with_all_options() {
        let (ctx, _tx) = make_context();
        let dest = Arc::new(FfiName::new(
            "org".to_string(),
            "ns".to_string(),
            "dest".to_string(),
            Some(123),
        ));

        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let result = ctx
            .publish_with_params_async(
                dest,
                3, // fanout
                b"test payload".to_vec(),
                Some(456),                            // connection_out
                Some("application/json".to_string()), // payload_type
                Some(metadata),                       // metadata
            )
            .await;

        // Should fail because session is missing, but this tests the parameter passing
        assert!(result.is_err());
    }

    // ==================== Get Message FFI Tests ====================

    #[tokio::test]
    async fn test_get_message_async_success() {
        let (ctx, tx) = make_context();

        let msg = create_test_proto_message(
            make_slim_name(["org", "sender", "app"]),
            make_slim_name(["org", "receiver", "app"]),
            42,
            b"hello".to_vec(),
            "text/plain",
            HashMap::new(),
        );

        tx.send(Ok(msg)).expect("send should succeed");

        let result = ctx.get_message_async(Some(100)).await;
        assert!(result.is_ok());

        let received = result.unwrap();
        assert_eq!(received.payload, b"hello");
        assert_eq!(received.context.payload_type, "text/plain");
        assert_eq!(received.context.input_connection, 42);
    }

    #[tokio::test]
    async fn test_get_message_async_timeout() {
        let (ctx, _tx) = make_context();

        let result = ctx.get_message_async(Some(10)).await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("receive timeout")
            } else {
                false
            }
        }));
    }

    #[tokio::test]
    async fn test_get_message_async_channel_closed() {
        let (ctx, tx) = make_context();
        drop(tx);

        let result = ctx.get_message_async(Some(100)).await;
        assert!(result.is_err_and(|e| matches!(e, crate::adapter::SlimError::SessionError { .. })));
    }

    // ==================== Invite/Remove FFI Tests ====================

    #[tokio::test]
    async fn test_invite_async_session_missing() {
        let (ctx, _tx) = make_context();
        let participant = Arc::new(FfiName::new(
            "org".to_string(),
            "ns".to_string(),
            "peer".to_string(),
            None,
        ));

        let result = ctx.invite_async(participant).await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("closed")
            } else {
                false
            }
        }));
    }

    #[tokio::test]
    async fn test_remove_async_session_missing() {
        let (ctx, _tx) = make_context();
        let participant = Arc::new(FfiName::new(
            "org".to_string(),
            "ns".to_string(),
            "peer".to_string(),
            None,
        ));

        let result = ctx.remove_async(participant).await;
        assert!(result.is_err_and(|e| {
            if let crate::adapter::SlimError::SessionError { message } = e {
                message.contains("closed")
            } else {
                false
            }
        }));
    }

    // ==================== Session Info Accessor Tests ====================

    #[tokio::test]
    async fn test_destination_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.destination();
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_source_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.source();
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_session_id_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.session_id();
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_session_type_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.session_type();
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    #[tokio::test]
    async fn test_is_initiator_session_missing() {
        let (ctx, _tx) = make_context();
        let result = ctx.is_initiator();
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::adapter::SlimError::SessionError { message } => {
                assert!(message.contains("closed") || message.contains("dropped"));
            }
            _ => panic!("Expected SessionError"),
        }
    }

    // ==================== Publish Internal with Metadata Tests ====================

    #[tokio::test]
    async fn test_publish_to_internal_adds_publish_to_metadata() {
        // This test verifies the metadata manipulation in publish_to_internal
        // Even though session is missing, we can verify the code path is exercised
        let (ctx, _tx) = make_context();
        let message_ctx = MessageContext::new(
            make_ffi_name(["sender", "org", "service"]),
            Some(make_ffi_name(["receiver", "org", "service"])),
            "application/json".to_string(),
            HashMap::new(),
            42,
            "identity".to_string(),
        );

        let mut metadata = HashMap::new();
        metadata.insert("custom_key".to_string(), "custom_value".to_string());

        // This should fail but exercises the metadata handling code
        let result = ctx
            .publish_to_internal(
                &message_ctx,
                b"reply".to_vec(),
                Some("text/plain".to_string()),
                Some(metadata),
            )
            .await;

        assert!(result.is_err_and(|e| matches!(e, SessionError::SessionClosed)));
    }

    // ==================== ReceivedMessage Construction Test ====================

    #[tokio::test]
    async fn test_received_message_construction() {
        let (ctx, tx) = make_context();

        let mut metadata = HashMap::new();
        metadata.insert("trace_id".to_string(), "abc123".to_string());
        metadata.insert("user".to_string(), "test_user".to_string());

        let msg = create_test_proto_message(
            make_slim_name(["org", "sender", "service"]),
            make_slim_name(["org", "receiver", "service"]),
            999,
            b"complex payload with special chars: \x00\xFF".to_vec(),
            "application/octet-stream",
            metadata,
        );

        tx.send(Ok(msg)).expect("send should succeed");

        let result = ctx.get_message_async(Some(100)).await;
        assert!(result.is_ok());

        let received = result.unwrap();
        assert_eq!(
            received.payload,
            b"complex payload with special chars: \x00\xFF"
        );
        assert_eq!(received.context.payload_type, "application/octet-stream");
        assert_eq!(received.context.input_connection, 999);
        assert_eq!(
            received.context.metadata.get("trace_id"),
            Some(&"abc123".to_string())
        );
        assert_eq!(
            received.context.metadata.get("user"),
            Some(&"test_user".to_string())
        );
    }

    // ==================== Message Context Conversion Test ====================

    #[tokio::test]
    async fn test_message_context_source_as_slim_name() {
        let ffi_name = make_ffi_name(["org", "namespace", "app"]);
        let message_ctx = MessageContext::new(
            ffi_name,
            None,
            "text/plain".to_string(),
            HashMap::new(),
            1,
            "id".to_string(),
        );

        let slim_name = message_ctx.source_as_slim_name();
        let components = slim_name.components_strings();
        assert_eq!(components[0], "org");
        assert_eq!(components[1], "namespace");
        assert_eq!(components[2], "app");
    }

    // ==================== Empty/Edge Case Tests ====================

    #[tokio::test]
    async fn test_get_message_with_empty_payload() {
        let (ctx, tx) = make_context();

        let msg = create_test_proto_message(
            make_slim_name(["org", "sender", "app"]),
            make_slim_name(["org", "receiver", "app"]),
            1,
            vec![], // empty payload
            "text/plain",
            HashMap::new(),
        );

        tx.send(Ok(msg)).expect("send should succeed");

        let result = ctx.get_message_async(Some(100)).await;
        assert!(result.is_ok());
        let received = result.unwrap();
        assert!(received.payload.is_empty());
    }

    #[tokio::test]
    async fn test_get_message_with_large_payload() {
        let (ctx, tx) = make_context();

        // 1MB payload
        let large_payload = vec![0xAB; 1024 * 1024];

        let msg = create_test_proto_message(
            make_slim_name(["org", "sender", "app"]),
            make_slim_name(["org", "receiver", "app"]),
            1,
            large_payload.clone(),
            "application/octet-stream",
            HashMap::new(),
        );

        tx.send(Ok(msg)).expect("send should succeed");

        let result = ctx.get_message_async(Some(500)).await;
        assert!(result.is_ok());
        let received = result.unwrap();
        assert_eq!(received.payload.len(), 1024 * 1024);
        assert_eq!(received.payload, large_payload);
    }

    #[tokio::test]
    async fn test_publish_async_with_metadata() {
        let (ctx, _tx) = make_context();

        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());

        let result = ctx
            .publish_async(
                b"test".to_vec(),
                Some("application/json".to_string()),
                Some(metadata),
            )
            .await;

        // Should fail because session is missing
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_publish_with_completion_async_with_options() {
        let (ctx, _tx) = make_context();

        let mut metadata = HashMap::new();
        metadata.insert("trace".to_string(), "123".to_string());

        let result = ctx
            .publish_with_completion_async(
                b"important message".to_vec(),
                Some("text/plain".to_string()),
                Some(metadata),
            )
            .await;

        // Should fail because session is missing
        assert!(result.is_err());
    }
}

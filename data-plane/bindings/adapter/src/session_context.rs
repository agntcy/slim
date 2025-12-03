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
    use super::*;
    use slim_datapath::api::{
        ApplicationPayload, ProtoMessage, ProtoPublish, ProtoPublishType, SessionHeader, SlimHeader,
    };
    use slim_datapath::messages::Name;
    use slim_session::SessionError;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn make_name(parts: [&str; 3]) -> Name {
        Name::from_strings(parts).with_id(0)
    }

    fn make_context() -> (
        BindingsSessionContext,
        mpsc::UnboundedSender<Result<ProtoMessage, SessionError>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<Result<ProtoMessage, SessionError>>();
        let ctx = BindingsSessionContext {
            session: std::sync::Weak::new(),
            rx: RwLock::new(rx),
        };
        (ctx, tx)
    }

    /// Helper to create a valid ProtoMessage for testing message reception
    fn create_test_proto_message(
        source: Name,
        dest: Name,
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

        let source = make_name(["org", "sender", "app"]);
        let dest = make_name(["org", "receiver", "app"]);
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
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("session channel closed")
        );
    }

    #[tokio::test]
    async fn test_get_session_message_decode_error() {
        let (ctx, tx) = make_context();

        // Send an error through the channel (simulates decode failure)
        tx.send(Err(SessionError::Processing("decode failed".to_string())))
            .expect("send should succeed");

        let result = ctx
            .get_session_message(Some(Duration::from_millis(100)))
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to decode message")
        );
    }

    #[tokio::test]
    async fn test_get_session_message_no_timeout() {
        let (ctx, tx) = make_context();

        // Spawn a task that sends a message after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let msg = create_test_proto_message(
                make_name(["org", "sender", "app"]),
                make_name(["org", "receiver", "app"]),
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
                make_name(["org", "sender", "app"]),
                make_name(["org", "receiver", "app"]),
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
    async fn test_publish_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let err = ctx
            .publish(
                &make_name(["dest", "app", "v1"]),
                1,
                b"payload".to_vec(),
                None,
                Some("text/plain".to_string()),
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Session has been dropped"));
    }

    #[tokio::test]
    async fn test_publish_with_all_params_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let err = ctx
            .publish(
                &make_name(["dest", "app", "v1"]),
                3,                                    // fanout
                b"payload".to_vec(),                  // blob
                Some(123),                            // conn_out
                Some("application/json".to_string()), // payload_type
                Some(metadata),                       // metadata
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Session has been dropped"));
    }

    #[tokio::test]
    async fn test_publish_to_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let message_ctx = MessageContext::new(
            make_name(["sender", "org", "service"]),
            Some(make_name(["receiver", "org", "service"])),
            "application/json".to_string(),
            HashMap::new(),
            42,
            "unique".to_string(),
        );

        let err = ctx
            .publish_to(
                &message_ctx,
                b"reply".to_vec(),
                Some("json".to_string()),
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Session has been dropped"));
    }

    // ==================== Invite/Remove Tests (Session Missing) ====================

    #[tokio::test]
    async fn test_invite_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let err = ctx
            .invite(&make_name(["org", "peer", "app"]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Session has been dropped"));
    }

    #[tokio::test]
    async fn test_remove_errors_when_session_missing() {
        let (ctx, _tx) = make_context();
        let err = ctx
            .remove(&make_name(["org", "peer", "app"]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Session has been dropped"));
    }

    // ==================== Context Creation Tests ====================

    #[tokio::test]
    async fn test_bindings_session_context_weak_ref() {
        let (ctx, _tx) = make_context();
        // With no actual SessionController, the weak ref should be None
        assert!(ctx.session.upgrade().is_none());
    }
}

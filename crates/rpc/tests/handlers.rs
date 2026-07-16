// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC using the native trait-object handler
//! registration API and the raw-bytes client call API.
//!
//! These tests exercise code paths that the typed `*_internal` tests in
//! `core.rs` do not cover:
//! - the public `Server::register_*` trait methods (trait-object handlers),
//! - the `handler_traits.rs` traits,
//! - the `stream_types.rs` wrappers `RequestStream`/`ResponseSink`/
//!   `ResponseStreamReader`/`RequestStreamWriter`/`BidiStreamHandler`, and
//! - the raw-bytes `Channel::call_*` methods.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::id::{ID, Kind};
use slim_datapath::api::ProtoName as Name;
use slim_service::service::Service;
use tokio::sync::Mutex;

use slim_rpc::{
    Channel, Context, ResponseSink, RpcCode, RpcError, Server, StreamMessage, StreamStreamHandler,
    StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler, UniffiRequestStream,
};

const TEST_VALID_SECRET: &str = "test-shared-secret-value-0123456789abcdef";

// ============================================================================
// Test Handlers (ported from the FFI e2e tests, using native types)
// ============================================================================

/// Simple echo handler for unary-unary
struct EchoHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for EchoHandler {
    async fn handle(&self, request: Vec<u8>, _context: Arc<Context>) -> Result<Vec<u8>, RpcError> {
        Ok(request)
    }
}

/// Handler that returns an error for unary-unary
struct ErrorHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for ErrorHandler {
    async fn handle(&self, _request: Vec<u8>, _context: Arc<Context>) -> Result<Vec<u8>, RpcError> {
        Err(RpcError::new(
            RpcCode::InvalidArgument,
            "Intentional error".to_string(),
        ))
    }
}

/// Handler that streams responses for unary-stream
struct CounterHandler;

#[async_trait::async_trait]
impl UnaryStreamHandler for CounterHandler {
    async fn handle(
        &self,
        request: Vec<u8>,
        _context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError> {
        let count = if request.len() >= 4 {
            u32::from_le_bytes([request[0], request[1], request[2], request[3]])
        } else {
            3
        };

        for i in 0..count {
            let response = i.to_le_bytes().to_vec();
            sink.send_async(response).await?;
        }

        sink.close_async().await?;
        Ok(())
    }
}

/// Handler that streams responses with an error for unary-stream
struct StreamErrorHandler;

#[async_trait::async_trait]
impl UnaryStreamHandler for StreamErrorHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        _context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError> {
        sink.send_async(vec![1, 2, 3]).await?;
        sink.send_async(vec![4, 5, 6]).await?;

        sink.send_error_async(RpcError::new(
            RpcCode::Internal,
            "Stream error after 2 messages".to_string(),
        ))
        .await?;

        Ok(())
    }
}

/// Handler that accumulates stream input for stream-unary
struct AccumulatorHandler;

#[async_trait::async_trait]
impl StreamUnaryHandler for AccumulatorHandler {
    async fn handle(
        &self,
        stream: Arc<UniffiRequestStream>,
        _context: Arc<Context>,
    ) -> Result<Vec<u8>, RpcError> {
        let mut total = 0u32;
        let mut count = 0u32;

        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    count += 1;
                    if data.len() >= 4 {
                        let value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                        total += value;
                    }
                }
                StreamMessage::Error(e) => return Err(e),
                StreamMessage::End => break,
            }
        }

        let mut result = total.to_le_bytes().to_vec();
        result.extend_from_slice(&count.to_le_bytes());
        Ok(result)
    }
}

/// Handler that detects error in stream input for stream-unary
struct StreamInputErrorHandler;

#[async_trait::async_trait]
impl StreamUnaryHandler for StreamInputErrorHandler {
    async fn handle(
        &self,
        stream: Arc<UniffiRequestStream>,
        _context: Arc<Context>,
    ) -> Result<Vec<u8>, RpcError> {
        let mut count = 0u32;

        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    count += 1;
                    if !data.is_empty() && data[0] == 255 {
                        return Err(RpcError::new(
                            RpcCode::InvalidArgument,
                            format!("Invalid data at message {count}"),
                        ));
                    }
                }
                StreamMessage::Error(e) => return Err(e),
                StreamMessage::End => break,
            }
        }

        Ok(count.to_le_bytes().to_vec())
    }
}

/// Handler that echoes stream for stream-stream
struct StreamEchoHandler;

#[async_trait::async_trait]
impl StreamStreamHandler for StreamEchoHandler {
    async fn handle(
        &self,
        stream: Arc<UniffiRequestStream>,
        _context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError> {
        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    sink.send_async(data).await?;
                }
                StreamMessage::Error(e) => {
                    sink.send_error_async(e).await?;
                    return Ok(());
                }
                StreamMessage::End => {
                    sink.close_async().await?;
                    return Ok(());
                }
            }
        }
    }
}

/// Handler that transforms stream data for stream-stream
struct TransformHandler;

#[async_trait::async_trait]
impl StreamStreamHandler for TransformHandler {
    async fn handle(
        &self,
        stream: Arc<UniffiRequestStream>,
        _context: Arc<Context>,
        sink: Arc<ResponseSink>,
    ) -> Result<(), RpcError> {
        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    let transformed: Vec<u8> = data.iter().map(|&b| b.wrapping_mul(2)).collect();
                    sink.send_async(transformed).await?;
                }
                StreamMessage::Error(e) => {
                    sink.send_error_async(e).await?;
                    return Ok(());
                }
                StreamMessage::End => {
                    sink.close_async().await?;
                    return Ok(());
                }
            }
        }
    }
}

// ============================================================================
// Context Handlers
// ============================================================================

/// Handler that returns context information (session id)
struct ContextInfoHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for ContextInfoHandler {
    async fn handle(&self, _request: Vec<u8>, context: Arc<Context>) -> Result<Vec<u8>, RpcError> {
        let session_id = context.session_id();
        Ok(session_id.as_bytes().to_vec())
    }
}

/// Handler that captures and validates all context information
struct ContextValidationHandler {
    captured_session_id: Arc<Mutex<Option<String>>>,
    captured_metadata: Arc<Mutex<Option<HashMap<String, String>>>>,
    captured_deadline: Arc<Mutex<Option<std::time::SystemTime>>>,
    captured_remaining: Arc<Mutex<Option<Duration>>>,
    captured_is_exceeded: Arc<Mutex<Option<bool>>>,
}

#[async_trait::async_trait]
impl UnaryUnaryHandler for ContextValidationHandler {
    async fn handle(&self, _request: Vec<u8>, context: Arc<Context>) -> Result<Vec<u8>, RpcError> {
        *self.captured_session_id.lock().await = Some(context.session_id());
        *self.captured_metadata.lock().await = Some(context.metadata());
        *self.captured_deadline.lock().await = Some(context.deadline());
        *self.captured_remaining.lock().await = Some(context.remaining_time());
        *self.captured_is_exceeded.lock().await = Some(context.is_deadline_exceeded());

        Ok(b"ok".to_vec())
    }
}

// ============================================================================
// Test Environment Setup (copied from tests/core.rs)
// ============================================================================

struct TestEnv {
    service: Arc<Service>,
    server: Arc<Server>,
    channel: Channel,
}

impl TestEnv {
    async fn new(test_name: &str) -> Self {
        let id = ID::new_with_name(Kind::new("slim").unwrap(), test_name).unwrap();
        let service = Arc::new(Service::new(id));

        let server_name = Name::from_strings(["org", "ns", "server"]);
        let secret = SharedSecret::new("server", TEST_VALID_SECRET).unwrap();

        let (server_app, server_notifications) = service
            .create_app(
                &server_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret.clone()),
            )
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Arc::new(Server::new_internal(
            server_app.clone(),
            server_app.app_name().clone(),
            server_notifications,
        ));

        // Create client
        let client_name = Name::from_strings(["org", "ns", "client"]);
        let secret = SharedSecret::new("client", TEST_VALID_SECRET).unwrap();
        let (client_app, _) = service
            .create_app(
                &client_name,
                AuthProvider::shared_secret(secret.clone()),
                AuthVerifier::shared_secret(secret),
            )
            .unwrap();
        let client_app = Arc::new(client_app);

        let channel = Channel::new_with_members_internal(
            client_app.clone(),
            vec![server_app.app_name().clone()],
            false,
            None,
        )
        .expect("single non-empty member list is always valid");

        Self {
            service,
            server,
            channel,
        }
    }

    async fn start_server(&self) {
        let server = self.server.clone();

        tokio::spawn(async move {
            if let Err(e) = server.serve_async().await {
                tracing::error!("Server error: {:?}", e);
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    async fn shutdown(&mut self) {
        tracing::info!("Shutting down server...");
        self.server.shutdown_internal().await;

        tracing::info!("Shutting down service...");
        self.service.shutdown().await.unwrap();
    }
}

// ============================================================================
// Unary-Unary via trait handlers
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_rpc_trait() {
    let mut env = TestEnv::new("trait-unary-echo").await;

    env.server.register_unary_unary(
        "TestService".to_string(),
        "Echo".to_string(),
        Arc::new(EchoHandler),
    );

    env.start_server().await;

    let request = vec![1, 2, 3, 4, 5];
    let response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "Echo".to_string(),
            request.clone(),
            Some(Duration::from_secs(5)),
            None,
        )
        .await
        .expect("Unary call failed");

    assert_eq!(response, request);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_error_handling_trait() {
    let mut env = TestEnv::new("trait-unary-error").await;

    env.server.register_unary_unary(
        "TestService".to_string(),
        "Error".to_string(),
        Arc::new(ErrorHandler),
    );

    env.start_server().await;

    let result = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "Error".to_string(),
            vec![1, 2, 3],
            Some(Duration::from_secs(30)),
            None,
        )
        .await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code(), RpcCode::InvalidArgument);
    assert!(error.message().contains("Intentional error"));

    env.shutdown().await;
}

// ============================================================================
// Unary-Stream via trait handlers
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_rpc_trait() {
    let mut env = TestEnv::new("trait-unary-stream").await;

    env.server.register_unary_stream(
        "TestService".to_string(),
        "Counter".to_string(),
        Arc::new(CounterHandler),
    );

    env.start_server().await;

    let count = 5u32;
    let request = count.to_le_bytes().to_vec();

    let reader = env
        .channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "Counter".to_string(),
            request,
            Some(Duration::from_secs(30)),
            None,
        )
        .await
        .expect("Unary stream call failed");

    let mut responses = Vec::new();
    loop {
        match reader.next_async().await {
            StreamMessage::Data(data) => responses.push(data),
            StreamMessage::Error(e) => panic!("Unexpected error: {e:?}"),
            StreamMessage::End => break,
        }
    }

    assert_eq!(responses.len(), 5);
    for (i, response) in responses.iter().enumerate() {
        let value = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
        assert_eq!(value, i as u32);
    }

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_error_handling_trait() {
    let mut env = TestEnv::new("trait-unary-stream-error").await;

    env.server.register_unary_stream(
        "TestService".to_string(),
        "StreamError".to_string(),
        Arc::new(StreamErrorHandler),
    );

    env.start_server().await;

    let reader = env
        .channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "StreamError".to_string(),
            vec![1],
            Some(Duration::from_secs(30)),
            None,
        )
        .await
        .expect("Unary stream call failed");

    let mut responses = Vec::new();
    let mut got_error = false;

    loop {
        match reader.next_async().await {
            StreamMessage::Data(data) => responses.push(data),
            StreamMessage::Error(e) => {
                assert_eq!(e.code(), RpcCode::Internal);
                assert!(e.message().contains("Stream error after 2 messages"));
                got_error = true;
                break;
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(responses.len(), 2);
    assert!(got_error);

    env.shutdown().await;
}

// ============================================================================
// Stream-Unary via trait handlers
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_rpc_trait() {
    let mut env = TestEnv::new("trait-stream-unary").await;

    env.server.register_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Arc::new(AccumulatorHandler),
    );

    env.start_server().await;

    let writer = env.channel.call_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Some(Duration::from_secs(30)),
        None,
    );

    let values = vec![10u32, 20u32, 30u32, 40u32];
    for value in &values {
        writer
            .send_async(value.to_le_bytes().to_vec())
            .await
            .expect("Failed to send");
    }

    let response = writer
        .finalize_stream_async()
        .await
        .expect("Failed to finalize");

    assert_eq!(response.len(), 8);
    let total = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
    let count = u32::from_le_bytes([response[4], response[5], response[6], response[7]]);

    assert_eq!(total, 100);
    assert_eq!(count, 4);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_error_handling_trait() {
    let mut env = TestEnv::new("trait-stream-unary-error").await;

    env.server.register_stream_unary(
        "TestService".to_string(),
        "StreamInputError".to_string(),
        Arc::new(StreamInputErrorHandler),
    );

    env.start_server().await;

    let writer = env.channel.call_stream_unary(
        "TestService".to_string(),
        "StreamInputError".to_string(),
        Some(Duration::from_secs(30)),
        None,
    );

    writer
        .send_async(vec![1, 2, 3])
        .await
        .expect("Failed to send");
    writer
        .send_async(vec![255, 0, 0])
        .await
        .expect("Failed to send");

    let result = writer.finalize_stream_async().await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code(), RpcCode::InvalidArgument);
    assert!(error.message().contains("Invalid data"));

    env.shutdown().await;
}

// ============================================================================
// Stream-Stream via trait handlers
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_echo_trait() {
    let mut env = TestEnv::new("trait-stream-stream-echo").await;

    env.server.register_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Arc::new(StreamEchoHandler),
    );

    env.start_server().await;

    let handler = env.channel.call_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Some(Duration::from_secs(30)),
        None,
    );

    let messages = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    for msg in &messages {
        handler
            .send_async(msg.clone())
            .await
            .expect("Failed to send");
    }

    handler.close_send_async().await.expect("Failed to close");

    let mut received = Vec::new();
    loop {
        match handler.recv_async().await {
            StreamMessage::Data(data) => received.push(data),
            StreamMessage::Error(e) => panic!("Unexpected error: {e:?}"),
            StreamMessage::End => break,
        }
    }

    assert_eq!(received.len(), 3);
    assert_eq!(received, messages);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_transform_trait() {
    let mut env = TestEnv::new("trait-stream-stream-transform").await;

    env.server.register_stream_stream(
        "TestService".to_string(),
        "Transform".to_string(),
        Arc::new(TransformHandler),
    );

    env.start_server().await;

    let handler = env.channel.call_stream_stream(
        "TestService".to_string(),
        "Transform".to_string(),
        Some(Duration::from_secs(30)),
        None,
    );

    let handler_clone = handler.clone();
    let send_handle = tokio::spawn(async move {
        let messages = vec![vec![1, 2, 3], vec![10, 20, 30], vec![100]];
        for msg in messages {
            handler_clone.send_async(msg).await.expect("Failed to send");
        }
        handler_clone
            .close_send_async()
            .await
            .expect("Failed to close");
    });

    let mut received = Vec::new();
    loop {
        match handler.recv_async().await {
            StreamMessage::Data(data) => received.push(data),
            StreamMessage::Error(e) => panic!("Unexpected error: {e:?}"),
            StreamMessage::End => break,
        }
    }

    send_handle.await.expect("Send task failed");

    assert_eq!(received.len(), 3);
    assert_eq!(received[0], vec![2, 4, 6]);
    assert_eq!(received[1], vec![20, 40, 60]);
    assert_eq!(received[2], vec![200]);

    env.shutdown().await;
}

// ============================================================================
// Context tests
// ============================================================================

// Returns the handler plus clones of its capture handles so tests can assert on
// what the handler saw after it's been moved into the server registration.
#[allow(clippy::type_complexity)]
fn new_validation_handler() -> (
    ContextValidationHandler,
    Arc<Mutex<Option<String>>>,
    Arc<Mutex<Option<HashMap<String, String>>>>,
    Arc<Mutex<Option<std::time::SystemTime>>>,
    Arc<Mutex<Option<Duration>>>,
    Arc<Mutex<Option<bool>>>,
) {
    let session_id = Arc::new(Mutex::new(None));
    let metadata = Arc::new(Mutex::new(None));
    let deadline = Arc::new(Mutex::new(None));
    let remaining = Arc::new(Mutex::new(None));
    let is_exceeded = Arc::new(Mutex::new(None));

    let handler = ContextValidationHandler {
        captured_session_id: session_id.clone(),
        captured_metadata: metadata.clone(),
        captured_deadline: deadline.clone(),
        captured_remaining: remaining.clone(),
        captured_is_exceeded: is_exceeded.clone(),
    };

    (
        handler,
        session_id,
        metadata,
        deadline,
        remaining,
        is_exceeded,
    )
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_access_trait() {
    let mut env = TestEnv::new("trait-context-access").await;

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ContextInfo".to_string(),
        Arc::new(ContextInfoHandler),
    );

    env.start_server().await;

    let response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ContextInfo".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
            None,
        )
        .await
        .expect("Context call failed");

    assert!(!response.is_empty());

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_session_id_trait() {
    let mut env = TestEnv::new("trait-context-session-id").await;

    let (handler, session_id, ..) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            None,
            None,
        )
        .await
        .expect("Call failed");

    let session_id = session_id.lock().await;
    assert!(session_id.is_some(), "Session ID should be captured");
    assert!(
        !session_id.as_ref().unwrap().is_empty(),
        "Session ID should not be empty"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_metadata_trait() {
    let mut env = TestEnv::new("trait-context-metadata").await;

    let (handler, _sid, metadata, ..) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            None,
            None,
        )
        .await
        .expect("Call failed");

    let metadata = metadata.lock().await;
    assert!(metadata.is_some(), "Metadata should be captured");

    let metadata_map = metadata.as_ref().unwrap();
    assert!(
        metadata_map.contains_key("slimrpc-timeout"),
        "Metadata should contain deadline"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_custom_metadata_trait() {
    let mut env = TestEnv::new("trait-context-custom-metadata").await;

    let (handler, _sid, metadata, ..) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let mut custom_metadata = HashMap::new();
    custom_metadata.insert("authorization".to_string(), "Bearer token123".to_string());
    custom_metadata.insert("request-id".to_string(), "abc-123-xyz".to_string());
    custom_metadata.insert("user-agent".to_string(), "test-client/1.0".to_string());

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
            Some(custom_metadata.clone()),
        )
        .await
        .expect("Call failed");

    let metadata = metadata.lock().await;
    assert!(metadata.is_some(), "Metadata should be captured");

    let metadata_map = metadata.as_ref().unwrap();
    assert_eq!(
        metadata_map.get("authorization"),
        Some(&"Bearer token123".to_string()),
        "Authorization metadata should match"
    );
    assert_eq!(
        metadata_map.get("request-id"),
        Some(&"abc-123-xyz".to_string()),
        "Request ID metadata should match"
    );
    assert_eq!(
        metadata_map.get("user-agent"),
        Some(&"test-client/1.0".to_string()),
        "User agent metadata should match"
    );
    assert!(
        metadata_map.contains_key("slimrpc-timeout"),
        "Metadata should contain deadline"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_deadline_trait() {
    let mut env = TestEnv::new("trait-context-deadline").await;

    let (handler, _sid, _meta, deadline, ..) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let timeout = Duration::from_secs(30);
    let start = std::time::SystemTime::now();

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    let deadline = deadline.lock().await;
    assert!(deadline.is_some(), "Deadline should be captured");

    let captured = deadline.unwrap();
    let expected = start + timeout;

    let diff = if captured > expected {
        captured.duration_since(expected).unwrap()
    } else {
        expected.duration_since(captured).unwrap()
    };

    assert!(
        diff < Duration::from_secs(2),
        "Deadline should be close to expected value"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_remaining_time_trait() {
    let mut env = TestEnv::new("trait-context-remaining-time").await;

    let (handler, _sid, _meta, _deadline, remaining, _exc) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let timeout = Duration::from_secs(60);

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    let remaining = remaining.lock().await;
    assert!(remaining.is_some(), "Remaining time should be captured");

    let remaining_duration = remaining.unwrap();
    assert!(
        remaining_duration > Duration::ZERO,
        "Remaining time should be positive"
    );
    assert!(
        remaining_duration <= timeout,
        "Remaining time should not exceed timeout"
    );
    assert!(
        remaining_duration >= timeout - Duration::from_secs(5),
        "Remaining time should be close to timeout"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_deadline_not_exceeded_trait() {
    let mut env = TestEnv::new("trait-context-not-exceeded").await;

    let (handler, _sid, _meta, _deadline, _remaining, is_exceeded) = new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(60)),
            None,
        )
        .await
        .expect("Call failed");

    let is_exceeded = is_exceeded.lock().await;
    assert!(
        is_exceeded.is_some(),
        "Deadline exceeded status should be captured"
    );
    assert!(
        !is_exceeded.unwrap(),
        "Deadline should not be exceeded for normal calls"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_all_fields_trait() {
    let mut env = TestEnv::new("trait-context-all-fields").await;

    let (handler, session_id, metadata, deadline, remaining, is_exceeded) =
        new_validation_handler();

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let _response = env
        .channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
            None,
        )
        .await
        .expect("Call failed");

    {
        let session_id = session_id.lock().await;
        assert!(session_id.is_some(), "Session ID should be captured");
        assert!(
            !session_id.as_ref().unwrap().is_empty(),
            "Session ID should not be empty"
        );
    }
    {
        let metadata = metadata.lock().await;
        assert!(metadata.is_some(), "Metadata should be captured");
        assert!(
            metadata.as_ref().unwrap().contains_key("slimrpc-timeout"),
            "Metadata should contain deadline"
        );
    }
    {
        let deadline = deadline.lock().await;
        assert!(deadline.is_some(), "Deadline should be captured");
    }
    {
        let remaining = remaining.lock().await;
        assert!(remaining.is_some(), "Remaining time should be captured");
        assert!(
            remaining.unwrap() > Duration::ZERO,
            "Remaining time should be positive"
        );
    }
    {
        let is_exceeded = is_exceeded.lock().await;
        assert!(
            is_exceeded.is_some(),
            "Deadline exceeded status should be captured"
        );
        assert!(!is_exceeded.unwrap(), "Deadline should not be exceeded");
    }

    env.shutdown().await;
}

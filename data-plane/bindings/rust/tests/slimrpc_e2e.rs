// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC UniFFI bindings
//!
//! These tests verify the four RPC interaction patterns through the UniFFI bindings:
//! - Unary-Unary: Single request, single response
//! - Stream-Unary: Streaming requests, single response
//! - Unary-Stream: Single request, streaming responses
//! - Stream-Stream: Streaming requests, streaming responses

use std::sync::Arc;
use std::time::Duration;

use slim_bindings::{
    App, Channel, Code, Direction, IdentityProviderConfig, IdentityVerifierConfig, Name, RpcError,
    Server, StreamMessage, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler,
    UnaryUnaryHandler, initialize_with_defaults,
};

// ============================================================================
// Test Handlers
// ============================================================================

/// Simple echo handler for unary-unary
struct EchoHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for EchoHandler {
    async fn handle(
        &self,
        request: Vec<u8>,
        _context: Arc<slim_bindings::RpcContext>,
    ) -> Result<Vec<u8>, RpcError> {
        // Echo the request back
        println!("EchoHandler received request: {:?}", request);
        Ok(request)
    }
}

/// Handler that returns an error for unary-unary
struct ErrorHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for ErrorHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        _context: Arc<slim_bindings::RpcContext>,
    ) -> Result<Vec<u8>, RpcError> {
        Err(RpcError::new(
            Code::InvalidArgument,
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
        _context: Arc<slim_bindings::RpcContext>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        // Parse count from request (simple u32 encoding)
        let count = if request.len() >= 4 {
            u32::from_le_bytes([request[0], request[1], request[2], request[3]])
        } else {
            3
        };

        // Send count messages
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
        _context: Arc<slim_bindings::RpcContext>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        // Send a couple of messages
        sink.send_async(vec![1, 2, 3]).await?;
        sink.send_async(vec![4, 5, 6]).await?;

        // Then send an error
        sink.send_error_async(RpcError::new(
            Code::Internal,
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
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::RpcContext>,
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

        // Return total and count
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
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::RpcContext>,
    ) -> Result<Vec<u8>, RpcError> {
        let mut count = 0u32;

        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    count += 1;
                    // Check for error marker (first byte == 255)
                    if !data.is_empty() && data[0] == 255 {
                        return Err(RpcError::new(
                            Code::InvalidArgument,
                            format!("Invalid data at message {}", count),
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
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::RpcContext>,
        sink: Arc<slim_bindings::ResponseSink>,
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
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::RpcContext>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    // Double each value
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
// Test Environment Setup
// ============================================================================

struct TestEnv {
    server: Server,
    _app: Arc<App>,
    _server_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TestEnv {
    async fn new(test_name: &str) -> Self {
        println!("TestEnv::new starting for {}", test_name);

        // Initialize the runtime if not already initialized
        initialize_with_defaults();
        println!("Runtime initialized");

        // Create server app
        let server_name = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            test_name.to_string(),
        ));
        println!("Server name created");

        let provider_config = IdentityProviderConfig::SharedSecret {
            id: "test-provider".to_string(),
            data: "test-secret-with-sufficient-length-for-hmac-key".to_string(),
        };
        let verifier_config = IdentityVerifierConfig::SharedSecret {
            id: "test-verifier".to_string(),
            data: "test-secret-with-sufficient-length-for-hmac-key".to_string(),
        };

        println!("Creating server app...");
        let server_app = App::new_with_direction_async(
            server_name.clone(),
            provider_config.clone(),
            verifier_config.clone(),
            Direction::Bidirectional,
        )
        .await
        .expect("Failed to create server app");
        println!("Server app created");

        // Create server using UniFFI constructor
        println!("Creating RPC server...");
        let server = Server::new(&server_app, server_name.clone());
        println!("RPC server created");

        println!("TestEnv::new completed for {}", test_name);
        Self {
            server,
            _app: server_app,
            _server_handle: None,
        }
    }

    async fn start_server(&mut self) {
        // Start server in background
        println!("Starting server in background...");
        self.server.serve_async().await.unwrap();

        // Give server time to start and subscribe
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    async fn create_client(&self, test_name: &str) -> Channel {
        let client_name = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            format!("{}-client", test_name),
        ));

        let provider_config = IdentityProviderConfig::SharedSecret {
            id: "test-provider-client".to_string(),
            data: "test-secret-with-sufficient-length-for-hmac-key".to_string(),
        };
        let verifier_config = IdentityVerifierConfig::SharedSecret {
            id: "test-verifier-client".to_string(),
            data: "test-secret-with-sufficient-length-for-hmac-key".to_string(),
        };

        let client_app = App::new_with_direction_async(
            client_name,
            provider_config,
            verifier_config,
            Direction::Bidirectional,
        )
        .await
        .expect("Failed to create client app");

        let server_name = Arc::new(Name::new(
            "org".to_string(),
            "test".to_string(),
            test_name.to_string(),
        ));

        Channel::new(client_app, server_name)
    }
}

// ============================================================================
// Test 1: Unary-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_rpc() {
    let mut env = TestEnv::new("unary-echo").await;

    // Register echo handler
    println!("Registering EchoHandler...");
    env.server.register_unary_unary(
        "TestService".to_string(),
        "Echo".to_string(),
        Arc::new(EchoHandler),
    );

    env.start_server().await;

    println!("Creating channel...");
    let channel = env.create_client("unary-echo").await;

    // Make a call
    println!("Making unary call...");
    let request = vec![1, 2, 3, 4, 5];
    let response = channel
        .call_unary_async(
            "TestService".to_string(),
            "Echo".to_string(),
            request.clone(),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("Unary call failed");

    assert_eq!(response, request);

    println!("Unary call succeeded");
    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_error_handling() {
    let mut env = TestEnv::new("unary-error").await;

    // Register error handler
    env.server.register_unary_unary(
        "TestService".to_string(),
        "Error".to_string(),
        Arc::new(ErrorHandler),
    );

    env.start_server().await;

    let channel = env.create_client("unary-error").await;

    // Make a call that should fail
    let request = vec![1, 2, 3];
    let result = channel
        .call_unary_async(
            "TestService".to_string(),
            "Error".to_string(),
            request,
            Some(Duration::from_secs(30)),
        )
        .await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("Intentional error"));

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 2: Unary-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_rpc() {
    let mut env = TestEnv::new("unary-stream").await;

    // Register counter handler
    env.server.register_unary_stream(
        "TestService".to_string(),
        "Counter".to_string(),
        Arc::new(CounterHandler),
    );

    env.start_server().await;

    let channel = env.create_client("unary-stream").await;

    // Request 5 messages
    let count = 5u32;
    let request = count.to_le_bytes().to_vec();

    let reader = channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "Counter".to_string(),
            request,
            Some(Duration::from_secs(30)),
        )
        .await
        .expect("Unary stream call failed");

    // Collect all responses
    let mut responses = Vec::new();
    loop {
        match reader.next_async().await {
            StreamMessage::Data(data) => {
                responses.push(data);
            }
            StreamMessage::Error(e) => {
                panic!("Unexpected error: {:?}", e);
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(responses.len(), 5);
    for (i, response) in responses.iter().enumerate() {
        let value = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
        assert_eq!(value, i as u32);
    }

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_error_handling() {
    let mut env = TestEnv::new("unary-stream-error").await;

    // Register error handler
    env.server.register_unary_stream(
        "TestService".to_string(),
        "StreamError".to_string(),
        Arc::new(StreamErrorHandler),
    );

    env.start_server().await;

    let channel = env.create_client("unary-stream-error").await;

    let reader = channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "StreamError".to_string(),
            vec![1],
            Some(Duration::from_secs(30)),
        )
        .await
        .expect("Unary stream call failed");

    // Collect responses until error
    let mut responses = Vec::new();
    let mut got_error = false;

    loop {
        match reader.next_async().await {
            StreamMessage::Data(data) => {
                responses.push(data);
            }
            StreamMessage::Error(e) => {
                assert_eq!(e.code(), Code::Internal);
                assert!(e.message().contains("Stream error after 2 messages"));
                got_error = true;
                break;
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(responses.len(), 2);
    assert!(got_error);

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 3: Stream-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_rpc() {
    let mut env = TestEnv::new("stream-unary").await;

    // Register accumulator handler
    env.server.register_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Arc::new(AccumulatorHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-unary").await;

    // Create stream writer
    let writer = channel.call_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Some(Duration::from_secs(30)),
    );

    // Send multiple values
    let values = vec![10u32, 20u32, 30u32, 40u32];
    for value in &values {
        writer
            .send_async(value.to_le_bytes().to_vec())
            .await
            .expect("Failed to send");
    }

    // Finalize and get response
    let response = writer.finalize_async().await.expect("Failed to finalize");

    // Parse response: total (4 bytes) + count (4 bytes)
    assert_eq!(response.len(), 8);
    let total = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
    let count = u32::from_le_bytes([response[4], response[5], response[6], response[7]]);

    assert_eq!(total, 100); // 10 + 20 + 30 + 40
    assert_eq!(count, 4);

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_error_handling() {
    let mut env = TestEnv::new("stream-unary-error").await;

    // Register error handler
    env.server.register_stream_unary(
        "TestService".to_string(),
        "StreamInputError".to_string(),
        Arc::new(StreamInputErrorHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-unary-error").await;

    // Create stream writer
    let writer = channel.call_stream_unary(
        "TestService".to_string(),
        "StreamInputError".to_string(),
        Some(Duration::from_secs(30)),
    );

    // Send a valid message
    writer
        .send_async(vec![1, 2, 3])
        .await
        .expect("Failed to send");

    // Send an invalid message (starts with 255)
    writer
        .send_async(vec![255, 0, 0])
        .await
        .expect("Failed to send");

    // Finalize - should get an error
    let result = writer.finalize_async().await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("Invalid data"));

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 4: Stream-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_echo() {
    let mut env = TestEnv::new("stream-stream-echo").await;

    // Register stream echo handler
    env.server.register_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Arc::new(StreamEchoHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-stream-echo").await;

    // Create bidirectional stream
    let handler = channel.call_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Some(Duration::from_secs(30)),
    );

    // Send messages
    let messages = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    for msg in &messages {
        handler
            .send_async(msg.clone())
            .await
            .expect("Failed to send");
    }

    // Close send side
    handler.close_send_async().await.expect("Failed to close");

    // Receive echoed messages
    let mut received = Vec::new();
    loop {
        match handler.recv_async().await {
            StreamMessage::Data(data) => {
                received.push(data);
            }
            StreamMessage::Error(e) => {
                panic!("Unexpected error: {:?}", e);
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(received.len(), 3);
    assert_eq!(received, messages);

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_transform() {
    let mut env = TestEnv::new("stream-stream-transform").await;

    // Register transform handler
    env.server.register_stream_stream(
        "TestService".to_string(),
        "Transform".to_string(),
        Arc::new(TransformHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-stream-transform").await;

    // Create bidirectional stream
    let handler = channel.call_stream_stream(
        "TestService".to_string(),
        "Transform".to_string(),
        Some(Duration::from_secs(30)),
    );

    // Send messages in a separate task
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

    // Receive transformed messages
    let mut received = Vec::new();
    loop {
        match handler.recv_async().await {
            StreamMessage::Data(data) => {
                received.push(data);
            }
            StreamMessage::Error(e) => {
                panic!("Unexpected error: {:?}", e);
            }
            StreamMessage::End => break,
        }
    }

    // Wait for send to complete
    send_handle.await.expect("Send task failed");

    // Verify transformation (each byte doubled)
    assert_eq!(received.len(), 3);
    assert_eq!(received[0], vec![2, 4, 6]); // [1,2,3] * 2
    assert_eq!(received[1], vec![20, 40, 60]); // [10,20,30] * 2
    assert_eq!(received[2], vec![200]); // [100] * 2

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 5: Multiple concurrent calls
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_concurrent_unary_calls() {
    let mut env = TestEnv::new("concurrent-unary").await;

    // Register echo handler
    env.server.register_unary_unary(
        "TestService".to_string(),
        "Echo".to_string(),
        Arc::new(EchoHandler),
    );

    env.start_server().await;

    let channel = env.create_client("concurrent-unary").await;

    // Make 10 concurrent calls
    let mut handles = Vec::new();
    for i in 0..10 {
        let channel = channel.clone();
        let handle = tokio::spawn(async move {
            let request = vec![i as u8; 10];
            let response = channel
                .call_unary_async(
                    "TestService".to_string(),
                    "Echo".to_string(),
                    request.clone(),
                    Some(Duration::from_secs(30)),
                )
                .await
                .expect("Unary call failed");

            assert_eq!(response, request);
        });
        handles.push(handle);
    }

    // Wait for all calls to complete
    for handle in handles {
        handle.await.expect("Task panicked");
    }

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 6: Handler registration
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_handler_registration() {
    let mut env = TestEnv::new("handler-registration").await;

    // Register multiple handlers
    env.server.register_unary_unary(
        "TestService".to_string(),
        "Echo1".to_string(),
        Arc::new(EchoHandler),
    );

    env.server.register_unary_unary(
        "TestService".to_string(),
        "Echo2".to_string(),
        Arc::new(EchoHandler),
    );

    env.server.register_unary_stream(
        "TestService".to_string(),
        "Counter".to_string(),
        Arc::new(CounterHandler),
    );

    env.start_server().await;

    // Get list of registered methods
    let methods = env.server.methods();
    assert!(methods.len() >= 3);

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 7: Context information
// ============================================================================

/// Handler that returns context information
struct ContextInfoHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for ContextInfoHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        context: Arc<slim_bindings::RpcContext>,
    ) -> Result<Vec<u8>, RpcError> {
        // Access context information
        let session_id = context.session_id();

        // Return session ID as bytes
        Ok(session_id.as_bytes().to_vec())
    }
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_access() {
    let mut env = TestEnv::new("context-access").await;

    // Register context info handler
    env.server.register_unary_unary(
        "TestService".to_string(),
        "ContextInfo".to_string(),
        Arc::new(ContextInfoHandler),
    );

    env.start_server().await;

    let channel = env.create_client("context-access").await;

    let response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ContextInfo".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
        )
        .await
        .expect("Context call failed");

    // Should get a session ID back
    assert!(!response.is_empty());

    env.server.shutdown_async().await;
}

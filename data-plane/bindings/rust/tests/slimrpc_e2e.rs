// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC UniFFI bindings
//!
//! These tests verify the four RPC interaction patterns through the UniFFI bindings:
//! - Unary-Unary: Single request, single response
//! - Stream-Unary: Streaming requests, single response
//! - Unary-Stream: Single request, streaming responses
//! - Stream-Stream: Streaming requests, streaming responses

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use slim_bindings::{
    App, Channel, Direction, IdentityProviderConfig, IdentityVerifierConfig, Name, RpcCode,
    RpcError, Server, StreamMessage, StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler,
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
        _context: Arc<slim_bindings::Context>,
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
        _context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
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
        _context: Arc<slim_bindings::Context>,
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
        _context: Arc<slim_bindings::Context>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        // Send a couple of messages
        sink.send_async(vec![1, 2, 3]).await?;
        sink.send_async(vec![4, 5, 6]).await?;

        // Then send an error
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
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::Context>,
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
        _context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        let mut count = 0u32;

        loop {
            match stream.next_async().await {
                StreamMessage::Data(data) => {
                    count += 1;
                    // Check for error marker (first byte == 255)
                    if !data.is_empty() && data[0] == 255 {
                        return Err(RpcError::new(
                            RpcCode::InvalidArgument,
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
        _context: Arc<slim_bindings::Context>,
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
        _context: Arc<slim_bindings::Context>,
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
// Slow Handlers for Deadline Testing
// ============================================================================

/// Handler that sleeps for 2 seconds before responding
struct SlowUnaryHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for SlowUnaryHandler {
    async fn handle(
        &self,
        request: Vec<u8>,
        _context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(request)
    }
}

/// Handler that generates stream items slowly
struct SlowStreamHandler {
    started: Arc<Mutex<bool>>,
    completed: Arc<Mutex<bool>>,
    items_sent: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl UnaryStreamHandler for SlowStreamHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        _context: Arc<slim_bindings::Context>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        *self.started.lock().await = true;

        for i in 0u32..5 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let response = i.to_le_bytes().to_vec();
            sink.send_async(response).await?;

            let mut count = self.items_sent.lock().await;
            *count += 1;
            drop(count);
        }

        sink.close_async().await?;
        *self.completed.lock().await = true;
        Ok(())
    }
}

/// Handler that takes long to setup before returning stream
struct SlowSetupStreamHandler {
    started: Arc<Mutex<bool>>,
    completed: Arc<Mutex<bool>>,
}

#[async_trait::async_trait]
impl UnaryStreamHandler for SlowSetupStreamHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        _context: Arc<slim_bindings::Context>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        *self.started.lock().await = true;

        // Sleep for 2 seconds before starting to stream
        tokio::time::sleep(Duration::from_secs(2)).await;

        for i in 0u32..3 {
            let response = i.to_le_bytes().to_vec();
            sink.send_async(response).await?;
        }

        sink.close_async().await?;
        *self.completed.lock().await = true;
        Ok(())
    }
}

/// Handler that processes stream input slowly
struct SlowStreamUnaryHandler {
    started: Arc<Mutex<bool>>,
    completed: Arc<Mutex<bool>>,
    messages_received: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl StreamUnaryHandler for SlowStreamUnaryHandler {
    async fn handle(
        &self,
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        *self.started.lock().await = true;

        let mut count = 0u32;

        loop {
            match stream.next_async().await {
                StreamMessage::Data(_msg) => {
                    count += 1;
                    *self.messages_received.lock().await = count;

                    // Slow processing
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    println!("Processed message {}", count);
                }
                StreamMessage::End => break,
                StreamMessage::Error(e) => return Err(e),
            }
        }

        // Additional slow processing after receiving all messages
        tokio::time::sleep(Duration::from_secs(2)).await;

        *self.completed.lock().await = true;
        Ok(count.to_le_bytes().to_vec())
    }
}

/// Handler that processes stream-to-stream slowly
struct SlowStreamStreamHandler {
    started: Arc<Mutex<bool>>,
    completed: Arc<Mutex<bool>>,
}

#[async_trait::async_trait]
impl StreamStreamHandler for SlowStreamStreamHandler {
    async fn handle(
        &self,
        stream: Arc<slim_bindings::RequestStream>,
        _context: Arc<slim_bindings::Context>,
        sink: Arc<slim_bindings::ResponseSink>,
    ) -> Result<(), RpcError> {
        *self.started.lock().await = true;

        // Consume one message then sleep for a long time
        match stream.next_async().await {
            StreamMessage::Data(_msg) => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            StreamMessage::End => {}
            StreamMessage::Error(e) => return Err(e),
        }

        for i in 0u32..3 {
            let response = i.to_le_bytes().to_vec();
            sink.send_async(response).await?;
        }

        sink.close_async().await?;
        *self.completed.lock().await = true;
        Ok(())
    }
}

/// Handler that ignores deadline and runs for a long time
struct LongRunningHandler {
    started: Arc<Mutex<bool>>,
    completed: Arc<Mutex<bool>>,
}

#[async_trait::async_trait]
impl UnaryUnaryHandler for LongRunningHandler {
    async fn handle(
        &self,
        request: Vec<u8>,
        _context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        *self.started.lock().await = true;
        println!("LongRunningHandler started, will run for 5 seconds");

        tokio::time::sleep(Duration::from_secs(5)).await;

        *self.completed.lock().await = true;
        println!("LongRunningHandler completed");

        Ok(request)
    }
}

/// Handler that captures deadline from context
struct DeadlineCaptureHandler {
    captured_deadline: Arc<Mutex<Option<std::time::SystemTime>>>,
}

#[async_trait::async_trait]
impl UnaryUnaryHandler for DeadlineCaptureHandler {
    async fn handle(
        &self,
        request: Vec<u8>,
        context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        *self.captured_deadline.lock().await = Some(context.deadline());
        Ok(request)
    }
}

// ============================================================================
// Test Environment Setup
// ============================================================================

struct TestEnv {
    server: Arc<Server>,
    _app: Arc<App>,
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
        let server = Arc::new(Server::new(&server_app, server_name.clone()));
        println!("RPC server created");

        println!("TestEnv::new completed for {}", test_name);
        Self {
            server,
            _app: server_app,
        }
    }

    async fn start_server(&self) {
        // Start server in background
        println!("Starting server in background...");
        let server = self.server.clone();

        // Spawn task to run the server
        tokio::spawn(async move {
            if let Err(e) = server.serve_async().await {
                eprintln!("Server error: {:?}", e);
            }
        });

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
    let env = TestEnv::new("unary-echo").await;

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
            None,
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
    let env = TestEnv::new("unary-error").await;

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
            None,
        )
        .await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.code(), RpcCode::InvalidArgument);
    assert!(error.message().contains("Intentional error"));

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 2: Unary-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_rpc() {
    let env = TestEnv::new("unary-stream").await;

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
            None,
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
    let env = TestEnv::new("unary-stream-error").await;

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
            None,
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

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 3: Stream-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_rpc() {
    let env = TestEnv::new("stream-unary").await;

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
        None,
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
    let env = TestEnv::new("stream-unary-error").await;

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
        None,
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
    assert_eq!(error.code(), RpcCode::InvalidArgument);
    assert!(error.message().contains("Invalid data"));

    env.server.shutdown_async().await;
}

// ============================================================================
// Test 4: Stream-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_echo() {
    let env = TestEnv::new("stream-stream-echo").await;

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
        None,
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
    let env = TestEnv::new("stream-stream-transform").await;

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
        None,
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
    let env = TestEnv::new("concurrent-unary").await;

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
                    None,
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
    let env = TestEnv::new("handler-registration").await;

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
        context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        // Access context information
        let session_id = context.session_id();

        // Return session ID as bytes
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
    async fn handle(
        &self,
        _request: Vec<u8>,
        context: Arc<slim_bindings::Context>,
    ) -> Result<Vec<u8>, RpcError> {
        // Capture session ID
        let session_id = context.session_id();
        *self.captured_session_id.lock().await = Some(session_id.clone());

        // Capture metadata
        let metadata = context.metadata();
        *self.captured_metadata.lock().await = Some(metadata.clone());

        // Capture deadline
        let deadline = context.deadline();
        *self.captured_deadline.lock().await = Some(deadline);

        // Capture remaining time
        let remaining = context.remaining_time();
        *self.captured_remaining.lock().await = Some(remaining);

        // Capture deadline exceeded status
        let is_exceeded = context.is_deadline_exceeded();
        *self.captured_is_exceeded.lock().await = Some(is_exceeded);

        Ok(b"ok".to_vec())
    }
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_access() {
    let env = TestEnv::new("context-access").await;

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
            None,
        )
        .await
        .expect("Context call failed");

    // Should get a session ID back
    assert!(!response.is_empty());

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_session_id() {
    let env = TestEnv::new("context-session-id").await;

    let captured_session_id = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: captured_session_id.clone(),
        captured_metadata: Arc::new(Mutex::new(None)),
        captured_deadline: Arc::new(Mutex::new(None)),
        captured_remaining: Arc::new(Mutex::new(None)),
        captured_is_exceeded: Arc::new(Mutex::new(None)),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-session-id").await;

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            None,
            None,
        )
        .await
        .expect("Call failed");

    // Verify session ID was captured and is not empty
    let session_id = captured_session_id.lock().await;
    assert!(session_id.is_some(), "Session ID should be captured");
    assert!(
        !session_id.as_ref().unwrap().is_empty(),
        "Session ID should not be empty"
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_metadata() {
    let env = TestEnv::new("context-metadata").await;

    let captured_metadata = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: Arc::new(Mutex::new(None)),
        captured_metadata: captured_metadata.clone(),
        captured_deadline: Arc::new(Mutex::new(None)),
        captured_remaining: Arc::new(Mutex::new(None)),
        captured_is_exceeded: Arc::new(Mutex::new(None)),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-metadata").await;

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            None,
            None,
        )
        .await
        .expect("Call failed");

    // Verify metadata was captured
    let metadata = captured_metadata.lock().await;
    assert!(metadata.is_some(), "Metadata should be captured");

    // Metadata should contain at least the deadline key
    let metadata_map = metadata.as_ref().unwrap();
    assert!(
        metadata_map.contains_key("slimrpc-timeout"),
        "Metadata should contain deadline"
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_custom_metadata() {
    let env = TestEnv::new("context-custom-metadata").await;

    let captured_metadata = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: Arc::new(Mutex::new(None)),
        captured_metadata: captured_metadata.clone(),
        captured_deadline: Arc::new(Mutex::new(None)),
        captured_remaining: Arc::new(Mutex::new(None)),
        captured_is_exceeded: Arc::new(Mutex::new(None)),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-custom-metadata").await;

    // Create custom metadata
    let mut custom_metadata = HashMap::new();
    custom_metadata.insert("authorization".to_string(), "Bearer token123".to_string());
    custom_metadata.insert("request-id".to_string(), "abc-123-xyz".to_string());
    custom_metadata.insert("user-agent".to_string(), "test-client/1.0".to_string());

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
            Some(custom_metadata.clone()),
        )
        .await
        .expect("Call failed");

    // Verify metadata was captured
    let metadata = captured_metadata.lock().await;
    assert!(metadata.is_some(), "Metadata should be captured");

    let metadata_map = metadata.as_ref().unwrap();

    // Verify custom metadata fields are present
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

    // Verify deadline is still present
    assert!(
        metadata_map.contains_key("slimrpc-timeout"),
        "Metadata should contain deadline"
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_with_metadata() {
    let env = TestEnv::new("unary-stream-metadata").await;

    // Register counter handler
    env.server.register_unary_stream(
        "TestService".to_string(),
        "Counter".to_string(),
        Arc::new(CounterHandler),
    );

    env.start_server().await;

    let channel = env.create_client("unary-stream-metadata").await;

    // Create custom metadata
    let mut custom_metadata = HashMap::new();
    custom_metadata.insert("client-id".to_string(), "test-client-123".to_string());
    custom_metadata.insert("stream-type".to_string(), "counter".to_string());

    let count = 5u32;
    let request = count.to_le_bytes().to_vec(); // Request 5 items
    let reader = channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "Counter".to_string(),
            request,
            Some(Duration::from_secs(30)),
            Some(custom_metadata),
        )
        .await
        .expect("Unary stream call with metadata failed");

    // Read all responses
    let mut count = 0;
    loop {
        match reader.next_async().await {
            StreamMessage::Data(_data) => {
                count += 1;
            }
            StreamMessage::Error(e) => {
                panic!("Stream error: {:?}", e);
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(count, 5, "Should receive 5 items");

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_with_metadata() {
    let env = TestEnv::new("stream-unary-metadata").await;

    // Register accumulator handler
    env.server.register_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Arc::new(AccumulatorHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-unary-metadata").await;

    // Create custom metadata
    let mut custom_metadata = HashMap::new();
    custom_metadata.insert("operation".to_string(), "sum".to_string());
    custom_metadata.insert("client-version".to_string(), "2.0".to_string());

    // Create stream writer with metadata
    let writer = channel.call_stream_unary(
        "TestService".to_string(),
        "Accumulator".to_string(),
        Some(Duration::from_secs(30)),
        Some(custom_metadata),
    );

    // Send multiple values
    let values = vec![1u32, 2, 3, 4, 5];
    for val in &values {
        writer
            .send_async(val.to_le_bytes().to_vec())
            .await
            .expect("Failed to send value");
    }

    // Finalize and get response
    let response = writer.finalize_async().await.expect("Failed to finalize");
    let sum = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);

    let expected_sum: u32 = values.iter().sum();
    assert_eq!(sum, expected_sum, "Sum should match");

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_with_metadata() {
    let env = TestEnv::new("stream-stream-metadata").await;

    // Register stream echo handler
    env.server.register_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Arc::new(StreamEchoHandler),
    );

    env.start_server().await;

    let channel = env.create_client("stream-stream-metadata").await;

    // Create custom metadata
    let mut custom_metadata = HashMap::new();
    custom_metadata.insert("correlation-id".to_string(), "stream-123".to_string());
    custom_metadata.insert("echo-mode".to_string(), "bidirectional".to_string());

    // Create bidirectional stream with metadata
    let handler = channel.call_stream_stream(
        "TestService".to_string(),
        "StreamEcho".to_string(),
        Some(Duration::from_secs(30)),
        Some(custom_metadata),
    );

    // Send messages
    let messages = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    for msg in &messages {
        handler.send_async(msg.clone()).await.expect("Send failed");
    }

    // Close the send side
    handler.close_send_async().await.expect("Close failed");

    // Receive all echoed messages
    let mut received = Vec::new();
    loop {
        match handler.recv_async().await {
            StreamMessage::Data(data) => {
                received.push(data);
            }
            StreamMessage::Error(e) => {
                panic!("Stream error: {:?}", e);
            }
            StreamMessage::End => break,
        }
    }

    assert_eq!(
        received.len(),
        messages.len(),
        "Should receive all messages"
    );
    for (sent, recv) in messages.iter().zip(received.iter()) {
        assert_eq!(sent, recv, "Messages should match");
    }

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_deadline() {
    let env = TestEnv::new("context-deadline").await;

    let captured_deadline = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: Arc::new(Mutex::new(None)),
        captured_metadata: Arc::new(Mutex::new(None)),
        captured_deadline: captured_deadline.clone(),
        captured_remaining: Arc::new(Mutex::new(None)),
        captured_is_exceeded: Arc::new(Mutex::new(None)),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-deadline").await;

    let timeout = Duration::from_secs(30);
    let start = std::time::SystemTime::now();

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    // Verify deadline was captured and is reasonable
    let deadline = captured_deadline.lock().await;
    assert!(deadline.is_some(), "Deadline should be captured");

    let captured = deadline.unwrap();
    let expected = start + timeout;

    // Deadline should be approximately start + timeout (within 2 seconds)
    let diff = if captured > expected {
        captured.duration_since(expected).unwrap()
    } else {
        expected.duration_since(captured).unwrap()
    };

    assert!(
        diff < Duration::from_secs(2),
        "Deadline should be close to expected value"
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_remaining_time() {
    let env = TestEnv::new("context-remaining-time").await;

    let captured_remaining = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: Arc::new(Mutex::new(None)),
        captured_metadata: Arc::new(Mutex::new(None)),
        captured_deadline: Arc::new(Mutex::new(None)),
        captured_remaining: captured_remaining.clone(),
        captured_is_exceeded: Arc::new(Mutex::new(None)),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-remaining-time").await;

    let timeout = Duration::from_secs(60);

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    // Verify remaining time was captured
    let remaining = captured_remaining.lock().await;
    assert!(remaining.is_some(), "Remaining time should be captured");

    let remaining_duration = remaining.unwrap();

    // Remaining time should be positive and less than or equal to timeout
    assert!(
        remaining_duration > Duration::ZERO,
        "Remaining time should be positive"
    );
    assert!(
        remaining_duration <= timeout,
        "Remaining time should not exceed timeout"
    );

    // Should be close to the timeout (within 5 seconds margin for overhead)
    assert!(
        remaining_duration >= timeout - Duration::from_secs(5),
        "Remaining time should be close to timeout"
    );

    env.server.shutdown_async().await;
}

// ============================================================================
// Deadline Tests - Client-side enforcement
// ============================================================================

#[tokio::test]
async fn test_client_deadline_unary_unary() {
    let env = TestEnv::new("client-deadline-unary").await;

    env.server.register_unary_unary(
        "TestService".to_string(),
        "SlowMethod".to_string(),
        Arc::new(SlowUnaryHandler),
    );

    env.start_server().await;

    let channel = env.create_client("client-deadline-unary").await;

    let request = vec![1, 2, 3, 4];

    // Call with a very short timeout (100ms) while handler takes 2 seconds
    let result = channel
        .call_unary_async(
            "TestService".to_string(),
            "SlowMethod".to_string(),
            request,
            Some(Duration::from_millis(100)),
            None,
        )
        .await;

    // Should timeout on the client side
    assert!(result.is_err(), "Expected timeout error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        RpcCode::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
async fn test_client_deadline_unary_stream() {
    let env = TestEnv::new("client-deadline-unary-stream").await;

    let started = Arc::new(Mutex::new(false));
    let completed = Arc::new(Mutex::new(false));
    let items_sent = Arc::new(Mutex::new(0u32));

    let handler = Arc::new(SlowStreamHandler {
        started: started.clone(),
        completed: completed.clone(),
        items_sent: items_sent.clone(),
    });

    env.server
        .register_unary_stream("TestService".to_string(), "SlowStream".to_string(), handler);

    env.start_server().await;

    let channel = env.create_client("client-deadline-unary-stream").await;

    let request = vec![1, 2, 3];

    // Call with a timeout of 1 second (should only get 2 items before timeout)
    let reader = channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "SlowStream".to_string(),
            request,
            Some(Duration::from_secs(1)),
            None,
        )
        .await
        .expect("Failed to create stream");

    let mut count = 0;
    let mut got_error = false;

    loop {
        match reader.next_async().await {
            StreamMessage::Data(_msg) => {
                count += 1;
            }
            StreamMessage::End => break,
            StreamMessage::Error(e) => {
                got_error = true;
                assert_eq!(
                    e.code(),
                    RpcCode::DeadlineExceeded,
                    "Expected DeadlineExceeded, got {:?}",
                    e.code()
                );
                break;
            }
        }
    }

    // Should have received some items but not all (5 total)
    assert!(
        count < 5,
        "Expected timeout before all items, got {}",
        count
    );
    assert!(got_error, "Expected a deadline exceeded error");

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify handler started but did not complete
    let was_started = *started.lock().await;
    let was_completed = *completed.lock().await;
    let sent = *items_sent.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");
    assert!(
        sent < 5,
        "Handler should not have sent all items, sent {}",
        sent
    );

    env.server.shutdown_async().await;
}

// ============================================================================
// Deadline Tests - Server-side enforcement
// ============================================================================

#[tokio::test]
async fn test_server_deadline_unary_unary() {
    let env = TestEnv::new("server-deadline-unary").await;

    env.server.register_unary_unary(
        "TestService".to_string(),
        "SlowHandler".to_string(),
        Arc::new(SlowUnaryHandler),
    );

    env.start_server().await;

    let channel = env.create_client("server-deadline-unary").await;

    let request = vec![1, 2, 3, 4];

    // Call with a short timeout (500ms) - server should enforce this
    let result = channel
        .call_unary_async(
            "TestService".to_string(),
            "SlowHandler".to_string(),
            request,
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should timeout on the server side
    assert!(result.is_err(), "Expected timeout error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        RpcCode::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
async fn test_server_deadline_unary_stream() {
    let env = TestEnv::new("server-deadline-unary-stream").await;

    let started = Arc::new(Mutex::new(false));
    let completed = Arc::new(Mutex::new(false));

    let handler = Arc::new(SlowSetupStreamHandler {
        started: started.clone(),
        completed: completed.clone(),
    });

    env.server.register_unary_stream(
        "TestService".to_string(),
        "SlowStreamHandler".to_string(),
        handler,
    );

    env.start_server().await;

    let channel = env.create_client("server-deadline-unary-stream").await;

    let request = vec![1, 2, 3];

    // Call with a short timeout (500ms) while handler setup takes 2 seconds
    let reader = channel
        .call_unary_stream_async(
            "TestService".to_string(),
            "SlowStreamHandler".to_string(),
            request,
            Some(Duration::from_millis(500)),
            None,
        )
        .await
        .expect("Failed to create stream");

    // Should get a deadline exceeded error when trying to read
    match reader.next_async().await {
        StreamMessage::Error(e) => {
            assert_eq!(
                e.code(),
                RpcCode::DeadlineExceeded,
                "Expected DeadlineExceeded, got {:?}",
                e.code()
            );
        }
        _ => panic!("Expected deadline exceeded error"),
    }

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify handler started but did not complete
    let was_started = *started.lock().await;
    let was_completed = *completed.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");

    env.server.shutdown_async().await;
}

#[tokio::test]
async fn test_server_deadline_stream_unary() {
    let env = TestEnv::new("server-deadline-stream-unary").await;

    let started = Arc::new(Mutex::new(false));
    let completed = Arc::new(Mutex::new(false));
    let messages_received = Arc::new(Mutex::new(0u32));

    let handler = Arc::new(SlowStreamUnaryHandler {
        started: started.clone(),
        completed: completed.clone(),
        messages_received: messages_received.clone(),
    });

    env.server.register_stream_unary(
        "TestService".to_string(),
        "SlowStreamUnary".to_string(),
        handler,
    );

    env.start_server().await;

    let channel = env.create_client("server-deadline-stream-unary").await;

    // Create stream writer
    let writer = channel.call_stream_unary(
        "TestService".to_string(),
        "SlowStreamUnary".to_string(),
        Some(Duration::from_millis(500)),
        None,
    );

    // Send messages
    let messages = vec![vec![1u8], vec![2u8], vec![3u8]];
    for msg in messages {
        writer.send_async(msg).await.expect("Failed to send");
    }

    // Finalize and get response - should timeout on the server side
    let result = writer.finalize_async().await;

    assert!(result.is_err(), "Expected timeout error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        RpcCode::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify handler started but did not complete
    let was_started = *started.lock().await;
    let was_completed = *completed.lock().await;
    let received = *messages_received.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");
    println!("Handler received {} messages before deadline", received);

    env.server.shutdown_async().await;
}

#[tokio::test]
async fn test_server_deadline_stream_stream() {
    let env = TestEnv::new("server-deadline-stream-stream").await;

    let started = Arc::new(Mutex::new(false));
    let completed = Arc::new(Mutex::new(false));

    let slow_handler = Arc::new(SlowStreamStreamHandler {
        started: started.clone(),
        completed: completed.clone(),
    });

    env.server.register_stream_stream(
        "TestService".to_string(),
        "SlowStreamStream".to_string(),
        slow_handler,
    );

    env.start_server().await;

    let channel = env.create_client("server-deadline-stream-stream").await;

    // Call with a short timeout (500ms) while handler takes 2 seconds
    let handler = channel.call_stream_stream(
        "TestService".to_string(),
        "SlowStreamStream".to_string(),
        Some(Duration::from_millis(500)),
        None,
    );

    // Send messages
    let messages = vec![vec![1u8], vec![2u8], vec![3u8]];
    for msg in messages {
        handler.send_async(msg).await.expect("Failed to send");
    }

    handler.close_send_async().await.expect("Failed to close");

    // Should get a deadline exceeded error when trying to read
    match handler.recv_async().await {
        StreamMessage::Error(e) => {
            assert_eq!(
                e.code(),
                RpcCode::DeadlineExceeded,
                "Expected DeadlineExceeded, got {:?}",
                e.code()
            );
        }
        _ => panic!("Expected deadline exceeded error"),
    }

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify handler started but did not complete
    let was_started = *started.lock().await;
    let was_completed = *completed.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");

    env.server.shutdown_async().await;
}

// ============================================================================
// Deadline Tests - Handler execution enforcement
// ============================================================================

#[tokio::test]
async fn test_server_enforces_deadline_during_handler_execution() {
    let env = TestEnv::new("server-enforces-deadline").await;

    let started = Arc::new(Mutex::new(false));
    let completed = Arc::new(Mutex::new(false));

    let handler = Arc::new(LongRunningHandler {
        started: started.clone(),
        completed: completed.clone(),
    });

    env.server.register_unary_unary(
        "TestService".to_string(),
        "LongRunning".to_string(),
        handler,
    );

    env.start_server().await;

    let channel = env.create_client("server-enforces-deadline").await;

    let request = vec![1, 2, 3, 4];

    // Set a short deadline (500ms) while handler takes 5 seconds
    let result = channel
        .call_unary_async(
            "TestService".to_string(),
            "LongRunning".to_string(),
            request,
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should fail with deadline exceeded
    assert!(result.is_err(), "Expected deadline exceeded error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        RpcCode::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the handler started but did not complete
    let was_started = *started.lock().await;
    let was_completed = *completed.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");

    env.server.shutdown_async().await;
}

// ============================================================================
// Deadline Tests - Deadline propagation
// ============================================================================

#[tokio::test]
async fn test_deadline_propagation() {
    let env = TestEnv::new("deadline-propagation").await;

    let captured_deadline = Arc::new(Mutex::new(None));

    let handler = Arc::new(DeadlineCaptureHandler {
        captured_deadline: captured_deadline.clone(),
    });

    env.server.register_unary_unary(
        "TestService".to_string(),
        "CaptureDeadline".to_string(),
        handler,
    );

    env.start_server().await;

    let channel = env.create_client("deadline-propagation").await;

    let request = vec![1, 2, 3, 4];

    // Call with a specific timeout
    let timeout = Duration::from_secs(30);
    let start = std::time::SystemTime::now();

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "CaptureDeadline".to_string(),
            request,
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    // Check that the handler received a deadline
    let deadline_opt = captured_deadline.lock().await;
    assert!(deadline_opt.is_some(), "Handler should receive a deadline");

    let deadline = deadline_opt.unwrap();
    let expected_deadline = start + timeout;

    // The deadline should be approximately the expected value (within 1 second tolerance)
    let diff = if deadline > expected_deadline {
        deadline.duration_since(expected_deadline).unwrap()
    } else {
        expected_deadline.duration_since(deadline).unwrap()
    };

    assert!(
        diff < Duration::from_secs(1),
        "Deadline should match expected value within tolerance, diff: {:?}",
        diff
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_deadline_not_exceeded() {
    let env = TestEnv::new("context-not-exceeded").await;

    let captured_is_exceeded = Arc::new(Mutex::new(None));
    let handler = ContextValidationHandler {
        captured_session_id: Arc::new(Mutex::new(None)),
        captured_metadata: Arc::new(Mutex::new(None)),
        captured_deadline: Arc::new(Mutex::new(None)),
        captured_remaining: Arc::new(Mutex::new(None)),
        captured_is_exceeded: captured_is_exceeded.clone(),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-not-exceeded").await;

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(60)),
            None,
        )
        .await
        .expect("Call failed");

    // Verify deadline exceeded status
    let is_exceeded = captured_is_exceeded.lock().await;
    assert!(
        is_exceeded.is_some(),
        "Deadline exceeded status should be captured"
    );
    assert!(
        !is_exceeded.unwrap(),
        "Deadline should not be exceeded for normal calls"
    );

    env.server.shutdown_async().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_context_all_fields() {
    let env = TestEnv::new("context-all-fields").await;

    let captured_session_id = Arc::new(Mutex::new(None));
    let captured_metadata = Arc::new(Mutex::new(None));
    let captured_deadline = Arc::new(Mutex::new(None));
    let captured_remaining = Arc::new(Mutex::new(None));
    let captured_is_exceeded = Arc::new(Mutex::new(None));

    let handler = ContextValidationHandler {
        captured_session_id: captured_session_id.clone(),
        captured_metadata: captured_metadata.clone(),
        captured_deadline: captured_deadline.clone(),
        captured_remaining: captured_remaining.clone(),
        captured_is_exceeded: captured_is_exceeded.clone(),
    };

    env.server.register_unary_unary(
        "TestService".to_string(),
        "ValidateContext".to_string(),
        Arc::new(handler),
    );

    env.start_server().await;

    let channel = env.create_client("context-all-fields").await;

    let _response = channel
        .call_unary_async(
            "TestService".to_string(),
            "ValidateContext".to_string(),
            vec![],
            Some(Duration::from_secs(30)),
            None,
        )
        .await
        .expect("Call failed");

    // Verify all context fields were captured
    {
        let session_id = captured_session_id.lock().await;
        assert!(session_id.is_some(), "Session ID should be captured");
    }
    {
        let metadata = captured_metadata.lock().await;
        assert!(metadata.is_some(), "Metadata should be captured");
    }
    {
        let deadline = captured_deadline.lock().await;
        assert!(deadline.is_some(), "Deadline should be captured");
    }
    {
        let remaining = captured_remaining.lock().await;
        assert!(remaining.is_some(), "Remaining time should be captured");
    }
    {
        let is_exceeded = captured_is_exceeded.lock().await;
        assert!(
            is_exceeded.is_some(),
            "Deadline exceeded status should be captured"
        );
    }

    // Verify session ID is valid
    {
        let session_id = captured_session_id.lock().await;
        assert!(
            !session_id.as_ref().unwrap().is_empty(),
            "Session ID should not be empty"
        );
    }

    // Verify metadata contains deadline
    {
        let metadata = captured_metadata.lock().await;
        assert!(
            metadata.as_ref().unwrap().contains_key("slimrpc-timeout"),
            "Metadata should contain deadline"
        );
    }

    // Verify remaining time is reasonable
    {
        let remaining = captured_remaining.lock().await;
        assert!(
            remaining.unwrap() > Duration::ZERO,
            "Remaining time should be positive"
        );
    }

    // Verify deadline is not exceeded
    {
        let is_exceeded = captured_is_exceeded.lock().await;
        assert!(!is_exceeded.unwrap(), "Deadline should not be exceeded");
    }

    env.server.shutdown_async().await;
}

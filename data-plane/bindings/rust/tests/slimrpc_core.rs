// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC
//!
//! These tests verify the four RPC interaction patterns:
//! - Unary-Unary: Single request, single response
//! - Stream-Unary: Streaming requests, single response
//! - Unary-Stream: Single request, streaming responses
//! - Stream-Stream: Streaming requests, streaming responses

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::pin_mut;
use futures::stream::{self, StreamExt};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::id::{ID, Kind};
use slim_datapath::messages::Name;
use slim_service::service::Service;
use slim_testing::utils::TEST_VALID_SECRET;
use tokio::sync::Mutex;

use slim_bindings::slimrpc::{
    Channel, Code, Context, Decoder, Encoder, RequestStream, Server, Status,
};

// ============================================================================
// Test Message Types
// ============================================================================

/// Simple request message for testing
#[derive(Debug, Clone, Default, PartialEq, bincode::Encode, bincode::Decode)]
struct TestRequest {
    pub message: String,
    pub value: i32,
}

impl Encoder for TestRequest {
    fn encode(self) -> Result<Vec<u8>, Status> {
        let encoded = bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Status::internal(format!("Encoding error: {}", e)))?;
        Ok(encoded)
    }
}

impl Decoder for TestRequest {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, Status> {
        let (decoded, _len): (TestRequest, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| Status::invalid_argument(format!("Decoding error: {}", e)))?;
        Ok(decoded)
    }
}

/// Simple response message for testing
#[derive(Debug, Clone, Default, PartialEq, bincode::Encode, bincode::Decode)]
struct TestResponse {
    pub result: String,
    pub count: i32,
}

impl Encoder for TestResponse {
    fn encode(self) -> Result<Vec<u8>, Status> {
        let encoded = bincode::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| Status::internal(format!("Encoding error: {}", e)))?;
        Ok(encoded)
    }
}

impl Decoder for TestResponse {
    fn decode(buf: impl Into<Vec<u8>>) -> Result<Self, Status> {
        let (decoded, _len): (TestResponse, usize) =
            bincode::decode_from_slice(&buf.into(), bincode::config::standard())
                .map_err(|e| Status::invalid_argument(format!("Decoding error: {}", e)))?;
        Ok(decoded)
    }
}

// ============================================================================
// Test Helpers
// ============================================================================

/// Test environment containing service, server, and client components
struct TestEnv {
    service: Arc<Service>,
    server: Arc<Server>,
    channel: Channel,
}

impl TestEnv {
    /// Create a new test environment with server and client (server not started yet)
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
            server_name.clone(),
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
        let channel = Channel::new_internal(client_app.clone(), server_name.clone());

        Self {
            service,
            server,
            channel,
        }
    }

    /// Start the server in the background
    async fn start_server(&self) {
        let server = self.server.clone();

        // Spawn task to run the server
        tokio::spawn(async move {
            if let Err(e) = server.serve_async().await {
                tracing::error!("Server error: {:?}", e);
            }
        });

        // Give server time to start and subscribe
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// Clean shutdown of the test environment
    async fn shutdown(&mut self) {
        tracing::info!("Shutting down server...");
        self.server.shutdown_internal().await;

        tracing::info!("Shutting down service...");
        self.service.shutdown().await.unwrap();
    }
}

// ============================================================================
// Test Helper Functions
// ============================================================================

/// Collect all responses from a stream into a vector
async fn collect_stream_responses<T>(
    mut stream: impl futures::Stream<Item = Result<T, Status>> + Unpin,
) -> Vec<T> {
    let mut responses = Vec::new();
    while let Some(result) = stream.next().await {
        responses.push(result.expect("Stream item failed"));
    }
    responses
}

/// Collect responses from a stream until an error occurs
async fn collect_stream_until_error<T>(
    mut stream: impl futures::Stream<Item = Result<T, Status>> + Unpin,
) -> (Vec<T>, Option<Status>) {
    let mut responses = Vec::new();
    let mut error = None;
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => responses.push(response),
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    (responses, error)
}

/// Register a unary-unary handler that counts calls and tracks session IDs
fn register_counting_handler(
    server: &Arc<Server>,
    service: &str,
    method: &str,
    call_count: Arc<Mutex<i32>>,
    session_ids: Option<Arc<Mutex<Vec<String>>>>,
) {
    server.register_unary_unary_internal(
        service,
        method,
        move |request: TestRequest, ctx: Context| {
            let count = call_count.clone();
            let ids = session_ids.clone();
            async move {
                let mut c = count.lock().await;
                *c += 1;
                let current = *c;
                drop(c);

                if let Some(ids) = ids {
                    let session_id = ctx.session().session_id().to_string();
                    tracing::info!(
                        "RPC '{}' (call #{}) handled by session ID: {}",
                        request.message,
                        current,
                        session_id
                    );
                    let mut id_list = ids.lock().await;
                    id_list.push(session_id);
                    drop(id_list);
                }

                Ok(TestResponse {
                    result: format!("Echo: {}", request.message),
                    count: current,
                })
            }
        },
    );
}

/// Assert that a result is an error with the expected code and optional message substring
fn assert_error_with_code(
    result: Result<impl std::fmt::Debug, Status>,
    code: Code,
    msg_contains: Option<&str>,
) {
    assert!(result.is_err(), "Expected an error");
    let err = result.unwrap_err();
    assert_eq!(err.code(), code, "Error code mismatch");
    if let Some(expected_msg) = msg_contains {
        let actual_msg = err.message().unwrap_or("");
        assert!(
            actual_msg.contains(expected_msg),
            "Error message '{}' does not contain '{}'",
            actual_msg,
            expected_msg
        );
    }
}

// ============================================================================
// Test 1: Unary-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_rpc() {
    let mut env = TestEnv::new("test-service-unary").await;

    env.server.register_unary_unary_internal(
        "TestService",
        "Echo",
        |request: TestRequest, _ctx: Context| async move {
            Ok(TestResponse {
                result: format!("Echo: {}", request.message),
                count: request.value * 2,
            })
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "Hello".to_string(),
        value: 42,
    };

    let response: TestResponse = env
        .channel
        .unary("TestService", "Echo", request, None, None)
        .await
        .expect("Unary call failed");

    // Verify response
    assert_eq!(response.result, "Echo: Hello");
    assert_eq!(response.count, 84);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_error_handling() {
    let mut env = TestEnv::new("test-service-error").await;

    env.server.register_unary_unary_internal(
        "TestService",
        "ErrorMethod",
        |_request: TestRequest, _ctx: Context| async move {
            Err::<TestResponse, _>(Status::invalid_argument("Invalid input"))
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 1,
    };

    let result: Result<TestResponse, Status> = env
        .channel
        .unary("TestService", "ErrorMethod", request, None, None)
        .await;

    assert_error_with_code(result, Code::InvalidArgument, Some("Invalid input"));

    env.shutdown().await;
}

// ============================================================================
// Test 2: Stream-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_rpc() {
    let mut env = TestEnv::new("test-service-stream-unary").await;

    env.server.register_stream_unary_internal(
        "TestService",
        "Sum",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            let mut total = 0;
            let mut messages = Vec::new();

            while let Some(req_result) = request_stream.next().await {
                let req: TestRequest = req_result?;
                total += req.value;
                messages.push(req.message);
            }

            Ok(TestResponse {
                result: messages.join(", "),
                count: total,
            })
        },
    );

    env.start_server().await;

    let requests = vec![
        TestRequest {
            message: "one".to_string(),
            value: 1,
        },
        TestRequest {
            message: "two".to_string(),
            value: 2,
        },
        TestRequest {
            message: "three".to_string(),
            value: 3,
        },
    ];
    let request_stream = stream::iter(requests);

    let response: TestResponse = env
        .channel
        .stream_unary("TestService", "Sum", request_stream, None, None)
        .await
        .expect("Stream-unary call failed");

    // Verify response
    assert_eq!(response.result, "one, two, three");
    assert_eq!(response.count, 6);

    env.shutdown().await;
}

// ============================================================================
// Test 2b: Stream-Unary RPC with Error Handling
// ============================================================================
//
// This test verifies that errors in the request stream are properly handled.
// Note: RequestStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>
// Each item in the stream is a Result that can contain:
// - Ok(T): Successfully received and deserialized message
// - Err(Status): Network error, deserialization error, or transport issue
//
// In this test, we simulate an error condition by having the handler validate
// input and return an error when invalid data is encountered.

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_error_handling() {
    let mut env = TestEnv::new("test-service-stream-unary-error").await;

    env.server.register_stream_unary_internal(
        "TestService",
        "SumWithValidation",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            let mut total = 0;
            let mut messages = Vec::new();

            // Iterate over the stream of Results
            while let Some(req_result) = request_stream.next().await {
                // Each item is a Result<TestRequest, Status>
                // Use ? to propagate any errors from the stream (network, deserialization, etc.)
                let req = req_result?;

                // Validate input - return error if value is negative
                if req.value < 0 {
                    tracing::info!("Received invalid value: {}", req.value);
                    return Err(Status::invalid_argument(format!(
                        "Negative values not allowed: {}",
                        req.value
                    )));
                }

                total += req.value;
                messages.push(req.message);
            }

            Ok(TestResponse {
                result: messages.join(", "),
                count: total,
            })
        },
    );

    env.start_server().await;

    // Note: The client sends Stream<Item = TestRequest>, not Stream<Item = Result<...>>
    // The Result wrapper is added by the transport layer on the server side
    let requests = vec![
        TestRequest {
            message: "one".to_string(),
            value: 1,
        },
        TestRequest {
            message: "two".to_string(),
            value: 2,
        },
        TestRequest {
            message: "invalid".to_string(),
            value: -5, // This invalid value will cause the handler to return an error
        },
        TestRequest {
            message: "three".to_string(),
            value: 3, // This won't be processed due to the error above
        },
    ];
    let request_stream = stream::iter(requests);

    let response: Result<TestResponse, Status> = env
        .channel
        .stream_unary(
            "TestService",
            "SumWithValidation",
            request_stream,
            None,
            None,
        )
        .await;

    // Verify that the error was propagated back
    assert!(response.is_err(), "Expected an error response");
    let err = response.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    let msg = err.message().unwrap();
    assert!(
        msg.contains("Negative values not allowed"),
        "Error message was: {}",
        msg
    );
    assert!(msg.contains("-5"), "Error message was: {}", msg);

    env.shutdown().await;
}

// ============================================================================
// Test 3: Unary-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_rpc() {
    let mut env = TestEnv::new("test-service-unary-stream").await;

    env.server.register_unary_stream_internal(
        "TestService",
        "Generate",
        |request: TestRequest, _ctx: Context| async move {
            let count = request.value;
            let message = request.message.clone();

            // Create an async stream that generates responses incrementally

            // register current time
            let response_stream = async_stream::stream! {
                for i in 1..=count {
                    // Simulate some async work (optional)
                    tokio::time::sleep(Duration::from_millis(1)).await;

                    yield Ok(TestResponse {
                        result: format!("{}-{}", message, i),
                        count: i,
                    });
                }
            };

            Ok(response_stream)
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "item".to_string(),
        value: 5,
    };

    let responses: Vec<TestResponse> = {
        let response_stream = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "Generate",
            request,
            None,
            None,
        );
        pin_mut!(response_stream);

        collect_stream_responses(response_stream).await
    };

    // Verify responses
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0].result, "item-1");
    assert_eq!(responses[0].count, 1);
    assert_eq!(responses[4].result, "item-5");
    assert_eq!(responses[4].count, 5);

    env.shutdown().await;
}

// ============================================================================
// Test 3b: Unary-Stream RPC with Error Handling
// ============================================================================
//
// This test verifies that errors in the response stream are properly propagated.
// The handler generates several responses successfully, then encounters an error
// condition and yields an error in the stream. The client should receive the
// successful responses followed by the error.

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_error_handling() {
    let mut env = TestEnv::new("test-service-unary-stream-error").await;

    env.server.register_unary_stream_internal(
        "TestService",
        "GenerateWithError",
        |request: TestRequest, _ctx: Context| async move {
            let count = request.value;
            let message = request.message.clone();

            // Create an async stream that generates some responses then an error
            let response_stream = async_stream::stream! {
                for i in 1..=count {
                    // After 3 items, simulate an error condition
                    if i > 3 {
                        tracing::info!("Simulating error after {} responses", i - 1);
                        yield Err(Status::internal(
                            format!("Failed to generate item {}", i)
                        ));
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(1)).await;

                    yield Ok(TestResponse {
                        result: format!("{}-{}", message, i),
                        count: i,
                    });
                }
            };

            Ok(response_stream)
        },
    );

    env.start_server().await;

    // Request 10 items, but handler will error after 3
    let request = TestRequest {
        message: "item".to_string(),
        value: 10,
    };

    let (responses, error_received) = {
        let response_stream = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "GenerateWithError",
            request,
            None,
            None,
        );
        pin_mut!(response_stream);

        collect_stream_until_error(response_stream).await
    };

    // Verify we received 3 successful responses before the error
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0].result, "item-1");
    assert_eq!(responses[1].result, "item-2");
    assert_eq!(responses[2].result, "item-3");

    // Verify the error was received
    assert!(error_received.is_some(), "Expected an error in the stream");
    let err = error_received.unwrap();
    assert_eq!(err.code(), Code::Internal);
    assert!(err.message().unwrap().contains("Failed to generate item 4"));

    env.shutdown().await;
}

// ============================================================================
// Test 4: Stream-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_rpc() {
    let mut env = TestEnv::new("test-service-stream-stream").await;

    env.server.register_stream_stream_internal(
        "TestService",
        "Transform",
        |request_stream, _ctx: Context| async move {
            // Using .map() processes items as they arrive (lazy/incremental)
            // For more complex async processing, use async_stream or spawn a task
            // with a channel (see the SlimRPC examples for the channel pattern)
            Ok(request_stream.map(|req_result| {
                req_result.map(|req: TestRequest| TestResponse {
                    result: req.message.to_uppercase(),
                    count: req.value * 10,
                })
            }))
        },
    );

    env.start_server().await;

    // Create request stream
    let requests = vec![
        TestRequest {
            message: "hello".to_string(),
            value: 1,
        },
        TestRequest {
            message: "world".to_string(),
            value: 2,
        },
        TestRequest {
            message: "rpc".to_string(),
            value: 3,
        },
    ];

    let responses: Vec<TestResponse> = {
        let request_stream = stream::iter(requests);

        let response_stream = env.channel.stream_stream::<TestRequest, TestResponse>(
            "TestService",
            "Transform",
            request_stream,
            None,
            None,
        );
        pin_mut!(response_stream);

        collect_stream_responses(response_stream).await
    };

    // Verify responses
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0].result, "HELLO");
    assert_eq!(responses[0].count, 10);
    assert_eq!(responses[1].result, "WORLD");
    assert_eq!(responses[1].count, 20);
    assert_eq!(responses[2].result, "RPC");
    assert_eq!(responses[2].count, 30);

    env.shutdown().await;
}

// ============================================================================
// Test 4b: Stream-Stream RPC with Channel Pattern (Async Processing)
// ============================================================================
//
// This test demonstrates an alternative pattern for stream-stream handlers
// where complex async processing is needed. It uses a channel to decouple
// request processing from response generation, allowing true bidirectional
// streaming with async operations.

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_with_async_processing() {
    let mut env = TestEnv::new("test-service-stream-stream-async").await;

    env.server.register_stream_stream_internal(
        "TestService",
        "ProcessAsync",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            // Create channel for responses
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

            // Spawn task to process requests asynchronously
            tokio::spawn(async move {
                while let Some(req_result) = request_stream.next().await {
                    match req_result {
                        Ok(req) => {
                            // Simulate some async processing work
                            tokio::time::sleep(Duration::from_millis(5)).await;

                            let response = TestResponse {
                                result: format!("Processed: {}", req.message),
                                count: req.value * 100,
                            };

                            if tx.send(Ok(response)).is_err() {
                                tracing::warn!("Response channel closed");
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
            });

            // Return stream from the receiver
            Ok(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        },
    );

    env.start_server().await;

    let requests = vec![
        TestRequest {
            message: "alpha".to_string(),
            value: 1,
        },
        TestRequest {
            message: "beta".to_string(),
            value: 2,
        },
    ];

    let responses: Vec<TestResponse> = {
        let request_stream = stream::iter(requests);

        let response_stream = env.channel.stream_stream::<TestRequest, TestResponse>(
            "TestService",
            "ProcessAsync",
            request_stream,
            None,
            None,
        );
        pin_mut!(response_stream);

        collect_stream_responses(response_stream).await
    };

    // Verify responses
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].result, "Processed: alpha");
    assert_eq!(responses[0].count, 100);
    assert_eq!(responses[1].result, "Processed: beta");
    assert_eq!(responses[1].count, 200);

    env.shutdown().await;
}

// ============================================================================
// Additional Edge Case Tests
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_empty_stream_unary() {
    let mut env = TestEnv::new("test-service-empty-stream").await;

    env.server.register_stream_unary_internal(
        "TestService",
        "EmptySum",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            println!("Processing empty stream...");

            let mut count = 0;
            while (request_stream.next().await).is_some() {
                count += 1;
            }

            Ok(TestResponse {
                result: "empty".to_string(),
                count,
            })
        },
    );

    env.start_server().await;

    // Empty stream
    let request_stream = stream::iter(Vec::<TestRequest>::new());

    let response: TestResponse = env
        .channel
        .stream_unary("TestService", "EmptySum", request_stream, None, None)
        .await
        .expect("Empty stream-unary call failed");

    assert_eq!(response.result, "empty");
    assert_eq!(response.count, 0);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_concurrent_unary_calls() {
    let mut env = TestEnv::new("test-service-concurrent").await;

    let call_counter = Arc::new(Mutex::new(0));
    let counter_clone = call_counter.clone();

    env.server.register_unary_unary_internal(
        "TestService",
        "Count",
        move |request: TestRequest, _ctx: Context| {
            let counter = counter_clone.clone();
            async move {
                let mut count = counter.lock().await;
                *count += 1;
                let current = *count;
                drop(count);

                Ok(TestResponse {
                    result: request.message,
                    count: current,
                })
            }
        },
    );

    env.start_server().await;

    let channel = Arc::new(env.channel.clone());
    let mut handles = vec![];
    for i in 0..5 {
        let channel_clone = channel.clone();
        let handle = tokio::spawn(async move {
            let request = TestRequest {
                message: format!("call-{}", i),
                value: i,
            };
            channel_clone
                .unary::<TestRequest, TestResponse>("TestService", "Count", request, None, None)
                .await
        });
        handles.push(handle);
    }

    // Wait for all calls to complete
    let mut results = vec![];
    for handle in handles {
        let result = handle.await.unwrap().unwrap();
        results.push(result);
    }

    // All calls should succeed
    assert_eq!(results.len(), 5);

    // Counter should have been incremented 5 times
    let final_count = *call_counter.lock().await;
    assert_eq!(final_count, 5);

    env.shutdown().await;
}

// ============================================================================
// Session Caching Tests
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_one_session_per_rpc() {
    let mut env = TestEnv::new("test-one-session-per-rpc").await;

    let call_count = Arc::new(Mutex::new(0));
    let session_ids = Arc::new(Mutex::new(Vec::new()));

    register_counting_handler(
        &env.server,
        "TestService",
        "Echo",
        call_count.clone(),
        Some(session_ids.clone()),
    );

    env.start_server().await;

    // First call - creates session
    let request1 = TestRequest {
        message: "first".to_string(),
        value: 1,
    };
    let response1: TestResponse = env
        .channel
        .unary("TestService", "Echo", request1, None, None)
        .await
        .expect("First RPC call failed");
    assert_eq!(response1.count, 1);

    // Second call - creates new session (no session reuse)
    let request2 = TestRequest {
        message: "second".to_string(),
        value: 2,
    };
    let response2: TestResponse = env
        .channel
        .unary("TestService", "Echo", request2, None, None)
        .await
        .expect("Second RPC call failed");
    assert_eq!(response2.count, 2);

    // Get the session IDs from first two calls
    let id_list = session_ids.lock().await;
    assert_eq!(
        id_list.len(),
        2,
        "Should have 2 session IDs recorded so far"
    );
    let first_session_id = id_list[0].clone();
    let second_session_id = id_list[1].clone();
    drop(id_list);

    // Each call should use different session (one session per RPC)
    assert_ne!(
        first_session_id, second_session_id,
        "Each RPC call should use different session"
    );
    tracing::info!("✓ First call used session: {}", first_session_id);
    tracing::info!(
        "✓ Second call used different session: {}",
        second_session_id
    );

    // Third call - creates another new session
    let request3 = TestRequest {
        message: "third".to_string(),
        value: 3,
    };
    let response3: TestResponse = env
        .channel
        .unary("TestService", "Echo", request3, None, None)
        .await
        .expect("Third RPC call failed");
    assert_eq!(response3.count, 3);

    // Get all session IDs
    let id_list = session_ids.lock().await;
    assert_eq!(id_list.len(), 3, "Should have 3 session IDs recorded");
    let third_session_id = id_list[2].clone();
    drop(id_list);

    // Third call should use DIFFERENT session from both previous calls
    assert_ne!(
        third_session_id, first_session_id,
        "Third call should use different session"
    );
    assert_ne!(
        third_session_id, second_session_id,
        "Third call should use different session"
    );
    tracing::info!(
        "✓ Third call used yet another different session: {}",
        third_session_id
    );

    // All 3 calls should have been processed
    let final_count = *call_count.lock().await;
    assert_eq!(final_count, 3);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_session_per_rpc_even_after_error() {
    let mut env = TestEnv::new("test-session-per-rpc-after-error").await;

    let call_count = Arc::new(Mutex::new(0));
    let session_ids = Arc::new(Mutex::new(Vec::new()));

    let count_clone = call_count.clone();
    let ids_clone = session_ids.clone();

    env.server.register_unary_unary_internal(
        "TestService",
        "FlakyMethod",
        move |request: TestRequest, ctx: Context| {
            let count = count_clone.clone();
            let ids = ids_clone.clone();
            async move {
                let mut c = count.lock().await;
                *c += 1;
                let current = *c;
                drop(c);

                let session_id = ctx.session().session_id().to_string();
                tracing::info!(
                    "RPC '{}' (call #{}) handled by session ID: {}",
                    request.message,
                    current,
                    session_id
                );

                let mut id_list = ids.lock().await;
                id_list.push(session_id.clone());
                drop(id_list);

                // Fail on first call
                if current == 1 {
                    return Err(Status::internal("Simulated error"));
                }

                Ok(TestResponse {
                    result: request.message,
                    count: current,
                })
            }
        },
    );

    env.start_server().await;

    // First call fails
    let request1 = TestRequest {
        message: "first".to_string(),
        value: 1,
    };
    let result1 = env
        .channel
        .unary::<TestRequest, TestResponse>("TestService", "FlakyMethod", request1, None, None)
        .await;
    assert!(result1.is_err());
    assert_eq!(result1.unwrap_err().code(), Code::Internal);

    // Second call should succeed using a NEW session (each RPC gets its own session)
    let request2 = TestRequest {
        message: "second".to_string(),
        value: 2,
    };
    let response2: TestResponse = env
        .channel
        .unary("TestService", "FlakyMethod", request2, None, None)
        .await
        .expect("Second RPC call should succeed");
    assert_eq!(response2.count, 2);

    // Both calls should have been attempted
    let final_count = *call_count.lock().await;
    assert_eq!(final_count, 2);

    // Get all session IDs
    let id_list = session_ids.lock().await;
    assert_eq!(id_list.len(), 2, "Should have 2 session IDs recorded");
    let first_session_id = id_list[0].clone();
    let second_session_id = id_list[1].clone();
    drop(id_list);

    // Both calls should use DIFFERENT sessions (one session per RPC)
    assert_ne!(
        first_session_id, second_session_id,
        "Each RPC should get its own session, even after error"
    );
    tracing::info!(
        "✓ Each RPC got its own session: first={}, second={}",
        first_session_id,
        second_session_id
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_different_methods_different_sessions() {
    let mut env = TestEnv::new("test-different-sessions").await;

    let method1_count = Arc::new(Mutex::new(0));
    let method2_count = Arc::new(Mutex::new(0));

    let m1_clone = method1_count.clone();
    let m2_clone = method2_count.clone();

    env.server.register_unary_unary_internal(
        "TestService",
        "Method1",
        move |request: TestRequest, _ctx: Context| {
            let count = m1_clone.clone();
            async move {
                let mut c = count.lock().await;
                *c += 1;
                drop(c);

                Ok(TestResponse {
                    result: format!("M1: {}", request.message),
                    count: request.value + 100,
                })
            }
        },
    );

    env.server.register_unary_unary_internal(
        "TestService",
        "Method2",
        move |request: TestRequest, _ctx: Context| {
            let count = m2_clone.clone();
            async move {
                let mut c = count.lock().await;
                *c += 1;
                drop(c);

                Ok(TestResponse {
                    result: format!("M2: {}", request.message),
                    count: request.value + 200,
                })
            }
        },
    );

    env.start_server().await;

    // Call both methods multiple times
    for i in 0..3 {
        let req1 = TestRequest {
            message: format!("call-{}", i),
            value: i,
        };
        let req2 = TestRequest {
            message: format!("call-{}", i),
            value: i,
        };

        let resp1: TestResponse = env
            .channel
            .unary("TestService", "Method1", req1, None, None)
            .await
            .expect("Method1 call failed");

        let resp2: TestResponse = env
            .channel
            .unary("TestService", "Method2", req2, None, None)
            .await
            .expect("Method2 call failed");

        assert_eq!(resp1.result, format!("M1: call-{}", i));
        assert_eq!(resp1.count, i + 100);

        assert_eq!(resp2.result, format!("M2: call-{}", i));
        assert_eq!(resp2.count, i + 200);
    }

    // Each method should have been called 3 times
    assert_eq!(*method1_count.lock().await, 3);
    assert_eq!(*method2_count.lock().await, 3);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_separate_sessions_for_streaming() {
    let mut env = TestEnv::new("test-separate-sessions-streaming").await;

    let call_count = Arc::new(Mutex::new(0));
    let session_ids = Arc::new(Mutex::new(Vec::new()));

    let count_clone = call_count.clone();
    let ids_clone = session_ids.clone();

    env.server.register_unary_stream_internal(
        "TestService",
        "GenerateNumbers",
        move |request: TestRequest, ctx: Context| {
            let count = count_clone.clone();
            let ids = ids_clone.clone();
            async move {
                let mut c = count.lock().await;
                *c += 1;
                let current = *c;
                drop(c);

                let session_id = ctx.session().session_id().to_string();
                tracing::info!(
                    "Streaming RPC '{}' (call #{}) handled by session ID: {}",
                    request.message,
                    current,
                    session_id
                );

                let mut id_list = ids.lock().await;
                id_list.push(session_id.clone());
                drop(id_list);

                let n = request.value;
                let stream = stream::iter((0..n).map(|i| {
                    Ok(TestResponse {
                        result: format!("item-{}", i),
                        count: i,
                    })
                }));
                Ok(stream)
            }
        },
    );

    env.start_server().await;

    // First streaming call in a scope
    {
        let request1 = TestRequest {
            message: "first".to_string(),
            value: 3,
        };
        let stream1 = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "GenerateNumbers",
            request1,
            None,
            None,
        );
        pin_mut!(stream1);

        let results1 = collect_stream_responses(stream1).await;
        assert_eq!(results1.len(), 3);
    } // stream1 dropped here

    // Second streaming call in a scope - should create a 2nd session
    {
        let request2 = TestRequest {
            message: "second".to_string(),
            value: 2,
        };
        let stream2 = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "GenerateNumbers",
            request2,
            None,
            None,
        );
        pin_mut!(stream2);

        let results2 = collect_stream_responses(stream2).await;
        assert_eq!(results2.len(), 2);
    } // stream2 dropped here

    // Both calls should have been processed
    let final_count = *call_count.lock().await;
    assert_eq!(final_count, 2);

    // Get all session IDs
    let id_list = session_ids.lock().await;
    assert_eq!(id_list.len(), 2, "Should have 2 session IDs recorded");
    let first_session_id = id_list[0].clone();
    let second_session_id = id_list[1].clone();
    drop(id_list);

    // Both calls should use DIFFERENT sessions (one session per RPC)
    assert_ne!(
        first_session_id, second_session_id,
        "Each streaming RPC should get its own session"
    );
    tracing::info!(
        "✓ Each streaming RPC got its own session: first={}, second={}",
        first_session_id,
        second_session_id
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_concurrent_calls_independent() {
    let mut env = TestEnv::new("test-concurrent-calls").await;

    // Track concurrent execution (now multiple calls can run concurrently since no session lock)
    let active_count = Arc::new(Mutex::new(0));
    let max_concurrent = Arc::new(Mutex::new(0));

    let active_clone = active_count.clone();
    let max_clone = max_concurrent.clone();

    env.server.register_unary_unary_internal(
        "TestService",
        "SlowEcho",
        move |request: TestRequest, _ctx: Context| {
            let active = active_clone.clone();
            let max = max_clone.clone();
            async move {
                // Increment active count
                let mut a = active.lock().await;
                *a += 1;
                let current = *a;
                drop(a);

                // Update max
                let mut m = max.lock().await;
                if current > *m {
                    *m = current;
                }
                drop(m);

                // Simulate slow processing
                tokio::time::sleep(Duration::from_millis(100)).await;

                // Decrement active count
                let mut a = active.lock().await;
                *a -= 1;
                drop(a);

                Ok(TestResponse {
                    result: request.message,
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    // Launch concurrent calls - they should run concurrently since each RPC gets its own session
    let channel = Arc::new(env.channel.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut handles = vec![];
    for i in 0..3 {
        let ch = channel.clone();
        let b = barrier.clone();
        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            b.wait().await;

            let req = TestRequest {
                message: format!("call-{}", i),
                value: i + 1,
            };
            ch.unary::<TestRequest, TestResponse>("TestService", "SlowEcho", req, None, None)
                .await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap().expect("RPC should succeed");
    }

    // Max concurrent should be 3 (all calls run concurrently, no session lock)
    let max = *max_concurrent.lock().await;
    assert_eq!(
        max, 3,
        "Calls should run concurrently since each RPC gets its own session"
    );

    env.shutdown().await;
}

/// Test that multiple calls to the same server from the same client
/// towards different gRPC handler types all work correctly
#[tokio::test]
#[tracing_test::traced_test]
async fn test_multiple_handler_types_same_client() {
    let mut env = TestEnv::new("test-multi-handlers").await;

    // Counters to track calls to each handler
    let unary_count = Arc::new(Mutex::new(0));
    let stream_unary_count = Arc::new(Mutex::new(0));
    let unary_stream_count = Arc::new(Mutex::new(0));
    let stream_stream_count = Arc::new(Mutex::new(0));

    // Register unary-unary handler
    let uu_counter = unary_count.clone();
    env.server.register_unary_unary_internal(
        "MultiService",
        "UnaryUnary",
        move |request: TestRequest, _ctx: Context| {
            let counter = uu_counter.clone();
            async move {
                let mut c = counter.lock().await;
                *c += 1;
                drop(c);

                Ok(TestResponse {
                    result: format!("UnaryUnary: {}", request.message),
                    count: request.value * 10,
                })
            }
        },
    );

    // Register stream-unary handler
    let su_counter = stream_unary_count.clone();
    env.server.register_stream_unary_internal(
        "MultiService",
        "StreamUnary",
        move |mut stream: RequestStream<TestRequest>, _ctx: Context| {
            let counter = su_counter.clone();
            async move {
                let mut c = counter.lock().await;
                *c += 1;
                drop(c);

                let mut sum = 0;
                let mut messages = vec![];
                while let Some(result) = stream.next().await {
                    let req = result?;
                    sum += req.value;
                    messages.push(req.message);
                }

                Ok(TestResponse {
                    result: format!("StreamUnary: {}", messages.join(",")),
                    count: sum,
                })
            }
        },
    );

    // Register unary-stream handler
    let us_counter = unary_stream_count.clone();
    env.server.register_unary_stream_internal(
        "MultiService",
        "UnaryStream",
        move |request: TestRequest, _ctx: Context| {
            let counter = us_counter.clone();
            async move {
                let mut c = counter.lock().await;
                *c += 1;
                drop(c);

                let responses = (0..request.value).map(move |i| {
                    Ok(TestResponse {
                        result: format!("UnaryStream-{}: {}", i, request.message.clone()),
                        count: i,
                    })
                });

                Ok(stream::iter(responses))
            }
        },
    );

    // Register stream-stream handler
    let ss_counter = stream_stream_count.clone();
    env.server.register_stream_stream_internal(
        "MultiService",
        "StreamStream",
        move |mut stream: RequestStream<TestRequest>, _ctx: Context| {
            let counter = ss_counter.clone();
            async move {
                let mut c = counter.lock().await;
                *c += 1;
                drop(c);

                let response_stream = async_stream::stream! {
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(req) => {
                                yield Ok(TestResponse {
                                    result: format!("StreamStream: {}", req.message),
                                    count: req.value * 100,
                                });
                            }
                            Err(e) => {
                                yield Err(e);
                                break;
                            }
                        }
                    }
                };

                Ok(response_stream)
            }
        },
    );

    env.start_server().await;

    // Now test calling each handler multiple times from the same client

    // Test 1: Call unary-unary handler twice
    for i in 0..2 {
        let req = TestRequest {
            message: format!("uu-call-{}", i),
            value: i + 1,
        };
        let resp: TestResponse = env
            .channel
            .unary("MultiService", "UnaryUnary", req, None, None)
            .await
            .expect("UnaryUnary call failed");

        assert_eq!(resp.result, format!("UnaryUnary: uu-call-{}", i));
        assert_eq!(resp.count, (i + 1) * 10);
    }

    // Test 2: Call stream-unary handler twice
    for i in 0..2 {
        let requests = vec![
            TestRequest {
                message: format!("su-msg-{}-1", i),
                value: 1,
            },
            TestRequest {
                message: format!("su-msg-{}-2", i),
                value: 2,
            },
            TestRequest {
                message: format!("su-msg-{}-3", i),
                value: 3,
            },
        ];

        let resp: TestResponse = env
            .channel
            .stream_unary(
                "MultiService",
                "StreamUnary",
                stream::iter(requests),
                None,
                None,
            )
            .await
            .expect("StreamUnary call failed");

        assert_eq!(
            resp.result,
            format!("StreamUnary: su-msg-{}-1,su-msg-{}-2,su-msg-{}-3", i, i, i)
        );
        assert_eq!(resp.count, 6);
    }

    // Test 3: Call unary-stream handler twice
    for i in 0..2 {
        let req = TestRequest {
            message: format!("us-call-{}", i),
            value: 3,
        };

        let response_stream =
            env.channel
                .unary_stream("MultiService", "UnaryStream", req, None, None);

        pin_mut!(response_stream);

        let responses: Vec<TestResponse> = collect_stream_responses(response_stream).await;
        assert_eq!(responses.len(), 3);
        for (count, resp) in responses.iter().enumerate() {
            assert_eq!(resp.result, format!("UnaryStream-{}: us-call-{}", count, i));
            assert_eq!(resp.count, count as i32);
        }
    }

    // Test 4: Call stream-stream handler twice
    for i in 0..2 {
        let requests = vec![
            TestRequest {
                message: format!("ss-msg-{}-1", i),
                value: 1,
            },
            TestRequest {
                message: format!("ss-msg-{}-2", i),
                value: 2,
            },
        ];

        let response_stream = env.channel.stream_stream(
            "MultiService",
            "StreamStream",
            stream::iter(requests),
            None,
            None,
        );

        pin_mut!(response_stream);

        let received: Vec<TestResponse> = collect_stream_responses(response_stream).await;

        assert_eq!(received.len(), 2);
        assert_eq!(received[0].result, format!("StreamStream: ss-msg-{}-1", i));
        assert_eq!(received[0].count, 100);
        assert_eq!(received[1].result, format!("StreamStream: ss-msg-{}-2", i));
        assert_eq!(received[1].count, 200);
    }

    // Verify each handler was called the expected number of times
    assert_eq!(*unary_count.lock().await, 2);
    assert_eq!(*stream_unary_count.lock().await, 2);
    assert_eq!(*unary_stream_count.lock().await, 2);
    assert_eq!(*stream_stream_count.lock().await, 2);

    env.shutdown().await;
}

// ============================================================================
// Test: Client-side deadline enforcement (timeout)
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_client_deadline_unary_unary() {
    let mut env = TestEnv::new("test-client-deadline-unary").await;

    // Register a handler that takes longer than the timeout
    env.server.register_unary_unary_internal(
        "TestService",
        "SlowMethod",
        |request: TestRequest, _ctx: Context| async move {
            // Sleep for 2 seconds
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(TestResponse {
                result: format!("Processed: {}", request.message),
                count: request.value,
            })
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Call with a very short timeout (100ms)
    let result: Result<TestResponse, Status> = env
        .channel
        .unary(
            "TestService",
            "SlowMethod",
            request,
            Some(Duration::from_millis(100)),
            None,
        )
        .await;

    // Should timeout on the client side
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_client_deadline_unary_stream() {
    let mut env = TestEnv::new("test-client-deadline-unary-stream").await;

    // Register a handler that streams slowly
    env.server.register_unary_stream_internal(
        "TestService",
        "SlowStream",
        |request: TestRequest, _ctx: Context| async move {
            Ok(stream::iter((0..5).map(move |i| {
                let msg = request.message.clone();
                async move {
                    // Each item takes 500ms
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok::<_, Status>(TestResponse {
                        result: format!("{}-{}", msg, i),
                        count: i,
                    })
                }
            }))
            .then(|fut| fut))
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "item".to_string(),
        value: 0,
    };

    // Call with a timeout of 1 second (should only get 2 items before timeout)
    let (responses, last_error) = {
        let response_stream = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "SlowStream",
            request,
            Some(Duration::from_secs(1)),
            None,
        );

        pin_mut!(response_stream);

        collect_stream_until_error(response_stream).await
    };
    let count = responses.len();

    // Should have received some items but then timed out
    assert!(
        count < 5,
        "Expected timeout before all items, got {}",
        count
    );
    assert!(last_error.is_some());
    let err = last_error.unwrap();
    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

// ============================================================================
// Test: Server-side deadline enforcement
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_deadline_unary_unary() {
    let mut env = TestEnv::new("test-server-deadline-unary").await;

    // Register a handler that takes longer than the deadline
    env.server.register_unary_unary_internal(
        "TestService",
        "SlowHandler",
        |request: TestRequest, _ctx: Context| async move {
            // Handler takes 2 seconds
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(TestResponse {
                result: format!("Processed: {}", request.message),
                count: request.value,
            })
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Call with a short timeout (500ms) - server should enforce this
    let result: Result<TestResponse, Status> = env
        .channel
        .unary(
            "TestService",
            "SlowHandler",
            request,
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should timeout on the server side (handler execution)
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_deadline_unary_stream() {
    let mut env = TestEnv::new("test-server-deadline-unary-stream").await;

    // Register a handler that takes too long to start streaming
    env.server.register_unary_stream_internal(
        "TestService",
        "SlowStreamHandler",
        |request: TestRequest, _ctx: Context| async move {
            // Handler setup takes 2 seconds before returning stream
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(stream::iter((0..3).map(move |i| {
                Ok::<_, Status>(TestResponse {
                    result: format!("{}-{}", request.message, i),
                    count: i,
                })
            })))
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "item".to_string(),
        value: 0,
    };

    // Call with a short timeout (500ms)
    let err = {
        let response_stream = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "SlowStreamHandler",
            request,
            Some(Duration::from_millis(500)),
            None,
        );

        pin_mut!(response_stream);

        // Should get a deadline exceeded error
        let result = response_stream.next().await;
        assert!(result.is_some());
        let item_result = result.unwrap();
        assert!(item_result.is_err());
        item_result.unwrap_err()
    };

    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_deadline_stream_unary() {
    let mut env = TestEnv::new("test-server-deadline-stream-unary").await;

    // Register a handler that takes too long to process the stream
    env.server.register_stream_unary_internal(
        "TestService",
        "SlowStreamUnary",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            // Collect all requests
            let mut messages = Vec::new();
            while let Some(req_result) = request_stream.next().await {
                let req = req_result?;
                messages.push(req.message);
            }

            // Then take too long to process
            tokio::time::sleep(Duration::from_secs(2)).await;

            Ok(TestResponse {
                result: messages.join(","),
                count: messages.len() as i32,
            })
        },
    );

    env.start_server().await;

    let requests = vec![
        TestRequest {
            message: "msg1".to_string(),
            value: 1,
        },
        TestRequest {
            message: "msg2".to_string(),
            value: 2,
        },
    ];

    // Call with a short timeout (500ms)
    let result: Result<TestResponse, Status> = env
        .channel
        .stream_unary(
            "TestService",
            "SlowStreamUnary",
            stream::iter(requests),
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should timeout on the server side
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_deadline_stream_stream() {
    let mut env = TestEnv::new("test-server-deadline-stream-stream").await;

    // Register a handler that takes too long to setup
    env.server.register_stream_stream_internal(
        "TestService",
        "SlowStreamStream",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            // Consume one request
            if let Some(req_result) = request_stream.next().await {
                let _ = req_result?;
            }

            // Then take too long before returning stream
            tokio::time::sleep(Duration::from_secs(2)).await;

            Ok(stream::iter((0..3).map(|i| {
                Ok::<_, Status>(TestResponse {
                    result: format!("response-{}", i),
                    count: i,
                })
            })))
        },
    );

    env.start_server().await;

    let requests = vec![
        TestRequest {
            message: "msg1".to_string(),
            value: 1,
        },
        TestRequest {
            message: "msg2".to_string(),
            value: 2,
        },
    ];

    // Call with a short timeout (500ms)
    let err = {
        let response_stream = env.channel.stream_stream(
            "TestService",
            "SlowStreamStream",
            stream::iter(requests),
            Some(Duration::from_millis(500)),
            None,
        );

        pin_mut!(response_stream);

        // Should get a deadline exceeded error
        let result = response_stream.next().await;
        assert!(result.is_some());
        let item_result: Result<TestResponse, Status> = result.unwrap();
        assert!(item_result.is_err());
        item_result.unwrap_err()
    };

    assert_eq!(err.code(), Code::DeadlineExceeded);

    env.shutdown().await;
}

// ============================================================================
// Test: Server checks deadline before handler execution
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_deadline_already_exceeded() {
    let mut env = TestEnv::new("test-server-deadline-already-exceeded").await;

    let handler_called = Arc::new(Mutex::new(false));
    let handler_called_clone = handler_called.clone();

    // Register a handler
    env.server.register_unary_unary_internal(
        "TestService",
        "CheckDeadline",
        move |request: TestRequest, _ctx: Context| {
            let called = handler_called_clone.clone();
            async move {
                *called.lock().await = true;
                Ok(TestResponse {
                    result: format!("Processed: {}", request.message),
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Call with an already expired deadline (1ms and then wait)
    let channel_clone = env.channel.clone();
    let result: Result<TestResponse, Status> = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        channel_clone
            .unary(
                "TestService",
                "CheckDeadline",
                request,
                Some(Duration::from_millis(1)),
                None,
            )
            .await
    })
    .await
    .unwrap();

    // Should fail with deadline exceeded
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);

    // Handler should not have been called (deadline checked before execution)
    // Note: There's a race condition here, but with 50ms delay + network time,
    // the deadline should be exceeded before the handler is called
    tokio::time::sleep(Duration::from_millis(100)).await;
    // We can't reliably assert this due to timing, but the test verifies
    // that deadline exceeded is returned

    env.shutdown().await;
}

// ============================================================================
// Test: Deadline propagation from client to server
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_deadline_propagation() {
    let mut env = TestEnv::new("test-deadline-propagation").await;

    let deadline_from_handler: Arc<Mutex<Option<SystemTime>>> = Arc::new(Mutex::new(None));
    let deadline_clone = deadline_from_handler.clone();

    // Register a handler that captures the deadline from context
    env.server.register_unary_unary_internal(
        "TestService",
        "CaptureDeadline",
        move |request: TestRequest, ctx: Context| {
            let deadline = deadline_clone.clone();
            async move {
                *deadline.lock().await = Some(ctx.deadline());
                Ok(TestResponse {
                    result: format!("Processed: {}", request.message),
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Call with a specific timeout
    let timeout = Duration::from_secs(30);
    let start = std::time::SystemTime::now();

    let _: TestResponse = env
        .channel
        .unary(
            "TestService",
            "CaptureDeadline",
            request,
            Some(timeout),
            None,
        )
        .await
        .expect("Call failed");

    // Check that the handler received a deadline
    let captured_deadline = deadline_from_handler.lock().await;
    assert!(
        captured_deadline.is_some(),
        "Handler should receive a deadline"
    );

    let deadline = captured_deadline.unwrap();
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

    env.shutdown().await;
}

// ============================================================================
// Test: Server-side deadline enforcement (infrastructure checks, not handler)
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_rejects_already_expired_deadline() {
    let mut env = TestEnv::new("test-expired-deadline").await;

    let handler_called = Arc::new(Mutex::new(false));
    let handler_called_clone = handler_called.clone();

    // Register a handler that should never be called
    env.server.register_unary_unary_internal(
        "TestService",
        "ShouldNotBeCalled",
        move |request: TestRequest, _ctx: Context| {
            let called = handler_called_clone.clone();
            async move {
                tracing::error!("Handler was called when it should have been rejected!");
                *called.lock().await = true;

                Ok(TestResponse {
                    result: format!("Processed: {}", request.message),
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Use a very short deadline (1ms) and wait before making the call
    // to ensure the deadline is already expired when the server receives it
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result: Result<TestResponse, Status> = env
        .channel
        .unary(
            "TestService",
            "ShouldNotBeCalled",
            request,
            Some(Duration::from_nanos(1)),
            None,
        )
        .await;

    // Should fail with deadline exceeded or timing-related error
    assert!(result.is_err(), "Expected deadline exceeded error");
    let err = result.unwrap_err();

    // The error might be DeadlineExceeded or Internal depending on timing
    // (if the deadline check happens before or after session setup)
    assert!(
        err.code() == Code::DeadlineExceeded || err.code() == Code::Internal,
        "Expected DeadlineExceeded or Internal, got {:?}",
        err.code()
    );

    // Give some time for any potential handler execution to occur
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the handler was NOT called (server rejected the request before handler execution)
    let was_called = *handler_called.lock().await;
    assert!(
        !was_called,
        "Handler should not have been called for expired deadline"
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_enforces_deadline_during_handler_execution() {
    let mut env = TestEnv::new("test-server-enforces-deadline").await;

    let handler_started = Arc::new(Mutex::new(false));
    let handler_completed = Arc::new(Mutex::new(false));
    let started_clone = handler_started.clone();
    let completed_clone = handler_completed.clone();

    // Register a handler that takes a long time (ignores deadline)
    env.server.register_unary_unary_internal(
        "TestService",
        "LongRunningIgnoresDeadline",
        move |request: TestRequest, _ctx: Context| {
            let started = started_clone.clone();
            let completed = completed_clone.clone();
            async move {
                *started.lock().await = true;
                tracing::info!("Handler started, will run for 5 seconds");

                // Handler intentionally ignores the deadline and runs for a long time
                tokio::time::sleep(Duration::from_secs(5)).await;

                *completed.lock().await = true;
                tracing::info!("Handler completed (this should not happen due to deadline)");

                Ok(TestResponse {
                    result: format!("Processed: {}", request.message),
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "test".to_string(),
        value: 42,
    };

    // Set a short deadline (500ms) while handler takes 5 seconds
    let result: Result<TestResponse, Status> = env
        .channel
        .unary(
            "TestService",
            "LongRunningIgnoresDeadline",
            request,
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should fail with deadline exceeded (enforced by server, not handler)
    assert!(result.is_err(), "Expected deadline exceeded error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        Code::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete (but it shouldn't)
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the handler started but did not complete
    // (server infrastructure cancelled it due to deadline)
    let was_started = *handler_started.lock().await;
    let was_completed = *handler_completed.lock().await;

    assert!(was_started, "Handler should have started execution");
    assert!(!was_completed, "Handler should not have completed");

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_enforces_deadline_for_stream_unary() {
    let mut env = TestEnv::new("test-server-deadline-stream-unary").await;

    let messages_received = Arc::new(Mutex::new(0));
    let handler_completed = Arc::new(Mutex::new(false));
    let message_received_clone = messages_received.clone();
    let handler_completed_clone = handler_completed.clone();

    // Register a stream-unary handler that processes messages slowly
    env.server.register_stream_unary_internal(
        "TestService",
        "SlowStreamProcessor",
        move |mut request_stream: RequestStream<TestRequest>, _ctx: Context| {
            let received = message_received_clone.clone();
            let completed = handler_completed_clone.clone();
            async move {
                // Process incoming messages slowly (ignores deadline)
                while let Some(req_result) = request_stream.next().await {
                    let req = req_result?;
                    *received.lock().await += 1;

                    tracing::info!(
                        "Processing message {}: {}",
                        *received.lock().await,
                        req.message
                    );

                    // Slow processing (200ms per message)
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                *completed.lock().await = true;

                let count = *received.lock().await;

                tracing::info!("Handler completed processing {} messages", count);

                Ok(TestResponse {
                    result: format!("Processed {} messages", count),
                    count,
                })
            }
        },
    );

    env.start_server().await;

    // Create a stream of 10 messages (would take 2 seconds to process)
    let requests = (0..10)
        .map(|i| TestRequest {
            message: format!("msg-{}", i),
            value: i,
        })
        .collect::<Vec<_>>();

    // Set a deadline of 500ms (server should enforce this)
    let result: Result<TestResponse, Status> = env
        .channel
        .stream_unary(
            "TestService",
            "SlowStreamProcessor",
            stream::iter(requests),
            Some(Duration::from_millis(500)),
            None,
        )
        .await;

    // Should fail with deadline exceeded (enforced by server infrastructure)
    assert!(result.is_err(), "Expected deadline exceeded error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        Code::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    let was_completed = *handler_completed.lock().await;
    let received = *messages_received.lock().await;

    assert!(!was_completed, "Server should not have completed");

    // Handler should not have completed due to server-enforced deadline
    assert!(
        received < 10,
        "Handler should not have processed all messages due to deadline, got {}",
        received
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_enforces_deadline_for_unary_stream() {
    let mut env = TestEnv::new("test-server-deadline-unary-stream").await;

    let messages_sent = Arc::new(Mutex::new(0));
    let handler_completed = Arc::new(Mutex::new(false));
    let sent_clone = messages_sent.clone();
    let completed_clone = handler_completed.clone();

    // Register a unary-stream handler that generates messages slowly
    env.server.register_unary_stream_internal(
        "TestService",
        "SlowStreamGenerator",
        move |request: TestRequest, _ctx: Context| {
            let sent = sent_clone.clone();
            let completed = completed_clone.clone();
            async move {
                let response_stream = async_stream::stream! {
                    // Try to generate many messages slowly (ignores deadline)
                    for i in 0..10 {
                        tracing::info!("Generating message {}", i);

                        // Slow generation (200ms per message)
                        tokio::time::sleep(Duration::from_millis(200)).await;

                        let mut count = sent.lock().await;
                        *count += 1;
                        drop(count);

                        yield Ok(TestResponse {
                            result: format!("{}-{}", request.message, i),
                            count: i,
                        });
                    }

                    *completed.lock().await = true;
                    tracing::info!("Handler completed generating all messages");
                };

                Ok(response_stream)
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "item".to_string(),
        value: 0,
    };

    // Set a deadline of 500ms (would take 2 seconds to generate all 10 messages)
    let (responses, error) = {
        let response_stream = env.channel.unary_stream::<TestRequest, TestResponse>(
            "TestService",
            "SlowStreamGenerator",
            request,
            Some(Duration::from_millis(500)),
            None,
        );

        pin_mut!(response_stream);

        collect_stream_until_error(response_stream).await
    };
    let responses_received = responses.len();

    // Should have received some messages but not all due to deadline
    assert!(
        responses_received < 10,
        "Should not have received all messages due to deadline, got {}",
        responses_received
    );

    // Should get a deadline exceeded error
    assert!(error.is_some(), "Expected deadline exceeded error");
    let err = error.unwrap();
    assert_eq!(
        err.code(),
        Code::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    let was_completed = *handler_completed.lock().await;
    let sent = *messages_sent.lock().await;

    assert!(!was_completed, "Handler should not have completed");
    assert!(
        sent < 10,
        "Handler should not have sent all messages due to deadline, sent {}",
        sent
    );

    env.shutdown().await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_server_enforces_deadline_for_stream_stream() {
    let mut env = TestEnv::new("test-server-deadline-stream-stream").await;

    let messages_received = Arc::new(Mutex::new(0));
    let messages_sent = Arc::new(Mutex::new(0));
    let handler_completed = Arc::new(Mutex::new(false));
    let received_clone = messages_received.clone();
    let sent_clone = messages_sent.clone();
    let completed_clone = handler_completed.clone();

    // Register a stream-stream handler that processes slowly
    env.server.register_stream_stream_internal(
        "TestService",
        "SlowStreamTransform",
        move |mut request_stream: RequestStream<TestRequest>, _ctx: Context| {
            let received = received_clone.clone();
            let sent = sent_clone.clone();
            let completed = completed_clone.clone();
            async move {
                let response_stream = async_stream::stream! {
                    // Process incoming messages slowly (ignores deadline)
                    while let Some(req_result) = request_stream.next().await {
                        match req_result {
                            Ok(req) => {
                                let mut recv_count = received.lock().await;
                                *recv_count += 1;
                                drop(recv_count);

                                tracing::info!("Processing message: {}", req.message);

                                // Slow processing (300ms per message)
                                tokio::time::sleep(Duration::from_millis(300)).await;

                                let mut send_count = sent.lock().await;
                                *send_count += 1;
                                drop(send_count);

                                yield Ok(TestResponse {
                                    result: format!("Processed: {}", req.message),
                                    count: req.value * 10,
                                });
                            }
                            Err(e) => {
                                yield Err(e);
                                break;
                            }
                        }
                    }

                    *completed.lock().await = true;
                    tracing::info!("Handler completed processing all messages");
                };

                Ok(response_stream)
            }
        },
    );

    env.start_server().await;

    // Create a stream of 10 messages (would take 3 seconds to process)
    let requests = (0..10)
        .map(|i| TestRequest {
            message: format!("msg-{}", i),
            value: i,
        })
        .collect::<Vec<_>>();

    // Set a deadline of 800ms (server should enforce this)
    let (responses, error) = {
        let response_stream = env.channel.stream_stream::<TestRequest, TestResponse>(
            "TestService",
            "SlowStreamTransform",
            stream::iter(requests),
            Some(Duration::from_millis(800)),
            None,
        );

        pin_mut!(response_stream);

        collect_stream_until_error(response_stream).await
    };
    let responses_received = responses.len();

    // Should have received some messages but not all due to deadline
    assert!(
        responses_received < 10,
        "Should not have received all messages due to deadline, got {}",
        responses_received
    );

    // Should get a deadline exceeded error
    assert!(error.is_some(), "Expected deadline exceeded error");
    let err = error.unwrap();
    assert_eq!(
        err.code(),
        Code::DeadlineExceeded,
        "Expected DeadlineExceeded, got {:?}",
        err.code()
    );

    // Give handler time to potentially complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    let was_completed = *handler_completed.lock().await;
    let sent = *messages_sent.lock().await;

    assert!(!was_completed, "Handler should not have completed");
    assert!(
        sent < 10,
        "Handler should not have sent all messages due to deadline, sent {}",
        sent
    );

    env.shutdown().await;
}

/// Test that verifies a server can be shut down and restarted successfully
#[tokio::test]
async fn test_server_restart() {
    let mut env = TestEnv::new("test-server-restart").await;

    // Register a handler that counts calls
    let call_count = Arc::new(Mutex::new(0u32));
    let count_clone = call_count.clone();

    env.server.register_unary_unary_internal(
        "TestService",
        "Counter",
        move |request: TestRequest, _ctx: Context| {
            let count = count_clone.clone();
            async move {
                let mut counter = count.lock().await;
                *counter += 1;
                let current_count = *counter;
                drop(counter);

                Ok(TestResponse {
                    result: format!(
                        "Call count: {}, Message: {}",
                        current_count, request.message
                    ),
                    count: current_count as i32,
                })
            }
        },
    );

    // Start server first time
    env.start_server().await;

    // Make first call
    let request1 = TestRequest {
        message: "First call".to_string(),
        value: 1,
    };

    let response1: TestResponse = env
        .channel
        .unary("TestService", "Counter", request1, None, None)
        .await
        .expect("First unary call failed");

    assert_eq!(response1.count, 1);
    assert!(response1.result.contains("Call count: 1"));
    assert!(response1.result.contains("First call"));

    // Make second call before shutdown
    let request2 = TestRequest {
        message: "Second call".to_string(),
        value: 2,
    };

    let response2: TestResponse = env
        .channel
        .unary("TestService", "Counter", request2, None, None)
        .await
        .expect("Second unary call failed");

    assert_eq!(response2.count, 2);
    assert!(response2.result.contains("Call count: 2"));

    // Shutdown the server
    tracing::info!("Shutting down server for restart test...");
    env.server.shutdown_async().await;

    // Give a brief moment for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Restart the server
    tracing::info!("Restarting server...");
    env.start_server().await;

    // Make third call after restart
    let request3 = TestRequest {
        message: "Third call after restart".to_string(),
        value: 3,
    };

    let response3: TestResponse = env
        .channel
        .unary("TestService", "Counter", request3, None, None)
        .await
        .expect("Third unary call after restart failed");

    // The counter should continue from where it left off (state is preserved)
    assert_eq!(response3.count, 3);
    assert!(response3.result.contains("Call count: 3"));
    assert!(response3.result.contains("Third call after restart"));

    // Make fourth call to ensure server is fully functional
    let request4 = TestRequest {
        message: "Fourth call".to_string(),
        value: 4,
    };

    let response4: TestResponse = env
        .channel
        .unary("TestService", "Counter", request4, None, None)
        .await
        .expect("Fourth unary call failed");

    assert_eq!(response4.count, 4);
    assert!(response4.result.contains("Call count: 4"));

    // Final shutdown
    env.shutdown().await;
}

/// Test that verifies shutting down the server while a handler is executing
/// The handler is cancelled and the client receives an error
#[tokio::test]
async fn test_server_shutdown_during_handler_execution() {
    let mut env = TestEnv::new("test-shutdown-during-handler").await;

    let handler_started = Arc::new(Mutex::new(false));
    let started_clone = handler_started.clone();

    // Register a handler that takes some time to execute
    env.server.register_unary_unary_internal(
        "TestService",
        "SlowHandler",
        move |request: TestRequest, _ctx: Context| {
            let started = started_clone.clone();
            async move {
                *started.lock().await = true;

                // Simulate a long-running handler (5 seconds)
                tokio::time::sleep(Duration::from_secs(5)).await;

                Ok(TestResponse {
                    result: format!("Processed: {}", request.message),
                    count: request.value,
                })
            }
        },
    );

    env.start_server().await;

    let request = TestRequest {
        message: "Test".to_string(),
        value: 42,
    };

    // Start the RPC call in a separate task
    let channel = env.channel.clone();
    let call_handle = tokio::spawn(async move {
        channel
            .unary::<TestRequest, TestResponse>("TestService", "SlowHandler", request, None, None)
            .await
    });

    // Wait for handler to start
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if *handler_started.lock().await {
            break;
        }
    }

    assert!(*handler_started.lock().await, "Handler should have started");

    // Give the handler more time to be deep in execution
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now shut down the server while the handler is still executing
    env.server.shutdown_async().await;

    // The RPC call should return an error since the handler was cancelled
    let result = call_handle.await.expect("Task should not panic");

    assert!(
        result.is_err(),
        "Expected an error when server shuts down during handler execution"
    );

    let error = result.unwrap_err();

    // The error should be Cancelled since the handler was cancelled during shutdown
    assert_eq!(
        error.code(),
        Code::Cancelled,
        "Expected Cancelled error when server shuts down during handler execution, got: {:?}",
        error.code()
    );

    env.shutdown().await;
}

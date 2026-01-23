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
use std::time::Duration;

use futures::pin_mut;
use futures::stream::{self, StreamExt};
use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::id::{ID, Kind};
use slim_datapath::messages::Name;
use slim_service::service::Service;
use slim_testing::utils::TEST_VALID_SECRET;
use tokio::sync::Mutex;


use agntcy_slimrpc::{Channel, Code, Context, RequestStream, Server, Status};
use slim_testing::slimrpc::{TestRequest, TestResponse};

// ============================================================================
// Test 1: Unary-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_rpc() {
    // Create a single service that both apps will use
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-unary").unwrap();
    let service = Arc::new(Service::new(id));

    // Create server app
    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    // Create and configure server
    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register unary-unary handler
    server.registry().register_unary_unary(
        "TestService",
        "Echo",
        |request: TestRequest, _ctx: Context| async move {
            Ok(TestResponse {
                result: format!("Echo: {}", request.message),
                count: request.value * 2,
            })
        },
    );

    // Start server in background
    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create client app using the same service
    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _client_notifications) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);

    // Create channel
    let channel = Channel::new(client_app.clone(), server_name);

    // Make unary call
    let request = TestRequest {
        message: "Hello".to_string(),
        value: 42,
    };

    let response: TestResponse = channel
        .unary("TestService", "Echo", request, None, None)
        .await
        .expect("Unary call failed");

    // Verify response
    assert_eq!(response.result, "Echo: Hello");
    assert_eq!(response.count, 84);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finiss
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_unary_error_handling() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-error").unwrap();
    let service = Service::new(id);

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register handler that returns an error
    server.registry().register_unary_unary(
        "TestService",
        "ErrorMethod",
        |_request: TestRequest, _ctx: Context| async move {
            Err::<TestResponse, _>(Status::invalid_argument("Invalid input"))
        },
    );

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    let request = TestRequest {
        message: "test".to_string(),
        value: 1,
    };

    let result: Result<TestResponse, Status> = channel
        .unary("TestService", "ErrorMethod", request, None, None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    println!("{}", err);
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(err.message(), Some("Invalid input"));

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

// ============================================================================
// Test 2: Stream-Unary RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_unary_rpc() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-stream-unary").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register stream-unary handler that sums all values
    server.registry().register_stream_unary(
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    // Create request stream
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

    let response: TestResponse = channel
        .stream_unary("TestService", "Sum", request_stream, None, None)
        .await
        .expect("Stream-unary call failed");

    // Verify response
    assert_eq!(response.result, "one, two, three");
    assert_eq!(response.count, 6);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
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
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-stream-unary-error").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register stream-unary handler that validates input and returns error on invalid data
    // The handler demonstrates proper error handling for RequestStream<T> where each
    // item is a Result<T, Status>
    server.registry().register_stream_unary(
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
                    return Err(Status::invalid_argument(
                        format!("Negative values not allowed: {}", req.value)
                    ));
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    // Create request stream with an invalid value (negative) that should trigger an error
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

    let response: Result<TestResponse, Status> = channel
        .stream_unary("TestService", "SumWithValidation", request_stream, None, None)
        .await;

    // Verify that the error was propagated back
    assert!(response.is_err(), "Expected an error response");
    let err = response.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    let msg = err.message().unwrap();
    assert!(msg.contains("Negative values not allowed"), "Error message was: {}", msg);
    assert!(msg.contains("-5"), "Error message was: {}", msg);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

// ============================================================================
// Test 3: Unary-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_unary_stream_rpc() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-unary-stream").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register unary-stream handler that generates a sequence
    // This demonstrates proper streaming - responses are generated incrementally
    // rather than collected upfront
    server.registry().register_unary_stream(
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    let request = TestRequest {
        message: "item".to_string(),
        value: 5,
    };

    let response_stream = channel.unary_stream("TestService", "Generate", request, None, None);
    pin_mut!(response_stream);

    // Collect all responses
    let mut responses = Vec::new();
    while let Some(result) = response_stream.next().await {
        let response: TestResponse = result.expect("Stream item failed");
        tracing::info!("Received response: {:?}", response);
        responses.push(response);
    }

    // Verify responses
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0].result, "item-1");
    assert_eq!(responses[0].count, 1);
    assert_eq!(responses[4].result, "item-5");
    assert_eq!(responses[4].count, 5);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
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
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-unary-stream-error").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register unary-stream handler that generates responses and then an error
    // This demonstrates error handling in response streams
    server.registry().register_unary_stream(
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    // Request 10 items, but handler will error after 3
    let request = TestRequest {
        message: "item".to_string(),
        value: 10,
    };

    let response_stream = channel.unary_stream("TestService", "GenerateWithError", request, None, None);
    pin_mut!(response_stream);

    // Collect responses until we hit the error
    let mut responses: Vec<TestResponse> = Vec::new();
    let mut error_received = None;

    while let Some(result) = response_stream.next().await {
        match result {
            Ok(response) => {
                responses.push(response);
            }
            Err(status) => {
                error_received = Some(status);
                break;
            }
        }
    }

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

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

// ============================================================================
// Test 4: Stream-Stream RPC
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_stream_stream_rpc() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-stream-stream").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register stream-stream handler that transforms each request
    // This demonstrates proper bidirectional streaming - requests are processed
    // incrementally as they arrive, and responses are sent immediately without
    // waiting for the entire input stream to complete
    server.registry().register_stream_stream(
        "TestService",
        "Transform",
        |request_stream, _ctx: Context| async move {
            // Using .map() processes items as they arrive (lazy/incremental)
            // For more complex async processing, use async_stream or spawn a task
            // with a channel (see the SlimRPC examples for the channel pattern)
            Ok(request_stream.map(|req_result| {
                req_result.and_then(|req: TestRequest| {
                    Ok(TestResponse {
                        result: req.message.to_uppercase(),
                        count: req.value * 10,
                    })
                })
            }))
        },
    );

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

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
    let request_stream = stream::iter(requests);

    let response_stream =
        channel.stream_stream("TestService", "Transform", request_stream, None, None);
    pin_mut!(response_stream);

    // Collect all responses
    let mut responses = Vec::new();
    while let Some(result) = response_stream.next().await {
        let response: TestResponse = result.expect("Stream item failed");
        responses.push(response);
    }

    // Verify responses
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0].result, "HELLO");
    assert_eq!(responses[0].count, 10);
    assert_eq!(responses[1].result, "WORLD");
    assert_eq!(responses[1].count, 20);
    assert_eq!(responses[2].result, "RPC");
    assert_eq!(responses[2].count, 30);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
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
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-stream-stream-async").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    // Register stream-stream handler using channel pattern for async processing
    // This pattern is useful when you need to do complex async work per request
    server.registry().register_stream_stream(
        "TestService",
        "ProcessAsync",
        |mut request_stream: agntcy_slimrpc::RequestStream<TestRequest>, _ctx: Context| async move {
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    // Create request stream
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
    let request_stream = stream::iter(requests);

    let response_stream =
        channel.stream_stream("TestService", "ProcessAsync", request_stream, None, None);
    pin_mut!(response_stream);

    // Collect all responses
    let mut responses = Vec::new();
    while let Some(result) = response_stream.next().await {
        let response: TestResponse = result.expect("Stream item failed");
        responses.push(response);
    }

    // Verify responses
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].result, "Processed: alpha");
    assert_eq!(responses[0].count, 100);
    assert_eq!(responses[1].result, "Processed: beta");
    assert_eq!(responses[1].count, 200);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

// ============================================================================
// Additional Edge Case Tests
// ============================================================================

#[tokio::test]
#[tracing_test::traced_test]
async fn test_empty_stream_unary() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-empty-stream").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    server.registry().register_stream_unary(
        "TestService",
        "EmptySum",
        |mut request_stream: RequestStream<TestRequest>, _ctx: Context| async move {
            let mut count = 0;
            while let Some(_) = request_stream.next().await {
                count += 1;
            }

            Ok(TestResponse {
                result: "empty".to_string(),
                count,
            })
        },
    );

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Channel::new(client_app.clone(), server_name);

    // Empty stream
    let request_stream = stream::iter(Vec::<TestRequest>::new());

    let response: TestResponse = channel
        .stream_unary("TestService", "EmptySum", request_stream, None, None)
        .await
        .expect("Empty stream-unary call failed");

    assert_eq!(response.result, "empty");
    assert_eq!(response.count, 0);

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[tracing_test::traced_test]
async fn test_concurrent_unary_calls() {
    let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-concurrent").unwrap();
    let service = Arc::new(Service::new(id));

    let server_name = Name::from_strings(["org", "ns", "server"]);
    let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
    let (server_app, server_notifications) = service
        .create_app(
            &server_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret.clone()),
        )
        .unwrap();
    let server_app = Arc::new(server_app);

    let server = Server::new(
        server_app.clone(),
        server_name.clone(),
        server_notifications,
    );

    let call_counter = Arc::new(Mutex::new(0));
    let counter_clone = call_counter.clone();

    server.registry().register_unary_unary(
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

    let server_clone = server.clone();
    let server_handle = tokio::spawn(async move {
        let _ = server_clone.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client_name = Name::from_strings(["org", "ns", "client"]);
    let (client_app, _) = service
        .create_app(
            &client_name,
            AuthProvider::shared_secret(secret.clone()),
            AuthVerifier::shared_secret(secret),
        )
        .unwrap();
    let client_app = Arc::new(client_app);
    let channel = Arc::new(Channel::new(client_app.clone(), server_name));

    // Make multiple concurrent calls
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

    // Cleanup
    tracing::info!("Shutting down server...");
    server.shutdown().await;

    // Wait for server task to finish
    tracing::info!("Waiting for server task to finish...");
    server_handle.await.unwrap();

    // Shutdown the data-plane as well
    tracing::info!("Shutting down service...");
    service.shutdown().await.unwrap();
}

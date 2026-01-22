// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for SlimRPC
//!
//! These tests verify the four RPC interaction patterns:
//! - Unary-Unary: Single request, single response
//! - Stream-Unary: Streaming requests, single response
//! - Unary-Stream: Single request, streaming responses
//! - Stream-Stream: Streaming requests, streaming responses

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::stream::{self, StreamExt};
    use futures::pin_mut;
    use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::messages::Name;
    use slim_service::service::Service;
    use slim_config::component::id::{ID, Kind};
    use slim_testing::utils::TEST_VALID_SECRET;
    use tokio::sync::Mutex;

    use crate::{
        Channel, Code, Context, Decoder, Encoder, Server, Status,
    };

    // ============================================================================
    // Test Message Types
    // ============================================================================

    /// Simple request message for testing
    #[derive(Debug, Clone, Default, PartialEq)]
    struct TestRequest {
        pub message: String,
        pub value: i32,
    }

    impl Encoder for TestRequest {
        fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status> {
            // Simple encoding: message length + message bytes + value as 4 bytes
            let msg_bytes = self.message.as_bytes();
            buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(msg_bytes);
            buf.extend_from_slice(&self.value.to_le_bytes());
            Ok(())
        }

        fn encoded_len(&self) -> usize {
            4 + self.message.len() + 4
        }
    }

    impl Decoder for TestRequest {
        fn decode(buf: &[u8]) -> Result<Self, Status> {
            if buf.len() < 8 {
                return Err(Status::invalid_argument("Buffer too small"));
            }

            let msg_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if buf.len() < 4 + msg_len + 4 {
                return Err(Status::invalid_argument("Invalid buffer length"));
            }

            let message = String::from_utf8(buf[4..4 + msg_len].to_vec())
                .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8: {}", e)))?;

            let value_start = 4 + msg_len;
            let value = i32::from_le_bytes([
                buf[value_start],
                buf[value_start + 1],
                buf[value_start + 2],
                buf[value_start + 3],
            ]);

            Ok(TestRequest { message, value })
        }

        fn merge(&mut self, buf: &[u8]) -> Result<(), Status> {
            let decoded = Self::decode(buf)?;
            self.message = decoded.message;
            self.value = decoded.value;
            Ok(())
        }
    }

    /// Simple response message for testing
    #[derive(Debug, Clone, Default, PartialEq)]
    struct TestResponse {
        pub result: String,
        pub count: i32,
    }

    impl Encoder for TestResponse {
        fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status> {
            let result_bytes = self.result.as_bytes();
            buf.extend_from_slice(&(result_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(result_bytes);
            buf.extend_from_slice(&self.count.to_le_bytes());
            Ok(())
        }

        fn encoded_len(&self) -> usize {
            4 + self.result.len() + 4
        }
    }

    impl Decoder for TestResponse {
        fn decode(buf: &[u8]) -> Result<Self, Status> {
            if buf.len() < 8 {
                return Err(Status::invalid_argument("Buffer too small"));
            }

            let result_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if buf.len() < 4 + result_len + 4 {
                return Err(Status::invalid_argument("Invalid buffer length"));
            }

            let result = String::from_utf8(buf[4..4 + result_len].to_vec())
                .map_err(|e| Status::invalid_argument(format!("Invalid UTF-8: {}", e)))?;

            let count_start = 4 + result_len;
            let count = i32::from_le_bytes([
                buf[count_start],
                buf[count_start + 1],
                buf[count_start + 2],
                buf[count_start + 3],
            ]);

            Ok(TestResponse { result, count })
        }

        fn merge(&mut self, buf: &[u8]) -> Result<(), Status> {
            let decoded = Self::decode(buf)?;
            self.result = decoded.result;
            self.count = decoded.count;
            Ok(())
        }
    }

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
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        // Create and configure server
        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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
        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_unary_unary_error_handling() {
        let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-error").unwrap();
        let service = Arc::new(Service::new(id));

        let server_name = Name::from_strings(["org", "ns", "server"]);
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
        let (server_app, server_notifications) = service
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
            .unwrap();
        let client_app = Arc::new(client_app);
        let channel = Channel::new(client_app.clone(), server_name);

        let request = TestRequest {
            message: "test".to_string(),
            value: 1,
        };

        let result: Result<TestResponse, Status> =
            channel.unary("TestService", "ErrorMethod", request, None, None).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), Some("Invalid input"));

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
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
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

        // Register stream-unary handler that sums all values
        server.registry().register_stream_unary(
            "TestService",
            "Sum",
            |mut request_stream: Box<dyn futures::Stream<Item = Result<TestRequest, Status>> + Send + Unpin + 'static>, _ctx: Context| async move {
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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
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
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

        // Register unary-stream handler that generates a sequence
        server.registry().register_unary_stream(
            "TestService",
            "Generate",
            |request: TestRequest, _ctx: Context| async move {
                let count = request.value;
                let responses: Vec<TestResponse> = (1..=count)
                    .map(|i| TestResponse {
                        result: format!("{}-{}", request.message, i),
                        count: i,
                    })
                    .collect();

                Ok(stream::iter(responses.into_iter().map(Ok)))
            },
        );

        let server_clone = server.clone();
        let server_handle = tokio::spawn(async move {
            let _ = server_clone.serve().await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let client_name = Name::from_strings(["org", "ns", "client"]);
        let (client_app, _) = service
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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
            responses.push(response);
        }

        // Verify responses
        assert_eq!(responses.len(), 5);
        assert_eq!(responses[0].result, "item-1");
        assert_eq!(responses[0].count, 1);
        assert_eq!(responses[4].result, "item-5");
        assert_eq!(responses[4].count, 5);

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
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
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

        // Register stream-stream handler that transforms each request
        server.registry().register_stream_stream(
            "TestService",
            "Transform",
            |request_stream, _ctx: Context| async move {
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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
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
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

        server.registry().register_stream_unary(
            "TestService",
            "EmptySum",
            |mut request_stream: Box<dyn futures::Stream<Item = Result<TestRequest, Status>> + Send + Unpin + 'static>, _ctx: Context| async move {
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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_concurrent_unary_calls() {
        let id = ID::new_with_name(Kind::new("slim").unwrap(), "test-service-concurrent").unwrap();
        let service = Arc::new(Service::new(id));

        let server_name = Name::from_strings(["org", "ns", "server"]);
        let secret = SharedSecret::new("test", TEST_VALID_SECRET).unwrap();
        let (server_app, server_notifications) = service
            .create_app(&server_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret.clone()))
            .unwrap();
        let server_app = Arc::new(server_app);

        let server = Server::new(server_app.clone(), server_name.clone(), server_notifications);

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
            .create_app(&client_name, AuthProvider::shared_secret(secret.clone()), AuthVerifier::shared_secret(secret))
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

        server.shutdown().await;
        drop(client_app);
        drop(server_app);
        let _ = tokio::time::timeout(Duration::from_millis(100), server_handle).await;
        drop(service);
    }
}
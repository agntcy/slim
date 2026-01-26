// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SlimRPC bindings
//!
//! These tests verify that the UniFFI-compatible SlimRPC bindings work correctly.

use std::sync::Arc;

use slim_bindings::{
    App, Code, Name, RequestStream, ResponseStream, RpcChannel, RpcContext, RpcServer, Status,
    StreamStreamHandler, StreamUnaryHandler, UnaryStreamHandler, UnaryUnaryHandler,
    initialize_with_defaults,
};

#[tokio::test]
async fn test_slimrpc_channel_creation() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test1".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test1".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Verify channel properties
    assert_eq!(channel.remote().components(), remote_name.components());
    assert_eq!(channel.connection_id(), None);
}

#[tokio::test]
async fn test_slimrpc_channel_with_connection_id() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test2".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel with connection ID
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test2".to_string(),
        "server".to_string(),
    ));

    let connection_id = Some(12345u64);
    let channel = RpcChannel::new_with_connection(app.clone(), remote_name.clone(), connection_id);

    // Verify channel properties
    assert_eq!(channel.remote().components(), remote_name.components());
    assert_eq!(channel.connection_id(), connection_id);
}

#[tokio::test]
async fn test_slimrpc_server_creation() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test3".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC server
    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test3".to_string(),
        "service".to_string(),
    ));

    let server = RpcServer::new(app.clone(), base_name.clone());

    // Verify server was created
    assert_eq!(server.methods().len(), 0); // No methods registered yet
}

#[tokio::test]
async fn test_slimrpc_server_with_connection_id() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test4".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC server with connection ID
    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test4".to_string(),
        "service".to_string(),
    ));

    let connection_id = Some(54321u64);
    let server = RpcServer::new_with_connection(app.clone(), base_name.clone(), connection_id);

    // Verify server was created successfully
    assert!(Arc::strong_count(&server) >= 1);
}

#[tokio::test]
async fn test_slimrpc_types_exist() {
    // This test just verifies that all the expected types are available
    // and can be constructed without error

    initialize_with_defaults();

    // These should all compile and be available
    let _code_ok = Code::Ok;
    let _code_err = Code::InvalidArgument;
    let _status = Status::ok();
    let _status_err = Status::new(Code::InvalidArgument, "test");
}

#[tokio::test]
async fn test_metadata_operations() {
    use slim_bindings::Metadata;

    // Create metadata and use builder pattern
    let metadata = Metadata::new();
    let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
    let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());

    // Get values
    assert_eq!(metadata.get("key1".to_string()), Some("value1".to_string()));
    assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));
    assert_eq!(metadata.get("nonexistent".to_string()), None);

    // Check keys
    assert!(metadata.contains_key("key1".to_string()));
    assert!(metadata.contains_key("key2".to_string()));
    assert!(!metadata.contains_key("nonexistent".to_string()));

    // Get all keys
    let keys = metadata.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys.contains(&"key1".to_string()));
    assert!(keys.contains(&"key2".to_string()));

    // Remove value
    let metadata = metadata.without("key1".to_string());
    assert_eq!(metadata.get("key1".to_string()), None);
    assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));

    // Clear metadata
    let metadata = metadata.clear();
    assert_eq!(metadata.keys().len(), 0);
}

#[tokio::test]
async fn test_status_codes() {
    // Test all status codes
    let codes = vec![
        Code::Ok,
        Code::Cancelled,
        Code::Unknown,
        Code::InvalidArgument,
        Code::DeadlineExceeded,
        Code::NotFound,
        Code::AlreadyExists,
        Code::PermissionDenied,
        Code::ResourceExhausted,
        Code::FailedPrecondition,
        Code::Aborted,
        Code::OutOfRange,
        Code::Unimplemented,
        Code::Internal,
        Code::Unavailable,
        Code::DataLoss,
        Code::Unauthenticated,
    ];

    for code in codes {
        let status = Status::new(code.clone(), "test message");
        assert_eq!(status.code, code);
        assert_eq!(status.message, Some("test message".to_string()));

        // Test is_ok and is_err
        if matches!(code, Code::Ok) {
            assert!(status.is_ok());
            assert!(!status.is_err());
        } else {
            assert!(!status.is_ok());
            assert!(status.is_err());
        }
    }

    // Test status with no message
    let status = Status::with_code(Code::Ok);
    assert_eq!(status.code, Code::Ok);
    assert_eq!(status.message, None);
}

#[tokio::test]
async fn test_unary_stream_rpc() {
    use slim_bindings::Metadata;

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test5".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test5".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test unary_stream method exists and has correct signature
    // Note: Returns a receiver that can be used to read responses
    let request = vec![1, 2, 3, 4];
    let metadata = Metadata::new();

    // Use the async version to avoid nested runtime
    let result = channel
        .unary_stream_async(
            "TestService".to_string(),
            "StreamMethod".to_string(),
            request,
            Some(5),
            Some(metadata),
        )
        .await;

    // Should return a receiver successfully
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_stream_unary_rpc() {
    use slim_bindings::{Metadata, VectorRequestSender};

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test6".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test6".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test stream_unary method exists and has correct signature
    let requests = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let sender = Arc::new(VectorRequestSender::new(requests));
    let metadata = Metadata::new();

    // Use the async version to avoid nested runtime
    let result = channel
        .stream_unary_async(
            "TestService".to_string(),
            "AggregateMethod".to_string(),
            sender,
            Some(5),
            Some(metadata),
        )
        .await;

    // We expect this to fail since there's no server, but the API should be correct
    assert!(result.is_err());
}

#[tokio::test]
async fn test_stream_stream_rpc() {
    use slim_bindings::{Metadata, VectorRequestSender};

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test7".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test7".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test stream_stream method exists and has correct signature
    let requests = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let sender = Arc::new(VectorRequestSender::new(requests));
    let metadata = Metadata::new();

    // Use the async version to avoid nested runtime
    let result = channel
        .stream_stream_async(
            "TestService".to_string(),
            "TransformMethod".to_string(),
            sender,
            Some(5),
            Some(metadata),
        )
        .await;

    // Should return a receiver successfully
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_all_rpc_patterns_compile() {
    use slim_bindings::VectorRequestSender;

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test8".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test8".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test all four RPC patterns
    let request = vec![1, 2, 3];

    // 1. Unary-Unary (this one will fail without server, but tests compilation)
    let _result = channel
        .unary_async(
            "Service".to_string(),
            "Method".to_string(),
            request.clone(),
            Some(5),
            None,
        )
        .await;

    // 2. Unary-Stream
    let _result = channel
        .unary_stream_async(
            "Service".to_string(),
            "StreamMethod".to_string(),
            request.clone(),
            Some(5),
            None,
        )
        .await;

    // 3. Stream-Unary
    let sender = Arc::new(VectorRequestSender::new(vec![request.clone()]));
    let _result = channel
        .stream_unary_async(
            "Service".to_string(),
            "AggregateMethod".to_string(),
            sender,
            Some(5),
            None,
        )
        .await;

    // 4. Stream-Stream
    let sender = Arc::new(VectorRequestSender::new(vec![request.clone()]));
    let _result = channel
        .stream_stream_async(
            "Service".to_string(),
            "TransformMethod".to_string(),
            sender,
            Some(5),
            None,
        )
        .await;
}

// ============================================================================
// Test Handler Implementations
// ============================================================================

struct TestUnaryUnaryHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for TestUnaryUnaryHandler {
    async fn handle(&self, _request: Vec<u8>, _ctx: Arc<RpcContext>) -> Result<Vec<u8>, Status> {
        Ok(vec![1, 2, 3])
    }
}

struct TestUnaryStreamHandler;

#[async_trait::async_trait]
impl UnaryStreamHandler for TestUnaryStreamHandler {
    async fn handle(
        &self,
        _request: Vec<u8>,
        _ctx: Arc<RpcContext>,
        response_stream: Arc<dyn ResponseStream>,
    ) -> Result<(), Status> {
        response_stream.send(vec![1, 2, 3]).await?;
        response_stream.send(vec![4, 5, 6]).await?;
        response_stream.close().await;
        Ok(())
    }
}

struct TestStreamUnaryHandler;

#[async_trait::async_trait]
impl StreamUnaryHandler for TestStreamUnaryHandler {
    async fn handle(
        &self,
        request_stream: Arc<dyn RequestStream>,
        _ctx: Arc<RpcContext>,
    ) -> Result<Vec<u8>, Status> {
        use slim_bindings::StreamResult;

        let mut count = 0;
        loop {
            match request_stream.next().await {
                StreamResult::Data { value: _data } => count += 1,
                StreamResult::End => break,
                StreamResult::Error { status } => return Err(status),
            }
        }
        Ok(vec![count as u8])
    }
}

struct TestStreamStreamHandler;

#[async_trait::async_trait]
impl StreamStreamHandler for TestStreamStreamHandler {
    async fn handle(
        &self,
        request_stream: Arc<dyn RequestStream>,
        _ctx: Arc<RpcContext>,
        response_stream: Arc<dyn ResponseStream>,
    ) -> Result<(), Status> {
        use slim_bindings::StreamResult;

        loop {
            match request_stream.next().await {
                StreamResult::Data { value: data } => {
                    response_stream.send(data).await?;
                }
                StreamResult::End => break,
                StreamResult::Error { status } => return Err(status),
            }
        }
        response_stream.close().await;
        Ok(())
    }
}

#[tokio::test]
async fn test_handler_registration() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app and server
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test9".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test9".to_string(),
        "service".to_string(),
    ));

    let server = RpcServer::new(app.clone(), base_name.clone());

    // Register handlers directly on server
    server.register_unary_unary(
        "TestService".to_string(),
        "Echo".to_string(),
        Arc::new(TestUnaryUnaryHandler),
    );

    server.register_unary_stream(
        "TestService".to_string(),
        "Replicate".to_string(),
        Arc::new(TestUnaryStreamHandler),
    );

    server.register_stream_unary(
        "TestService".to_string(),
        "Aggregate".to_string(),
        Arc::new(TestStreamUnaryHandler),
    );

    server.register_stream_stream(
        "TestService".to_string(),
        "Transform".to_string(),
        Arc::new(TestStreamStreamHandler),
    );

    // Verify server has the registered methods
    let methods = server.methods();
    assert!(methods.contains(&"TestService/Echo".to_string()));
    assert!(methods.contains(&"TestService/Replicate".to_string()));
    assert!(methods.contains(&"TestService/Aggregate".to_string()));
    assert!(methods.contains(&"TestService/Transform".to_string()));
}

#[tokio::test]
async fn test_server_registration_operations() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app and server
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test10".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret_async(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .await
    .expect("Failed to create app");

    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test10".to_string(),
        "service".to_string(),
    ));

    let server = RpcServer::new(app.clone(), base_name.clone());

    // Initially empty
    assert_eq!(server.methods().len(), 0);

    // Register a handler
    server.register_unary_unary(
        "Service1".to_string(),
        "Method1".to_string(),
        Arc::new(TestUnaryUnaryHandler),
    );

    assert_eq!(server.methods().len(), 1);

    // Register more handlers
    server.register_unary_stream(
        "Service1".to_string(),
        "Method2".to_string(),
        Arc::new(TestUnaryStreamHandler),
    );

    server.register_stream_unary(
        "Service2".to_string(),
        "Method1".to_string(),
        Arc::new(TestStreamUnaryHandler),
    );

    assert_eq!(server.methods().len(), 3);
}

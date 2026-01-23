// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for SlimRPC bindings
//!
//! These tests verify that the UniFFI-compatible SlimRPC bindings work correctly.

use std::sync::Arc;
use std::time::Duration;

use slim_bindings::{
    App, Name, RpcChannel, RpcServer, initialize_with_defaults, shutdown_blocking,
    IdentityProviderConfig, IdentityVerifierConfig, UnaryUnaryHandler, UnaryStreamHandler,
    StreamUnaryHandler, StreamStreamHandler, RpcContext, Status, Code, RequestStream, ResponseStream,
    StreamResult, VectorRequestSender,
};

#[test]
fn test_slimrpc_channel_creation() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test1".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(app_name.clone(), "test-secret-that-is-long-enough-for-hmac-validation".to_string())
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

    // Note: Don't shutdown in individual tests to avoid affecting other tests
}

#[test]
fn test_slimrpc_channel_with_connection_id() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test2".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(app_name.clone(), "test-secret-that-is-long-enough-for-hmac-validation".to_string())
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

    // Note: Don't shutdown in individual tests to avoid affecting other tests
}

#[test]
fn test_slimrpc_server_creation() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test3".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret(app_name.clone(), "test-secret-that-is-long-enough-for-hmac-validation".to_string())
        .expect("Failed to create app");

    // Create RPC server
    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test3".to_string(),
        "service".to_string(),
    ));

    let server = RpcServer::new(app.clone(), base_name.clone());

    // Server should be created successfully
    assert!(Arc::strong_count(&server) >= 1);

    // Note: Don't shutdown in individual tests to avoid affecting other tests
}

#[test]
fn test_slimrpc_server_with_connection_id() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test4".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret(app_name.clone(), "test-secret-that-is-long-enough-for-hmac-validation".to_string())
        .expect("Failed to create app");

    // Create RPC server with connection ID
    let base_name = Arc::new(Name::new(
        "org".to_string(),
        "test4".to_string(),
        "service".to_string(),
    ));

    let connection_id = Some(54321u64);
    let server = RpcServer::new_with_connection(app.clone(), base_name.clone(), connection_id);

    // Server should be created successfully
    assert!(Arc::strong_count(&server) >= 1);

    // Note: Don't shutdown in individual tests to avoid affecting other tests
}

#[test]
fn test_slimrpc_types_exist() {
    // This test just verifies that all the exported types are accessible
    use slim_bindings::{
        Code, Codec, Context, Decoder, Encoder, HandlerResponse, HandlerType, Metadata,
        RpcChannel, RpcContext, RpcError, RpcMessageContext, RpcServer, RpcSessionContext,
        Server, Status, StatusError, DEADLINE_KEY, MAX_TIMEOUT,
        STATUS_CODE_KEY,
    };

    // Just verify they compile and are in scope
    let _ = Code::Ok;
    let _ = HandlerType::UnaryUnary;
    let _ = DEADLINE_KEY;
    let _ = MAX_TIMEOUT;
    let _ = STATUS_CODE_KEY;
}

#[test]
fn test_metadata_operations() {
    use slim_bindings::Metadata;

    // Create new metadata
    let metadata = Metadata::new();
    assert!(metadata.is_empty());
    assert_eq!(metadata.len(), 0);

    // Add some key-value pairs
    let metadata = metadata.with_insert("key1".to_string(), "value1".to_string());
    let metadata = metadata.with_insert("key2".to_string(), "value2".to_string());

    assert!(!metadata.is_empty());
    assert_eq!(metadata.len(), 2);

    // Get values
    assert_eq!(metadata.get("key1".to_string()), Some("value1".to_string()));
    assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));
    assert_eq!(metadata.get("key3".to_string()), None);

    // Check if key exists
    assert!(metadata.contains_key("key1".to_string()));
    assert!(!metadata.contains_key("key3".to_string()));

    // Get keys and values
    let keys = metadata.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys.contains(&"key1".to_string()));
    assert!(keys.contains(&"key2".to_string()));

    let values = metadata.values();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&"value1".to_string()));
    assert!(values.contains(&"value2".to_string()));

    // Remove a key
    let metadata = metadata.without("key1".to_string());
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata.get("key1".to_string()), None);
    assert_eq!(metadata.get("key2".to_string()), Some("value2".to_string()));

    // Clear metadata
    let metadata = metadata.clear();
    assert!(metadata.is_empty());
    assert_eq!(metadata.len(), 0);
}

#[test]
fn test_status_codes() {
    use slim_bindings::{Code, Status};

    // Test status code creation
    let ok_status = Status::with_code(Code::Ok);
    assert!(ok_status.is_ok());
    assert!(!ok_status.is_err());

    let error_status = Status::new(Code::Internal, "test error".to_string());
    assert!(!error_status.is_ok());
    assert!(error_status.is_err());
    assert_eq!(error_status.message, Some("test error".to_string()));

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
        let status = Status::with_code(code);
        assert_eq!(status.code, code);
        
        if code as i32 == 0 {
            assert!(status.is_ok());
        } else {
            assert!(status.is_err());
        }
    }
}

#[test]
fn test_unary_stream_rpc() {
    use slim_bindings::Metadata;

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test5".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
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

    // This returns a receiver immediately, actual errors occur when reading
    let result = channel.unary_stream(
        "TestService".to_string(),
        "StreamMethod".to_string(),
        request,
        Some(5),
        Some(metadata),
    );

    // Should return a receiver successfully
    assert!(result.is_ok());
}

#[test]
fn test_stream_unary_rpc() {
    use slim_bindings::Metadata;

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test6".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test6".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test stream_unary method exists and has correct signature
    let requests = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let sender = Arc::new(VectorRequestSender::new(requests));
    let metadata = Metadata::new();

    // This would fail when the server tries to process, but the call succeeds
    let result = channel.stream_unary(
        "TestService".to_string(),
        "AggregateMethod".to_string(),
        sender,
        Some(5),
        Some(metadata),
    );

    // We expect this to fail since there's no server
    assert!(result.is_err());
}

#[test]
fn test_stream_stream_rpc() {
    use slim_bindings::Metadata;

    // Initialize the runtime
    initialize_with_defaults();

    // Create app
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test7".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .expect("Failed to create app");

    // Create RPC channel
    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test7".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Test stream_stream method exists and has correct signature
    let requests = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let sender = Arc::new(VectorRequestSender::new(requests));
    let metadata = Metadata::new();

    // This returns a receiver immediately, actual errors occur when reading
    let result = channel.stream_stream(
        "TestService".to_string(),
        "BidiMethod".to_string(),
        sender,
        Some(5),
        Some(metadata),
    );

    // Should return a receiver successfully
    assert!(result.is_ok());
}

#[test]
fn test_all_rpc_patterns_compile() {
    // This test just ensures all RPC patterns are available in the API
    initialize_with_defaults();

    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test8".to_string(),
        "client".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
    .expect("Failed to create app");

    let remote_name = Arc::new(Name::new(
        "org".to_string(),
        "test8".to_string(),
        "server".to_string(),
    ));

    let channel = RpcChannel::new(app.clone(), remote_name.clone());

    // Verify all four RPC patterns are available:
    // 1. Unary-Unary
    let _unary_result = channel.unary(
        "Service".to_string(),
        "Method".to_string(),
        vec![],
        None,
        None,
    );

    // 2. Unary-Stream
    let _unary_stream_result = channel.unary_stream(
        "Service".to_string(),
        "Method".to_string(),
        vec![],
        None,
        None,
    );

    // 3. Stream-Unary
    let sender = Arc::new(VectorRequestSender::new(vec![]));
    let _stream_unary_result = channel.stream_unary(
        "Service".to_string(),
        "Method".to_string(),
        sender,
        None,
        None,
    );

    // 4. Stream-Stream
    let sender = Arc::new(VectorRequestSender::new(vec![]));
    let _stream_stream_result = channel.stream_stream(
        "Service".to_string(),
        "Method".to_string(),
        sender,
        None,
        None,
    );

    // All patterns should compile and be callable
    // (they'll fail at runtime without a server, but that's expected)
}

// Test handler implementations

struct TestUnaryUnaryHandler;

#[async_trait::async_trait]
impl UnaryUnaryHandler for TestUnaryUnaryHandler {
    async fn handle(&self, request: Vec<u8>, _context: Arc<RpcContext>) -> Result<Vec<u8>, Status> {
        // Echo back the request
        Ok(request)
    }
}

struct TestUnaryStreamHandler;

#[async_trait::async_trait]
impl UnaryStreamHandler for TestUnaryStreamHandler {
    async fn handle(&self, request: Vec<u8>, _context: Arc<RpcContext>, response_stream: Arc<dyn ResponseStream>) -> Result<(), Status> {
        // Send multiple responses
        response_stream.send(request.clone()).await?;
        response_stream.send(request.clone()).await?;
        response_stream.send(request).await?;
        response_stream.close().await;
        Ok(())
    }
}

struct TestStreamUnaryHandler;

#[async_trait::async_trait]
impl StreamUnaryHandler for TestStreamUnaryHandler {
    async fn handle(&self, request_stream: Arc<dyn RequestStream>, _context: Arc<RpcContext>) -> Result<Vec<u8>, Status> {
        // Concatenate all requests
        let mut result = Vec::new();
        loop {
            match request_stream.next().await {
                StreamResult::Data { value } => {
                    result.extend(value);
                }
                StreamResult::End => break,
                StreamResult::Error { status } => return Err(status),
            }
        }
        Ok(result)
    }
}

struct TestStreamStreamHandler;

#[async_trait::async_trait]
impl StreamStreamHandler for TestStreamStreamHandler {
    async fn handle(&self, request_stream: Arc<dyn RequestStream>, _context: Arc<RpcContext>, response_stream: Arc<dyn ResponseStream>) -> Result<(), Status> {
        // Transform each request and send it back
        loop {
            match request_stream.next().await {
                StreamResult::Data { value } => {
                    let mut req = value;
                    req.reverse();
                    response_stream.send(req).await?;
                }
                StreamResult::End => break,
                StreamResult::Error { status } => return Err(status),
            }
        }
        response_stream.close().await;
        Ok(())
    }
}

#[test]
fn test_handler_registration() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app and server
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test9".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
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

#[test]
fn test_server_registration_operations() {
    // Initialize the runtime
    initialize_with_defaults();

    // Create app and server
    let app_name = Arc::new(Name::new(
        "org".to_string(),
        "test10".to_string(),
        "server".to_string(),
    ));

    let app = App::new_with_secret(
        app_name.clone(),
        "test-secret-that-is-long-enough-for-hmac-validation".to_string(),
    )
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
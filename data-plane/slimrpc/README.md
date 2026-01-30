# SlimRPC

A gRPC-like RPC framework built on top of the SLIM messaging protocol.

## Overview

SlimRPC provides a familiar gRPC-style RPC interface using SLIM sessions as the underlying transport. It supports all four standard gRPC streaming patterns:

- **Unary-Unary**: Single request → Single response
- **Unary-Stream**: Single request → Streaming responses
- **Stream-Unary**: Streaming requests → Single response
- **Stream-Stream**: Streaming requests → Streaming responses

## Architecture

This crate contains the **core SlimRPC implementation** using SLIM core types directly, with no dependency on the FFI bindings layer. This makes it suitable for:

- Pure Rust applications using SLIM
- Custom integrations that don't need FFI
- Building higher-level abstractions

The bindings crate (`agntcy-slim-bindings`) provides a separate FFI layer built on top of this core library for language interoperability (Go, Python, Swift, Kotlin, etc.).

## Key Types

### Core SLIM Dependencies

SlimRPC works directly with core SLIM types:

- `slim_service::app::App<P, V>` - The SLIM application instance
- `slim_session::context::SessionContext` - Session context for RPC calls
- `slim_datapath::messages::Name` - SLIM names for service addressing
- `slim_auth::auth_provider::{AuthProvider, AuthVerifier}` - Authentication types

### SlimRPC Types

- `Channel` - Client-side RPC channel for making calls
- `Server` - Server-side RPC handler and dispatcher
- `Context` - Request context with metadata, deadlines, session info
- `Status` / `Code` - RPC status and error codes (gRPC-compatible)
- `Metadata` - Key-value metadata for requests/responses

## Client Usage

```rust
use std::sync::Arc;
use slim_datapath::messages::Name;
use slimrpc::{Channel, Metadata};

// Create a channel to a remote service
let remote_name = Name::from_strings(["org", "namespace", "service"]);
let channel = Channel::new(app.clone(), remote_name);

// Make a unary RPC call
let response = channel
    .unary("MyService", "MyMethod", request, None, None)
    .await?;

// Make a streaming RPC call
let mut stream = channel.unary_stream(
    "MyService", 
    "StreamMethod", 
    request, 
    Some(Duration::from_secs(30)),
    None
);

while let Some(response) = stream.next().await {
    // Process streaming response
}
```

## Server Usage

```rust
use std::sync::Arc;
use slim_datapath::messages::Name;
use slimrpc::{Server, Context, Status};

// Create a server
let base_name = Name::from_strings(["org", "namespace", "myapp"]);
let mut server = Server::new(app, base_name, notification_rx);

// Register a unary-unary handler
server.registry().register_unary_unary(
    "MyService",
    "MyMethod",
    |request: MyRequest, ctx: Context| async move {
        // Handle the request
        let response = process_request(request).await?;
        Ok(response)
    }
);

// Start the server
server.serve().await?;
```

## Connection ID Propagation

SlimRPC supports connection ID propagation for multi-node SLIM topologies:

```rust
// Client with connection ID
let channel = Channel::new_with_connection(
    app.clone(), 
    remote_name, 
    Some(connection_id)
);

// Server with connection ID
let server = Server::new_with_connection(
    app, 
    base_name, 
    Some(connection_id),
    notification_rx
);
```

This allows sessions and subscriptions to propagate correctly across SLIM nodes.

## Codec Traits

SlimRPC uses `Encoder` and `Decoder` traits for message serialization:

```rust
pub trait Encoder {
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), Status>;
    fn encode_to_vec(&self) -> Result<Vec<u8>, Status>;
    fn encoded_len(&self) -> usize;
}

pub trait Decoder: Default {
    fn decode(buf: &[u8]) -> Result<Self, Status>;
    fn merge(&mut self, buf: &[u8]) -> Result<(), Status>;
}
```

These are typically implemented by protobuf-generated code from the `slimrpc-compiler` plugin.

## Error Handling

SlimRPC uses gRPC-compatible status codes:

```rust
use slimrpc::{Status, Code};

// Create status errors
return Err(Status::invalid_argument("Invalid request"));
return Err(Status::not_found("Resource not found"));
return Err(Status::internal("Internal error"));

// Check status codes
if status.code() == Code::NotFound {
    // Handle not found
}
```

## Metadata

Metadata can be attached to requests and responses:

```rust
use slimrpc::Metadata;

let mut metadata = Metadata::new();
metadata.insert("user-id", "12345");
metadata.insert("trace-id", "abc-def-123");

// Send with request
let response = channel.unary(
    "MyService",
    "MyMethod",
    request,
    Some(Duration::from_secs(10)),
    Some(metadata)
).await?;

// Access in handler
fn my_handler(request: MyRequest, ctx: Context) -> Result<MyResponse, Status> {
    let user_id = ctx.metadata().get("user-id");
    // ...
}
```

## Timeouts and Deadlines

```rust
// Set timeout on client side
let response = channel.unary(
    "MyService",
    "MyMethod",
    request,
    Some(Duration::from_secs(30)), // timeout
    None
).await?;

// Check deadline in handler
fn my_handler(request: MyRequest, ctx: Context) -> Result<MyResponse, Status> {
    if ctx.is_deadline_exceeded() {
        return Err(Status::deadline_exceeded("Request timeout"));
    }
    
    let remaining = ctx.remaining_time();
    // ...
}
```

## Integration with Protobuf

Use the `slimrpc-compiler` crate to generate SlimRPC service stubs from `.proto` files:

```proto
service MyService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc StreamData(DataRequest) returns (stream DataResponse);
}
```

The compiler generates Rust code implementing the `Encoder` and `Decoder` traits, along with service trait definitions.

## Testing

Run tests with:

```bash
cargo test
```

Some tests require a running SLIM infrastructure and are marked with `#[ignore]`:

```bash
cargo test -- --ignored
```

## License

Apache-2.0
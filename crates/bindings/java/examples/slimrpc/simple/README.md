# slimrpc Simple Example (Java)

This example demonstrates all slimrpc RPC patterns in Java with comprehensive client and server implementations.

## Prerequisites

- Java 21+
- Maven 3.9+
- Running SLIM server on `localhost:46357`
- `buf` CLI tool installed
- `slim-bindings-java` installed locally (`mvn install` from `data-plane/bindings/java/`)

## Generate Code

```bash
buf generate
```

This generates:
- `types/` - Standard protobuf Java types
- `slimrpc/` - slimrpc client and server stubs

## Run the Example

```bash
cd data-plane/bindings/java
```

In the first terminal, start a SLIM instance:

```bash
task examples:server
```

In another terminal, start the rpc server:

```bash
task examples:rpc:server
```

In the third terminal, run the rpc client:

```bash
task examples:rpc:client
```

### Group (Multicast) Client

To run the group client, start two server instances (server1 and server2) and then:

```bash
task examples:rpc:group-client
```

The group client broadcasts RPCs to multiple server instances using `TestGroupClient`
and demonstrates all four multicast streaming patterns.

## Code Structure

### Server (`src/main/java/.../SlimrpcServerMain.java`)

Implements all four RPC patterns:

1. **UnaryUnary**: Simple request-response
2. **UnaryStream**: Single request, multiple responses (server streaming)
3. **StreamUnary**: Multiple requests, single response (client streaming)
4. **StreamStream**: Bidirectional streaming

The server implements the `TestServer` interface using anonymous class overrides on `UnimplementedTestServer` for forward compatibility. Sync stream wrappers (`ServerRequestStreamSync`, `ServerResponseStreamSync`, `ServerBidiStreamSync`) provide blocking `send()`/`recv()` methods.

### Client (`src/main/java/.../SlimrpcClientMain.java`)

Demonstrates all four RPC patterns using sync wrappers:

#### 1. Unary-Unary
```java
ExampleResponse response = client.ExampleUnaryUnary(request, Duration.ofSeconds(10), null);
```

#### 2. Unary-Stream (Server Streaming)
```java
ResponseStreamReader reader = client.ExampleUnaryStream(request, timeout, null);
ClientResponseStreamSync<ExampleResponse> stream = ClientResponseStreamSync.create(reader, parser);
while (true) {
    ExampleResponse resp = stream.recv();
    if (resp == null) break;
}
```

#### 3. Stream-Unary (Client Streaming)
```java
ClientRequestStreamSync<ExampleRequest, ExampleResponse> stream = client.ExampleStreamUnary(timeout, null);
stream.send(request);
ExampleResponse response = stream.finalizeStream();
```

#### 4. Stream-Stream (Bidirectional Streaming)
```java
ClientBidiStreamSync<ExampleRequest> stream = client.ExampleStreamStream(timeout, null);
stream.send(request);
stream.closeSend();
StreamMessage msg = stream.recv();
```

## Features

- All 4 RPC patterns (unary-unary, unary-stream, stream-unary, stream-stream)
- Multicast (group) RPC across multiple server instances
- Type-safe client and server interfaces
- Synchronous stream wrappers for blocking send/recv
- Automatic serialization/deserialization
- Proper error handling with RPC exception conversion
- Stream end detection (null return)
- Forward-compatible server with unimplemented method stubs

### Group Client (`src/main/java/.../SlimrpcGroupClientMain.java`)

Demonstrates multicast RPC using a group channel that targets multiple server instances:

#### 1. Multicast Unary-Unary
```java
Channel channel = Channel.newGroupWithConnection(app, serverNames, connId);
TestSlimrpc.TestGroupClient client = new TestSlimrpc.TestGroupClientImpl(channel);
MulticastResponseStream<ExampleResponse> stream = client.ExampleUnaryUnary(request, timeout, null);
// Iterate: each item carries a context (source server) and the response
```

#### 2. Multicast Unary-Stream (Server Streaming)
```java
MulticastResponseStream<ExampleResponse> stream = client.ExampleUnaryStream(request, timeout, null);
// Iterate multicast items from all servers
```

#### 3. Multicast Stream-Unary (Client Streaming)
```java
MulticastClientBidiStream<ExampleRequest, ExampleResponse> stream = client.ExampleStreamUnary(timeout, null);
stream.send(request);
stream.closeSend();
// Receive one multicast response per server
```

#### 4. Multicast Stream-Stream (Bidirectional Streaming)
```java
MulticastClientBidiStream<ExampleRequest, ExampleResponse> stream = client.ExampleStreamStream(timeout, null);
// Send in background thread, receive multicast items
```

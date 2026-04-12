# slimrpc Simple Example (Kotlin)

This example demonstrates all slimrpc RPC patterns in Kotlin with comprehensive client and server implementations using coroutines.

## Prerequisites

- Java 17+ (JDK)
- Gradle (wrapper included)
- Running SLIM server on `localhost:46357`
- `buf` CLI tool installed
- `slim-bindings-kotlin` installed locally (`./gradlew publishToMavenLocal` from `data-plane/bindings/kotlin/`)

## Generate Code

First build the Kotlin protoc plugin:

```bash
cargo build --release -p agntcy-protoc-slimrpc-plugin --bin protoc-gen-slimrpc-kotlin
```

Then generate the code:

```bash
buf generate
```

This generates:
- `types/` - Standard protobuf Java types (usable from Kotlin)
- `slimrpc/` - slimrpc Kotlin client and server stubs

## Run the Example

In the first terminal, start a SLIM instance:

```bash
# From data-plane/bindings/kotlin/
task examples:server
```

In another terminal, start the rpc server:

```bash
./gradlew server
```

In the third terminal, run the rpc client:

```bash
./gradlew client
```

### Group (Multicast) Client

To run the group client, start two server instances (server1 and server2) and then:

```bash
./gradlew groupClient
```

The group client broadcasts RPCs to multiple server instances using `TestGroupClient`
and demonstrates all four multicast streaming patterns.

## Code Structure

### Server (`src/main/kotlin/.../SlimrpcServerMain.kt`)

Implements all four RPC patterns using `suspend` functions:

1. **UnaryUnary**: Simple request-response
2. **UnaryStream**: Single request, multiple responses (server streaming)
3. **StreamUnary**: Multiple requests, single response (client streaming)
4. **StreamStream**: Bidirectional streaming

The server implements the `TestServer` interface using anonymous object overrides on `UnimplementedTestServer` for forward compatibility. Suspend stream wrappers (`ServerRequestStream`, `ServerResponseStream`, `ServerBidiStream`) provide coroutine-based `send()`/`recv()` methods.

### Client (`src/main/kotlin/.../SlimrpcClientMain.kt`)

Demonstrates all four RPC patterns using Kotlin coroutines:

#### 1. Unary-Unary
```kotlin
val response = client.ExampleUnaryUnary(request, Duration.ofSeconds(10), null)
```

#### 2. Unary-Stream (Server Streaming)
```kotlin
val reader = client.ExampleUnaryStream(request, timeout, null)
val stream = ClientResponseStream.create(reader) { ExampleResponse.parseFrom(it) }
while (true) {
    val resp = stream.recv() ?: break
}
```

#### 3. Stream-Unary (Client Streaming)
```kotlin
val stream = client.ExampleStreamUnary(timeout, null)
stream.send(request)
val response = stream.finalizeStream()
```

#### 4. Stream-Stream (Bidirectional Streaming)
```kotlin
val stream = client.ExampleStreamStream(timeout, null)
stream.send(request)
stream.closeSend()
val msg = stream.recv()
```

## Features

- All 4 RPC patterns (unary-unary, unary-stream, stream-unary, stream-stream)
- Multicast (group) RPC across multiple server instances
- Type-safe client and server interfaces
- Kotlin coroutine-based stream wrappers (`suspend` send/recv)
- Automatic serialization/deserialization
- Proper error handling with RPC exception conversion
- Stream end detection (null return)
- Forward-compatible server with unimplemented method stubs

### Group Client (`src/main/kotlin/.../SlimrpcGroupClientMain.kt`)

Demonstrates multicast RPC using a group channel that targets multiple server instances:

#### 1. Multicast Unary-Unary
```kotlin
val channel = Channel.newGroupWithConnection(app, serverNames, connId)
val client = TestSlimrpc.TestGroupClientImpl(channel)
val stream = client.ExampleUnaryUnary(request, timeout, null)
// Iterate: each item carries a context (source server) and the response
```

#### 2. Multicast Unary-Stream (Server Streaming)
```kotlin
val stream = client.ExampleUnaryStream(request, timeout, null)
// Iterate multicast items from all servers
```

#### 3. Multicast Stream-Unary (Client Streaming)
```kotlin
val stream = client.ExampleStreamUnary(timeout, null)
stream.send(request)
stream.closeSend()
// Receive one multicast response per server
```

#### 4. Multicast Stream-Stream (Bidirectional Streaming)
```kotlin
val stream = client.ExampleStreamStream(timeout, null)
// Send in coroutine, receive multicast items
```

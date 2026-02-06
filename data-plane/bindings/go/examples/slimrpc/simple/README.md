# slimrpc Simple Example (Go)

This example demonstrates all slimrpc RPC patterns in Go with comprehensive client and server implementations.

## Prerequisites

- Go 1.21+
- Running SLIM server on `localhost:46357`
- `buf` CLI tool installed

## Generate Code

```bash
buf generate
```

This generates:
- `types/example.pb.go` - Standard protobuf types
- `types/example_slimrpc.pb.go` - slimrpc client and server stubs

## Run the Example

In one terminal, start the server:

```bash
go run cmd/server/server.go
```

In another terminal, run the client:

```bash
go run cmd/client/client.go
```

## Code Structure

### Server (`cmd/server/server.go`)

Implements all four RPC patterns:

1. **UnaryUnary**: Simple request-response
2. **UnaryStream**: Single request, multiple responses
3. **StreamUnary**: Multiple requests, single response
4. **StreamStream**: Bidirectional streaming

The server implements the `TestServer` interface and embeds `UnimplementedTestServer` for forward compatibility.

### Client (`cmd/client/client.go`)

Demonstrates all four RPC patterns:

#### 1. Unary-Unary
```go
response, err := client.ExampleUnaryUnary(ctx, request)
```

#### 2. Unary-Stream (Server Streaming)
```go
stream, err := client.ExampleUnaryStream(ctx, request)
for {
    resp, err := stream.Recv()
    if resp == nil { break }
    // Process response
}
```

#### 3. Stream-Unary (Client Streaming)
```go
stream, err := client.ExampleStreamUnary(ctx)
for i := 0; i < 10; i++ {
    stream.Send(request)
}
response, err := stream.CloseAndRecv()
```

#### 4. Stream-Stream (Bidirectional Streaming)
```go
stream, err := client.ExampleStreamStream(ctx)

// Send in goroutine
go func() {
    for i := 0; i < 10; i++ {
        stream.Send(request)
    }
    stream.CloseSend()
}()

// Receive
for {
    resp, err := stream.Recv()
    if resp == nil { break }
    // Process response
}
```

## Generated Code

The `buf generate` command produces type-safe interfaces for all RPC patterns:

### Client Interface
```go
type TestClient interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
    ExampleUnaryStream(ctx context.Context, req *ExampleRequest) (Test_ExampleUnaryStreamClient, error)
    ExampleStreamUnary(ctx context.Context) (Test_ExampleStreamUnaryClient, error)
    ExampleStreamStream(ctx context.Context) (Test_ExampleStreamStreamClient, error)
}
```

### Server Interface
```go
type TestServer interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
    ExampleUnaryStream(req *ExampleRequest, stream Test_ExampleUnaryStreamServer) error
    ExampleStreamUnary(stream Test_ExampleStreamUnaryServer) error
    ExampleStreamStream(stream Test_ExampleStreamStreamServer) error
}
```

### Stream Interfaces
Each streaming method gets its own interface with `Send()` and/or `Recv()` methods:

```go
// Server streaming
type Test_ExampleUnaryStreamServer interface {
    Send(*ExampleResponse) error
}

// Client streaming
type Test_ExampleStreamUnaryServer interface {
    Recv() (*ExampleRequest, error)
}

// Bidirectional streaming
type Test_ExampleStreamStreamServer interface {
    Send(*ExampleResponse) error
    Recv() (*ExampleRequest, error)
}
```

## Features

- ✅ All 4 RPC patterns (unary-unary, unary-stream, stream-unary, stream-stream)
- ✅ Type-safe client and server interfaces
- ✅ Automatic serialization/deserialization
- ✅ Proper error handling with RPC error conversion
- ✅ Stream end detection (nil return)
- ✅ Forward-compatible server with unimplemented method stubs
- ✅ Concurrent send/receive for bidirectional streaming

## Notes

- `Recv()` returns `nil` when the stream ends (not an error)
- Error handling automatically converts `RpcError` to standard Go errors
- Bidirectional streaming uses goroutines for concurrent send/receive
- The server automatically closes response streams when handlers return

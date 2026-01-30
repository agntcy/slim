# slimrpc Simple Example (Go)

This example demonstrates basic slimrpc usage in Go with unary RPC calls.

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
go run server.go
```

In another terminal, run the client:

```bash
go run client.go
```

## Code Structure

### Server (`server.go`)

1. Implements the `TestServer` interface
2. Embeds `UnimplementedTestServer` for forward compatibility
3. Creates a SLIM app and server
4. Registers the service implementation
5. Starts serving requests

### Client (`client.go`)

1. Creates a SLIM app and channel
2. Creates a typed client stub
3. Makes RPC calls with automatic serialization/deserialization

## Generated Code

The `buf generate` command produces:

```go
// Client Interface
type TestClient interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
}

// Server Interface
type TestServer interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
}

// Registration Function
func RegisterTestServer(server *slim_bindings.Server, impl TestServer)
```

## Notes

- The example currently only implements unary-unary RPC
- Streaming RPC support (unary-stream, stream-unary, stream-stream) coming soon
- Error handling uses Go's standard error type
- Context is used for cancellation and timeouts

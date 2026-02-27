# Slim RPC Compiler

The Slim RPC Compiler is a collection of protoc plugins that generate client
stubs and server handlers for slimrpc from Protocol Buffer service definitions.
These plugins enable you to build high-performance RPC services using the
slimrpc framework.

## Supported Languages

- **Python**: `protoc-gen-slimrpc-python`
- **Go**: `protoc-gen-slimrpc-go`

## Features

- Generates type-safe client stubs and server handlers for slimrpc services
- Supports all gRPC streaming patterns: unary-unary, unary-stream, stream-unary,
  and stream-stream
- Compatible with both `protoc` and `buf` build systems (buf recommended)
- Automatic import resolution for Protocol Buffer dependencies

## Installation

### Compile from Source

1. Clone the repository:

```bash
git clone https://github.com/agntcy/slim.git
cd slim/data-plane
```

2. Build the plugins:

```bash
cargo build --release
```

3. The compiled binaries will be available at:
   - `target/release/protoc-gen-slimrpc-python`
   - `target/release/protoc-gen-slimrpc-go`

## Usage

The recommended way to use the slimrpc compiler is through `buf`. Both Python
and Go examples use this approach.

### Using with buf (Recommended)

#### Prerequisites

- `buf` CLI installed
- Compiled slimrpc plugins (see Installation above)

#### Python Example

Create a `buf.gen.yaml` file in your project:

```yaml
version: v2
managed:
  enabled: true
inputs:
  - proto_file: example.proto
plugins:
  # Generate slimrpc stubs
  - local: /path/to/target/release/protoc-gen-slimrpc-python
    out: types
  # Generate standard protobuf code
  - remote: buf.build/protocolbuffers/python:v29.3
    out: types
  # Generate type stubs
  - remote: buf.build/protocolbuffers/pyi:v31.1
    out: types
```

#### Go Example

Create a `buf.gen.yaml` file in your project:

```yaml
version: v2
managed:
  enabled: true
plugins:
  # Generate standard .pb.go files
  - remote: buf.build/protocolbuffers/go
    out: types
    opt:
      - paths=source_relative
  # Generate slimrpc stubs
  - local: /path/to/target/release/protoc-gen-slimrpc-go
    out: types
    opt:
      - paths=source_relative
```

#### Generate Code

```bash
buf generate
```

This will generate:
- **Python**: `*_pb2.py` (protobuf types) and `*_pb2_slimrpc.py` (slimrpc stubs)
- **Go**: `*.pb.go` (protobuf types) and `*_slimrpc.pb.go` (slimrpc stubs)

### Using with protoc (Alternative)

If you prefer to use `protoc` directly:

#### Python

```bash
protoc \
  --python_out=. \
  --pyi_out=. \
  --plugin=protoc-gen-slimrpc-python=/path/to/protoc-gen-slimrpc-python \
  --slimrpc-python_out=. \
  example.proto
```

#### Go

```bash
protoc \
  --go_out=. \
  --plugin=protoc-gen-slimrpc-go=/path/to/protoc-gen-slimrpc-go \
  --slimrpc-go_out=. \
  example.proto
```

## Generated Code Structure

### Python

For a service definition, the generated `*_pb2_slimrpc.py` contains:

#### Client Stub

```python
class TestStub:
    """Client stub for Test."""
    
    def __init__(self, channel: slim_bindings.Channel):
        self._channel = channel
    
    async def ExampleUnaryUnary(
        self, 
        request: pb2.ExampleRequest, 
        timeout: slim_bindings.Duration | None = None
    ) -> pb2.ExampleResponse:
        """Call ExampleUnaryUnary method."""
        # Implementation handles serialization and channel communication
```

#### Server Handler Protocol

```python
class TestServer(Protocol):
    """Server protocol for Test service."""
    
    async def ExampleUnaryUnary(
        self,
        request: pb2.ExampleRequest,
        context: slim_bindings.ServerContext,
    ) -> pb2.ExampleResponse:
        """Handle ExampleUnaryUnary."""
        ...
```

#### Registration Function

```python
def register_test_server(
    server: slim_bindings.Server,
    handler: TestServer,
) -> None:
    """Register Test service handlers with the server."""
    # Registers all service methods
```

### Go

For a service definition, the generated `*_slimrpc.pb.go` contains:

#### Client Interface

```go
type TestClient interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
    ExampleUnaryStream(ctx context.Context, req *ExampleRequest) (ResponseStream[*ExampleResponse], error)
    ExampleStreamUnary(ctx context.Context) (RequestStream[*ExampleRequest, *ExampleResponse], error)
    ExampleStreamStream(ctx context.Context) (BidiStream[*ExampleRequest, *ExampleResponse], error)
}
```

#### Server Interface

```go
type TestServer interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
    ExampleUnaryStream(req *ExampleRequest, stream ResponseStream[*ExampleResponse]) error
    ExampleStreamUnary(stream RequestStream[*ExampleRequest, *ExampleResponse]) error
    ExampleStreamStream(stream BidiStream[*ExampleRequest, *ExampleResponse]) error
}
```

#### Registration Function

```go
func RegisterTestServer(server slim_bindings.ServerInterface, handler TestServer) {
    // Registers all service methods with type-safe handlers
}
```

## Examples

Complete working examples are available in the repository:

- **Python**: [`bindings/python/examples/slimrpc/simple`](../bindings/python/examples/slimrpc/simple)
- **Go**: [`bindings/go/examples/slimrpc/simple`](../bindings/go/examples/slimrpc/simple)

Both examples demonstrate all four RPC patterns with comprehensive client and
server implementations.

## Plugin Parameters

### Python Plugin

- `types_import`: Customize how protobuf types are imported
  - Example: `types_import="from my_package import types_pb2 as pb2"`
  - Default: Uses local import based on the proto file name

### Go Plugin

- `paths`: Control output path strategy (handled by buf/protoc, ignored by plugin)
  - `source_relative`: Generate files relative to the proto file location

- `types_import`: Go import path for the protobuf-generated types package.
  Required when the generated slimrpc file lives in a different package than
  the proto types.
  - Example: `types_import=github.com/myorg/myrepo/types`
  - Default: not set (types are used as bare unqualified names, requires same package)

- `types_alias`: Optional Go package alias to use when referencing proto types.
  Only valid when `types_import` is also set.
  - Example: `types_alias=pb`
  - Default: last path component of `types_import` (e.g., `"types"` for `.../types`)

## Troubleshooting

### Plugin Not Found

If you get an error that the plugin is not found:

- Ensure the plugin binary is in your PATH or use full path in `buf.gen.yaml`
- Check that the binary is executable: `chmod +x /path/to/protoc-gen-slimrpc-*`

### Import Errors

If you encounter import errors:

- Make sure the generated files are in your module path
- For Python: Verify the `types_import` parameter matches your project structure
- For Go: Ensure `go.mod` is properly configured with correct module paths

### Build Errors

If the plugin fails to build:

- Ensure you have Rust and Cargo installed (latest stable version)
- Check that all dependencies are available
- Try cleaning and rebuilding: `cargo clean && cargo build --release`

## Contributing

Please see the main repository's contributing guidelines at
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## License

This project is licensed under the Apache 2.0 License - see the
[LICENSE.md](../../LICENSE.md) file for details.

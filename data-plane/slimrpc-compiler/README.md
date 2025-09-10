# Slim RPC Compiler

The Slim RPC Compiler (`protoc-slimrpc-plugin`) is a protoc plugin that
generates Python client stubs and server servicers for SRPC (Slim RPC) from
Protocol Buffer service definitions. This plugin enables you to build
high-performance RPC services using the SRPC framework.

## Features

- Generates Python client stubs for calling SlimRPC services
- Generates Python server servicers for implementing SlimRPC services
- Supports all gRPC streaming patterns: unary-unary, unary-stream, stream-unary,
  and stream-stream
- Compatible with both `protoc` and `buf` build systems
- Automatic import resolution for Protocol Buffer dependencies

## Installation

### Option 1: Install via Cargo

```bash
cargo install agntcy-protoc-slimrpc-plugin
```

This will install the `protoc-slimrpc-plugin` binary to your Cargo bin directory
(usually `~/.cargo/bin`).

### Option 2: Compile from Source

1. Clone the repository:

```bash
git clone https://github.com/agntcy/slim.git
cd slim/data-plane/slimrpc-compiler
```

2. Build the plugin:

```bash
cargo build --release
```

3. The compiled binary will be available at
   `data-plane/target/release/protoc-slimrpc-plugin`

## Usage

### Example Protocol Buffer Definition

Create a file called `example.proto`:

```proto
syntax = "proto3";

package example_service;

service Test {
  rpc ExampleUnaryUnary(ExampleRequest) returns (ExampleResponse);
  rpc ExampleUnaryStream(ExampleRequest) returns (stream ExampleResponse);
  rpc ExampleStreamUnary(stream ExampleRequest) returns (ExampleResponse);
  rpc ExampleStreamStream(stream ExampleRequest) returns (stream ExampleResponse);
}

message ExampleRequest {
  string example_string = 1;
  int64  example_integer = 2;
}

message ExampleResponse {
  string example_string = 1;
  int64  example_integer = 2;
}
```

### Using with protoc

#### Prerequisites

Make sure you have:

- `protoc` (Protocol Buffer compiler) installed
- The `protoc-slimrpc-plugin` binary in your PATH or specify its full path

#### Generate Python Files

```bash
# Generate both the protobuf Python files and SRPC files
protoc \
  --python_out=. \
  --pyi_out=. \
  --plugin=~/.cargo/bin/protoc-slimrpc-plugin \
  --slimrpc_out=. \
  example.proto
```

This will generate:

- `example_pb2.py` - Standard protobuf Python bindings
- `example_pb2_slimrpc.py` - SRPC client stubs and server servicers

#### With Custom Types Import

You can specify a custom import for the types module. This allows to import the
types from an external package.

For instance, if you don't want to generate the types and you want to import
them from a2a.grpc.a2a_pb2`,, you can do:

```bash
protoc \
  --plugin=~/.cargo/bin/protoc-slimrpc-plugin \
  --slimrpc_out=types_import="from a2a.grpc import a2a_pb2 as a2a__pb2":. \
  example.proto
```

### Using with buf

#### Prerequisites

- `buf` CLI installed
- `protoc-slimrpc-plugin` binary in your PATH

#### Create buf.gen.yaml

Create a `buf.gen.yaml` file in your project root:

```yaml
version: v2
managed:
  enabled: true
inputs:
  - proto_file: example.proto
plugins:
  - local: /path/to/protoc-slimrpc-plugin
    out: .
  - remote: buf.build/protocolbuffers/python
    out: .
```

#### Generate Code

```bash
buf generate
```

Or generate from a specific file:

```bash
buf generate --path example.proto
```

#### Advanced buf Configuration

For more complex setups with custom options:

```yaml
version: v2
managed:
  enabled: true
plugins:
  - local: protoc-slimrpc-plugin
    out: generated
    opt:
      - types_import=from .pb2_types import example_pb2 as pb2
    strategy: all
  - remote: buf.build/protocolbuffers/python
    out: generated
    strategy: all
```

## Generated Code Structure

For the example above, the generated `example_pb2_slimrpc.py` will contain:

### Client Stub

```python
class TestStub:
    """Client stub for Test."""
    def __init__(self, channel):
        """Constructor.

        Args:
            channel: A slimrpc.Channel.
        """
        self.ExampleUnaryUnary = channel.unary_unary(...)
        self.ExampleUnaryStream = channel.unary_stream(...)
        # ... other methods
```

### Server Servicer

```python
class TestServicer():
    """Server servicer for Test. Implement this class to provide your service logic."""

    def ExampleUnaryUnary(self, request, context):
        """Method for ExampleUnaryUnary. Implement your service logic here."""
        raise slimrpc_rpc.SRPCResponseError(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )
    # ... other methods
```

### Registration Function

```python
def add_TestServicer_to_server(servicer, server: slimrpc.Server):
    # Registers the servicer with the SRPC server
    pass
```

## Plugin Parameters

The plugin supports the following parameters:

- `types_import`: Customize how protobuf types are imported
  - Example: `types_import="from my_package import types_pb2 as pb2"`
  - Default: Uses local import based on the proto file name

## Example Usage in Python

### Client Usage

```python
import asyncio
import logging
from collections.abc import AsyncGenerator

import slimrpc
from slimrpc.examples.simple.types.example_pb2 import ExampleRequest
from slimrpc.examples.simple.types.example_pb2_slimrpc import TestStub

logger = logging.getLogger(__name__)


async def amain() -> None:
    channel = slimrpc.Channel(
        local="agntcy/grpc/client",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
        remote="agntcy/grpc/server",
    )

    # Stubs
    stubs = TestStub(channel)

    # Call method
    try:
        request = ExampleRequest(example_integer=1, example_string="hello")
        response = await stubs.ExampleUnaryUnary(request, timeout=2)

        logger.info(f"Response: {response}")

        responses = stubs.ExampleUnaryStream(request, timeout=2)
        async for resp in responses:
            logger.info(f"Stream Response: {resp}")

        async def stream_requests() -> AsyncGenerator[ExampleRequest]:
            for i in range(10):
                yield ExampleRequest(example_integer=i, example_string=f"Request {i}")

        response = await stubs.ExampleStreamUnary(stream_requests(), timeout=2)
        logger.info(f"Stream Unary Response: {response}")
    except asyncio.TimeoutError:
        logger.error("timeout while waiting for response")
```

### Server Usage

```python
import asyncio
import logging
from collections.abc import AsyncIterable

from slimrpc.context import Context
from slimrpc.examples.simple.types.example_pb2 import ExampleRequest, ExampleResponse
from slimrpc.examples.simple.types.example_pb2_slimrpc import (
    TestServicer,
    add_TestServicer_to_server,
)
from slimrpc.server import Server

logger = logging.getLogger(__name__)


class TestService(TestServicer):
    async def ExampleUnaryUnary(
        self, request: ExampleRequest, context: Context
    ) -> ExampleResponse:
        logger.info(f"Received unary-unary request: {request}")

        return ExampleResponse(example_integer=1, example_string="Hello, World!")

    async def ExampleUnaryStream(
        self, request: ExampleRequest, context: Context
    ) -> AsyncIterable[ExampleResponse]:
        logger.info(f"Received unary-stream request: {request}")

        # generate async responses stream
        for i in range(5):
            logger.info(f"Sending response {i}")
            yield ExampleResponse(example_integer=i, example_string=f"Response {i}")

    async def ExampleStreamUnary(
        self, request_iterator: AsyncIterable[ExampleRequest], context: Context
    ) -> ExampleResponse:
        logger.info(f"Received stream-unary request: {request_iterator}")

        async for request in request_iterator:
            logger.info(f"Received stream-unary request: {request}")
        response = ExampleResponse(
            example_integer=1, example_string="Stream Unary Response"
        )
        return response

    async def ExampleStreamStream(
        self, request_iterator: AsyncIterable[ExampleRequest], context: Context
    ) -> AsyncIterable[ExampleResponse]:
        """Missing associated documentation comment in .proto file."""
        raise NotImplementedError("Method not implemented!")


def create_server(
    local: str,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str = "",
) -> Server:
    """
    Create a new SRPC server instance.
    """
    server = Server(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )

    return server


async def amain() -> None:
    server = create_server(
        local="agntcy/grpc/server",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
    )

    # Create RPCs
    add_TestServicer_to_server(
        TestService(),
        server,
    )

    await server.run()
```

## Troubleshooting

### Plugin Not Found

If you get an error that the plugin is not found:

- Ensure `protoc-slimrpc-plugin` is in your PATH
- Or specify the full path:
  `--plugin=protoc-gen-slimrpc=/full/path/to/protoc-slimrpc-plugin`

### Import Errors

If you encounter Python import errors:

- Make sure the generated `*_pb2.py` files are in your Python path
- Use the `types_import` parameter to customize import paths
- Ensure all Protocol Buffer dependencies are generated

### Build Errors

If the plugin fails to build:

- Ensure you have Rust and Cargo installed
- Check that all dependencies are available
- Try cleaning and rebuilding: `cargo clean && cargo build --release`

## Contributing

Please see the main repository's contributing guidelines at
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## License

This project is licensed under the Apache 2.0 License - see the
[LICENSE.md](../../LICENSE.md) file for details.

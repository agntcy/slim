# SLIMRPC Compiler

The Slim RPC Compiler is a collection of protoc plugins that generate client
stubs and server handlers for [SLIMRPC (Slim RPC)](./slim-rpc.md) from Protocol
Buffer service definitions. These plugins enable you to build high-performance
RPC services using the SLIMRPC framework.

## Supported Languages

- **Python**: `protoc-gen-slimrpc-python`
- **Go**: `protoc-gen-slimrpc-go`

## Features

The Slim RPC Compiler has the following features:

- Generates type-safe client stubs and server handlers for SLIMRPC services.
- Supports all gRPC streaming patterns: unary-unary, unary-stream, stream-unary,
  and stream-stream.
- Compatible with both `protoc` and `buf` build systems (buf recommended).
- Automatic import resolution for Protocol Buffer dependencies.

## Installation

You can install the SLIMRPC compiler either by downloading pre-built binaries or building from source using Cargo.

=== "Pre-built Binaries (Python)"

    Download the pre-built binary for your platform from the [latest release](https://github.com/agntcy/slim/releases/tag/protoc-slimrpc-plugin-v1.0.2):

    === "Linux (x86_64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-linux-x86_64.tar.gz
        tar -xzf protoc-gen-slimrpc-python-linux-x86_64.tar.gz
        chmod +x protoc-gen-slimrpc-python
        sudo mv protoc-gen-slimrpc-python /usr/local/bin/
        ```

    === "Linux (ARM64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-linux-arm64.tar.gz
        tar -xzf protoc-gen-slimrpc-python-linux-arm64.tar.gz
        chmod +x protoc-gen-slimrpc-python
        sudo mv protoc-gen-slimrpc-python /usr/local/bin/
        ```

    === "macOS (ARM64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-macos-arm64.tar.gz
        tar -xzf protoc-gen-slimrpc-python-macos-arm64.tar.gz
        chmod +x protoc-gen-slimrpc-python
        sudo mv protoc-gen-slimrpc-python /usr/local/bin/
        ```

    === "macOS (x86_64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-macos-x86_64.tar.gz
        tar -xzf protoc-gen-slimrpc-python-macos-x86_64.tar.gz
        chmod +x protoc-gen-slimrpc-python
        sudo mv protoc-gen-slimrpc-python /usr/local/bin/
        ```

    === "Windows (x86_64)"

        ```powershell
        Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-windows-x86_64.zip" -OutFile "protoc-gen-slimrpc-python-windows-x86_64.zip"
        Expand-Archive -Path protoc-gen-slimrpc-python-windows-x86_64.zip -DestinationPath .
        # Add the binary to your PATH or move it to a directory in your PATH
        ```

    === "Windows (ARM64)"

        ```powershell
        Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-python-windows-arm64.zip" -OutFile "protoc-gen-slimrpc-python-windows-arm64.zip"
        Expand-Archive -Path protoc-gen-slimrpc-python-windows-arm64.zip -DestinationPath .
        # Add the binary to your PATH or move it to a directory in your PATH
        ```

=== "Pre-built Binaries (Go)"

    Download the pre-built binary for your platform from the [latest release](https://github.com/agntcy/slim/releases/tag/protoc-slimrpc-plugin-v1.0.2):

    === "Linux (x86_64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-linux-x86_64.tar.gz
        tar -xzf protoc-gen-slimrpc-go-linux-x86_64.tar.gz
        chmod +x protoc-gen-slimrpc-go
        sudo mv protoc-gen-slimrpc-go /usr/local/bin/
        ```

    === "Linux (ARM64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-linux-arm64.tar.gz
        tar -xzf protoc-gen-slimrpc-go-linux-arm64.tar.gz
        chmod +x protoc-gen-slimrpc-go
        sudo mv protoc-gen-slimrpc-go /usr/local/bin/
        ```

    === "macOS (ARM64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-macos-arm64.tar.gz
        tar -xzf protoc-gen-slimrpc-go-macos-arm64.tar.gz
        chmod +x protoc-gen-slimrpc-go
        sudo mv protoc-gen-slimrpc-go /usr/local/bin/
        ```

    === "macOS (x86_64)"

        ```bash
        curl -LO https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-macos-x86_64.tar.gz
        tar -xzf protoc-gen-slimrpc-go-macos-x86_64.tar.gz
        chmod +x protoc-gen-slimrpc-go
        sudo mv protoc-gen-slimrpc-go /usr/local/bin/
        ```

    === "Windows (x86_64)"

        ```powershell
        Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-windows-x86_64.zip" -OutFile "protoc-gen-slimrpc-go-windows-x86_64.zip"
        Expand-Archive -Path protoc-gen-slimrpc-go-windows-x86_64.zip -DestinationPath .
        # Add the binary to your PATH or move it to a directory in your PATH
        ```

    === "Windows (ARM64)"

        ```powershell
        Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/protoc-slimrpc-plugin-v1.0.2/protoc-gen-slimrpc-go-windows-arm64.zip" -OutFile "protoc-gen-slimrpc-go-windows-arm64.zip"
        Expand-Archive -Path protoc-gen-slimrpc-go-windows-arm64.zip -DestinationPath .
        # Add the binary to your PATH or move it to a directory in your PATH
        ```

=== "Build from Source (Cargo)"

    You can build and install the plugin from source using Cargo:

    ```bash
    cargo install agntcy-protoc-slimrpc-plugin
    ```

    This will install the `protoc-slimrpc-plugin` binaries to your Cargo bin directory (usually `~/.cargo/bin`).

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

If using Go, you might need to specify your Go package as well
as shown in [the simple example](https://github.com/agntcy/slim/blob/slim-bindings-v1.1.1/data-plane/bindings/go/examples/slimrpc/simple/example.proto).

### Using with Buf (Recommended)

#### Prerequisites

- `buf` CLI [installed](https://buf.build/docs/cli/installation/)
- The appropriate `protoc-gen-slimrpc-python` or `protoc-gen-slimrpc-go` binary in your PATH, or specify the full path in the `buf.gen.yaml` file

#### Create `buf.gen.yaml`

Create a `buf.gen.yaml` file in your project root:

=== "Python"

    ```yaml
    version: v2
    managed:
      enabled: true
    inputs:
      - proto_file: example.proto
    plugins:
      # Generate slimrpc stubs
      - local: protoc-gen-slimrpc-python
        out: types
      # Generate standard protobuf code
      - remote: buf.build/protocolbuffers/python:v29.3
        out: types
      # Generate type stubs
      - remote: buf.build/protocolbuffers/pyi:v31.1
        out: types
    ```

=== "Go"

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
      - local: protoc-gen-slimrpc-go
        out: types
        opt:
          - paths=source_relative
    ```

#### Generate Code

```bash
buf generate
```

This will generate:

- **Python**: `*_pb2.py` (protobuf types), `*_pb2.pyi` (type stubs), and `*_pb2_slimrpc.py` (slimrpc stubs)
- **Go**: `*.pb.go` (protobuf types) and `*_slimrpc.pb.go` (slimrpc stubs)

#### Advanced buf Configuration (Python)

You can customize the types import for Python. For example, to use existing
types from `a2a.grpc.a2a_pb2`, you can modify the `buf.gen.yaml` as follows:

```yaml
version: v2
managed:
  enabled: true
plugins:
  - local: protoc-gen-slimrpc-python
    out: generated
    opt:
      - types_import=from a2a.grpc import a2a_pb2 as a2a__pb2
  - remote: buf.build/protocolbuffers/python
    out: generated
```

### Using with Protocol Buffer Compiler

#### Prerequisites

Make sure you have:

- `protoc` (Protocol Buffer compiler) installed
- The appropriate `protoc-gen-slimrpc-python` or `protoc-gen-slimrpc-go` binary in your PATH or specify its full path

#### Generate Files

=== "Python"

    ```bash
    # Generate both the protobuf Python files and SLIMRPC files
    protoc \
      --python_out=. \
      --pyi_out=. \
      --plugin=protoc-gen-slimrpc-python=/usr/local/bin/protoc-gen-slimrpc-python \
      --slimrpc-python_out=. \
      example.proto
    ```

    This will generate:

    - `example_pb2.py` - Standard protobuf Python bindings
    - `example_pb2_slimrpc.py` - SLIMRPC client stubs and server servicers

=== "Go"

    ```bash
    # Generate both the protobuf Go files and SLIMRPC files
    protoc \
      --go_out=. \
      --plugin=protoc-gen-slimrpc-go=/usr/local/bin/protoc-gen-slimrpc-go \
      --slimrpc-go_out=. \
      example.proto
    ```

    This will generate:

    - `example.pb.go` - Standard protobuf Go bindings
    - `example_slimrpc.pb.go` - SLIMRPC client stubs and server servicers

#### With Custom Types Import (Python)

You can specify a custom import for the types module. This allows you to import
the types from an external package.

For instance, if you don't want to generate the types and you want to import
them from `a2a.grpc.a2a_pb2`, you can do:

```bash
protoc \
  --plugin=protoc-gen-slimrpc-python=/usr/local/bin/protoc-gen-slimrpc-python \
  --slimrpc-python_out=types_import="from a2a.grpc import a2a_pb2 as a2a__pb2":. \
  example.proto
```

## Examples

Complete working examples are available in the repository:

- **Python**: [bindings/python/examples/slimrpc/simple](https://github.com/agntcy/slim/tree/slim-bindings-v1.1.1/data-plane/bindings/python/examples/slimrpc/simple)
- **Go**: [bindings/go/examples/slimrpc/simple](https://github.com/agntcy/slim/tree/slim-bindings-v1.1.1/data-plane/bindings/go/examples/slimrpc/simple)

Both examples demonstrate all four RPC patterns with comprehensive client and server implementations.

## Generated Code Structure

For detailed information about the generated code structure, including client stubs, server handlers, and registration functions for both Python and Go, please refer to the [SLIMRPC documentation](./slim-rpc.md#generated-code).

The compiler generates type-safe client stubs and server handlers that support all gRPC streaming patterns (unary-unary, unary-stream, stream-unary, and stream-stream) with language-specific idioms and async/await patterns.

## Plugin Parameters

### Python Plugin

- `types_import`: Customize how protobuf types are imported
  - Example: `types_import="from my_package import types_pb2 as pb2"`
  - Default: Uses local import based on the proto file name

### Go Plugin

- `paths`: Control output path strategy
  - `source_relative`: Generate files relative to the proto file location
  - Default: Uses Go package paths

## Troubleshooting

### Plugin Not Found

If you get an error that the plugin is not found:

- Ensure `protoc-gen-slimrpc-python` or `protoc-gen-slimrpc-go` is in your PATH
- Or specify the full path:
  - For Python: `--plugin=protoc-gen-slimrpc=/full/path/to/protoc-gen-slimrpc-python`
  - For Go: `--plugin=protoc-gen-slimrpc=/full/path/to/protoc-gen-slimrpc-go`

### Import Errors

If you encounter import errors:

- **Python**: Make sure the generated `*_pb2.py` files are in your Python path. Use the `types_import` parameter to customize import paths.
- **Go**: Ensure `go.mod` is properly configured with correct module paths.
- Verify all Protocol Buffer dependencies are generated.

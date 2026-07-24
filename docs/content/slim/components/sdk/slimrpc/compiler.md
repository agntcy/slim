# SLIMRPC Compiler

The SLIMRPC compiler is a set of `protoc` plugins that generate client and server stubs from `.proto` files for use over SLIM. It works alongside the standard protobuf code generators — you run both in a single `buf generate` (or `protoc`) invocation.

## Supported Languages

| Language | Plugin binary |
|---|---|
| Python | `protoc-gen-slimrpc-python` |
| Go | `protoc-gen-slimrpc-go` |
| Java | `protoc-gen-slimrpc-java` |
| Kotlin | `protoc-gen-slimrpc-java` (same plugin) |
| .NET (C#) | `protoc-gen-slimrpc-csharp` |

## Installation

Build the plugins from source using Cargo:

```bash
git clone https://github.com/agntcy/slim-bindings.git
cd slim-bindings
cargo build --release -p slimrpc-compiler
```

The plugin binaries are placed in `target/release/`. Add that directory to your `PATH`, or copy the binaries to a directory already on your `PATH`.

## Usage with buf (Recommended)

Add the SLIMRPC plugin to your `buf.gen.yaml` alongside the standard protobuf plugin:

=== "Python"

    ```yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - local: protoc-gen-slimrpc-python
        out: types
      - remote: buf.build/protocolbuffers/python:v29.3
        out: types
      - remote: buf.build/protocolbuffers/pyi:v31.1
        out: types
    ```

=== "Go"

    ```yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/go
        out: types
        opt:
          - paths=source_relative
      - local: protoc-gen-slimrpc-go
        out: types
        opt:
          - paths=source_relative
    ```

=== "Java"

    ```yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/java
        out: src/main/java
      - local: protoc-gen-slimrpc-java
        out: src/main/java
    ```

=== "Kotlin"

    Kotlin uses the same Java plugin. Add both to generate Java sources consumable from Kotlin:

    ```yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/java
        out: src/main/java
      - local: protoc-gen-slimrpc-java
        out: src/main/java
    ```

=== ".NET"

    ```yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/csharp
        out: Generated
        opt: base_namespace=ExampleService
      - local: protoc-gen-slimrpc-csharp
        out: Generated
        opt: base_namespace=ExampleService,types_namespace=ExampleService
    ```

Then run:

```bash
buf generate
```

## Usage with protoc

```bash
# Python
protoc --slimrpc-python_out=types --python_out=types example.proto

# Go
protoc --slimrpc-go_out=types --go_out=types example.proto

# Java
protoc --slimrpc-java_out=src/main/java --java_out=src/main/java example.proto

# C#
protoc --slimrpc-csharp_out=Generated --csharp_out=Generated example.proto
```

## Generated Code

Each language generates:

- **Client stub** — makes calls to a remote SLIMRPC server via a `Channel`
- **Server interface** — the interface you implement to handle incoming calls
- **Registration function** — registers your implementation with a `Server` instance

See the [Serving](../tutorials/slimrpc/tutorial-serve.md) and [Using](../tutorials/slimrpc/tutorial-client.md) tutorials for how to use the generated code.

## Full Reference

For all plugin options (`types_import`, `paths`, `base_namespace`, etc.) and troubleshooting, see the [SLIMRPC compiler README](https://github.com/agntcy/slim-bindings/blob/main/slimrpc-compiler/README.md) in the slim-bindings repository.

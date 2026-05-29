# SLIM Python Bindings (UniFFI)

Python bindings for SLIM (Secure Low-Latency Interactive Messaging) using UniFFI.

This provides a Python interface to the SLIM data plane, enabling secure,
low-latency messaging with support for point-to-point and group
(multicast) communication patterns.

## Overview

These Python bindings are generated from the `agntcy-slim-bindings` crate
using [UniFFI](https://mozilla.github.io/uniffi-rs/), providing a native
Python interface that wraps the high-performance Rust implementation.

### Key Features

- **Point-to-Point Messaging**: Direct communication between two endpoints
- **Group Messaging**: Multicast communication with multiple participants
- **Secure by Default**: Support for TLS, mTLS, and various authentication methods
- **MLS Encryption**: End-to-end encryption for sessions
- **Delivery Confirmation**: Optional completion handles for reliable messaging
- **Flexible Authentication**: Shared secrets, JWT, SPIRE integration
- **slimrpc Support**: Protocol Buffers RPC over SLIM - see [SLIMRPC.md](SLIMRPC.md) for details

## Architecture

The Python bindings are built using Maturin, which automatically generates
Python bindings from the Rust UniFFI adapter:

```
data-plane/bindings/
├── rust/          # Rust UniFFI bindings (shared by Go, Python, etc.)
│   ├── src/
│   │   ├── app.rs
│   │   ├── build_info.rs
│   │   ├── client_config.rs
│   │   ├── common_config.rs
│   │   ├── completion_handle.rs
│   │   ├── config.rs
│   │   ├── errors.rs
│   │   ├── identity.rs
│   │   ├── identity_config.rs
│   │   ├── init_config.rs
│   │   ├── lib.rs
│   │   ├── message_context.rs
│   │   ├── name.rs
│   │   ├── server_config.rs
│   │   ├── service.rs
│   │   └── session.rs
│   └── Cargo.toml
├── go/               # Go-specific bindings and examples
└── python/           # Python-specific bindings and examples (this directory)
    ├── examples/              # Example applications
    ├── tests/                 # Unit and integration tests
    └── Taskfile.yaml          # Build and development tasks
```

## Prerequisites

- **Rust toolchain** (1.70+)
- **Python** (3.10+)
- **uv** (Python package manager): https://docs.astral.sh/uv/
- **Task** (optional, for convenient build commands)

## Installation

### Development Build

```bash
cd data-plane/bindings/python
task python:bindings:build
```

This will:
1. Install all dependencies
2. Compile the Rust UniFFI adapter
3. Generate Python bindings using Maturin
4. Install the package in development mode

## Creating Distribution Packages

### Build Wheels for Multiple Python Versions

To create distributable wheel packages for Python 3.10, 3.11, 3.12, 3.13 and 3.14:

```bash
task python:bindings:packaging
```

Or directly with Maturin:

```bash
uv run maturin build --release -i 3.10 3.11 3.12 3.13
```

Maturin automatically:
1. Compiles the Rust UniFFI adapter library
2. Generates Python bindings from UniFFI scaffolding
3. Bundles the native library into platform-specific wheels
4. Creates wheels for each specified Python version

The resulting wheels are self-contained and ready for distribution.

#### Custom Build Options

You can customize the build with the following variables:

```bash
# Build for a specific target architecture
task python:bindings:packaging TARGET=aarch64-apple-darwin

# Build in debug mode (default is release)
task python:bindings:packaging PROFILE=debug

# Cross-compile for Linux on macOS
task python:bindings:packaging TARGET=x86_64-unknown-linux-gnu
```

#### Output Structure

After running the packaging task, you'll find:

```
dist/
├── slim_uniffi_bindings-0.7.0-cp310-*.whl  # Python 3.10 wheel
├── slim_uniffi_bindings-0.7.0-cp311-*.whl  # Python 3.11 wheel
├── slim_uniffi_bindings-0.7.0-cp312-*.whl  # Python 3.12 wheel
└── slim_uniffi_bindings-0.7.0-cp313-*.whl  # Python 3.13 wheel
```

Note: The native library is automatically bundled inside each wheel.

#### Installing from Wheel

Users can install the wheel package directly:

```bash
pip install slim_uniffi_bindings-0.7.0-cp310-*.whl
```

The native library is automatically included in the wheel and will be loaded at runtime.

## Examples

Examples are a **separate project** in the `examples/` directory.

See [examples/README.md](examples/README.md) for detailed instructions.

### Quick Start with Examples

```bash
cd examples

# View available examples
task

# Run simple example
task simple

# Run point-to-point examples
task p2p:alice    # Terminal 1
task p2p:bob      # Terminal 2
```

## Quick Start

### Simple Example

```python
import slim_uniffi_bindings as slim

# Initialize crypto provider
slim.initialize_crypto_provider()

# Get version
print(f"SLIM Version: {slim.get_version()}")

# Create an app with shared secret authentication
app_name = {
    'components': ['org', 'example', 'app'],
    'id': None
}
app = slim.create_app_with_secret(app_name, "my-secret")

print(f"App ID: {app.id()}")
print(f"App Name: {'/'.join(app.name().components)}")
```

Run the simple example:
```bash
cd examples
task simple
```

### Point-to-Point Communication

**Terminal 1 - Receiver (Alice):**
```bash
cd examples
task p2p:alice
```

**Terminal 2 - Sender (Bob):**
```bash
cd examples
task p2p:bob
```

### Group Communication

**Terminal 1 - Participant (Alice):**
```bash
cd examples
task group:participant:alice
```

**Terminal 2 - Participant (Bob):**
```bash
cd examples
task group:participant:bob
```

**Terminal 3 - Moderator:**
```bash
cd examples
task group:moderator
```

For more details, see [examples/README.md](examples/README.md).

## API Overview

### Application Creation

```python
# Create app with shared secret
app = slim.create_app_with_secret(app_name, shared_secret)

# Get app information
app_id = app.id()
app_name = app.name()
```

### Server Operations

```python
# Connect to server
client_config = {
    'endpoint': 'http://localhost:46357',
    'tls': {'insecure': True, ...}
}
conn_id = app.connect(client_config)

# Run server
server_config = {
    'endpoint': '127.0.0.1:46357',
    'tls': {'insecure': True, ...}
}
app.run_server(server_config)

# Disconnect
app.disconnect(conn_id)
```

### Session Management

```python
# Create session
session_config = {
    'session_type': 'PointToPoint',  # or 'Group'
    'enable_mls': False,
    'max_retries': 3,
    'interval_ms': 100,
    'initiator': True,
    'metadata': {}
}
session = app.create_session(session_config, destination_name)

# Listen for incoming session
session = app.listen_for_session(timeout_ms=30000)

# Delete session
app.delete_session(session)
```

### Messaging

```python
# Send message (fire-and-forget)
session.publish(data, "text/plain", metadata)

# Send with delivery confirmation
completion = session.publish_with_completion(data, "text/plain", metadata)
completion.wait()  # Block until delivered

# Receive message
msg = session.get_message(timeout_ms=5000)
print(f"Payload: {msg.payload}")
print(f"From: {msg.context.source_name}")
print(f"Type: {msg.context.payload_type}")

# Reply to message
session.publish_to(msg.context, reply_data, "text/plain", None)
```

### Group Operations

```python
# Invite participant to group
session.invite(participant_name)

# Remove participant
session.remove(participant_name)
```

## Examples

### Examples Directory Structure

```
examples/
├── common/
│   └── common.py          # Shared utilities
├── simple/
│   └── main.py            # Basic functionality demo
├── point_to_point/
│   └── main.py            # P2P messaging
└── group/
    └── main.py            # Group/multicast messaging
```

### Running Examples

All examples require a running SLIM server. Start the Go server:

```bash
cd data-plane/bindings/go
task example:server
```

Then run Python examples:

```bash
# Simple example
task example

# Point-to-point
task example:p2p:alice      # Terminal 1
task example:p2p:bob        # Terminal 2

# Group messaging
task example:group:participant:alice    # Terminal 1
task example:group:participant:bob      # Terminal 2
task example:group:moderator            # Terminal 3
```

## Testing

### Unit Tests

```bash
task test
# or
python -m pytest tests/unit_test.py -v
```

### Integration Tests

Integration tests require a running SLIM server:

```bash
# Terminal 1: Start server
cd ../go && task example:server

# Terminal 2: Run integration tests
SLIM_INTEGRATION_TEST=1 python -m pytest tests/integration_test.py -v -s
```

## Development

### Available Tasks

```bash
task                           # Show help
task build                     # Build package with Maturin
task test                      # Run tests
task python:bindings:packaging # Build wheels for multiple Python versions
task clean                     # Clean build artifacts
```

### Project Structure

- `slim_uniffi_bindings/` - Python package (bindings generated by Maturin)
- `examples/` - Example applications
- `tests/` - Unit and integration tests
- `Taskfile.yaml` - Build automation
- `pyproject.toml` - Package configuration (Maturin build system)

## Comparison with Go Bindings

Both Python and Go bindings use the same UniFFI adapter, ensuring API consistency:

| Feature | Python | Go |
|---------|--------|-----|
| Binding Generation | uniffi-bindgen | uniffi-bindgen-go |
| API Style | Pythonic (snake_case) | Idiomatic Go (PascalCase) |
| Error Handling | Exceptions | Error returns |
| Async Support | Sync wrapper over async Rust | Sync wrapper over async Rust |
| Examples | ✅ | ✅ |
| Tests | ✅ | ✅ |

## API Reference

### Core Types

- **`Name`**: Application/service identifier with components and optional ID
- **`SessionConfig`**: Configuration for creating sessions
- **`TlsConfig`**: TLS settings for secure connections
- **`ServerConfig`**: Server endpoint and TLS configuration
- **`ClientConfig`**: Client endpoint and TLS configuration
- **`MessageContext`**: Message metadata (source, destination, type, metadata)
- **`ReceivedMessage`**: Received message with context and payload

### Main Classes

- **`BindingsAdapter`**: Main app interface for session management
- **`BindingsSessionContext`**: Session interface for messaging
- **`FfiCompletionHandle`**: Completion handle for delivery confirmation

### Session Types

- **`PointToPoint`**: Direct one-to-one communication
- **`Group`**: One-to-many multicast communication

## Troubleshooting

### ImportError: Cannot find slim_uniffi_bindings

Make sure you've built the package:
```bash
task build
# or
uv run maturin develop
```

### Connection Refused

Ensure the SLIM server is running:
```bash
cd ../go && task example:server
```

### Build Errors

If you encounter build errors, try cleaning and rebuilding:
```bash
task clean
uv run maturin develop
```

## Contributing

When contributing to the Python bindings:

1. Maintain API consistency with Go bindings
2. Follow Python naming conventions (snake_case)
3. Add tests for new functionality
4. Update examples if adding features
5. Keep documentation up to date

## License

Apache-2.0 - See [LICENSE.md](../../../LICENSE.md) for details

## See Also

- [slimrpc Documentation](SLIMRPC.md) - Protocol Buffers RPC over SLIM
- [Go Bindings](../go/README.md)
- [UniFFI Adapter](../adapter/src/)
- [SLIM Documentation](../../../README.md)

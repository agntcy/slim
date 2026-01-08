# SLIM UniFFI Examples

Example applications demonstrating the SLIM UniFFI Python bindings with async/await support.

## Overview

This package provides working examples of:
- **Point-to-Point Messaging**: Direct communication between two applications
- **Group Messaging**: Multi-participant communication via channels
- **Server**: Running a SLIM server instance

All examples use the async methods from `slim_uniffi_bindings` to demonstrate asynchronous programming patterns.

## Prerequisites

1. **Parent package must be built first:**
   ```bash
   cd ..  # Go to parent directory (data-plane/bindings/python)
   uv run maturin develop
   # OR
   task build
   ```

2. **Install the examples package:**
   ```bash
   cd examples
   uv sync
   ```
   This installs the package in development mode and makes the entry points available.

3. **SLIM server must be running** (for P2P and group examples):
   ```bash
   # From data-plane/bindings/go
   task example:server
   # OR run the Python server:
   task python:example:server
   ```

## Quick Start

### Setup

First time setup:
```bash
# Build parent package with Maturin
cd /Users/smagyari/dev/genai/slim/data-plane/bindings/python
uv run maturin develop

# Install examples package
cd examples
uv sync
```

### Run Server

```bash
task python:example:server
```

### Point-to-Point Communication

**Terminal 1 - Alice (Receiver):**
```bash
task python:example:p2p:alice
```

**Terminal 2 - Bob (Sender, no MLS):**
```bash
task python:example:p2p:no-mls:bob
```

**Terminal 2 - Bob (Sender, with MLS):**
```bash
task python:example:p2p:mls:bob
```

### Group Communication

**Terminal 1 - Participant (client-1):**
```bash
task python:example:group:client-1
```

**Terminal 2 - Participant (client-2):**
```bash
task python:example:group:client-2
```

**Terminal 3 - Moderator:**
```bash
task python:example:group:moderator
```

## Available Tasks

View all available tasks:
```bash
task --list
```

## Project Structure

```
examples/
├── src/
│   └── slim_bindings_examples/
│       ├── __init__.py          # Main entry point and CLI dispatcher
│       ├── common.py            # Shared utilities and helpers
│       ├── point_to_point.py   # P2P messaging example
│       ├── group.py             # Group messaging example
│       └── slim.py              # Server example
├── pyproject.toml               # Project configuration
├── Taskfile.yaml                # Task runner configuration
└── Dockerfile                   # Container build configuration
```

## Key Differences from slim_bindings

This package uses `slim_uniffi_bindings` with async methods:

| Operation | slim_bindings | slim_uniffi_bindings |
|-----------|--------------|---------------------|
| Import | `import slim_bindings` | `import slim_uniffi_bindings.generated.slim_bindings as slim` |
| Create app | `slim_bindings.Slim(name, provider, verifier)` | `slim.create_app_with_secret(name, secret)` |
| Connect | `await app.connect(dict)` | `await app.connect_async(ClientConfig)` |
| Create session | `await app.create_session(name, config)` | `await app.create_session_async(config, name)` |
| Publish | `await session.publish(bytes)` | `await session.publish_async(bytes, type, metadata)` |
| Receive | `await session.get_message()` | `await session.get_message_async(timeout_ms)` |

## Configuration

### Shared Secret

The default shared secret is defined in `Taskfile.yaml`:
```bash
SHARED_SECRET="jasfhuejasdfhays3wtkrktasdhfsadu2rtkdhsfgeht"
```

For production use, always use proper authentication mechanisms.

### Connection Parameters

Default connection to local server:
```json
{
  "endpoint": "http://localhost:46357",
  "tls": {"insecure": true}
}
```

## Command Line Interface

Each example can be run directly:

```bash
# Using uv
uv run --package slim-uniffi-examples p2p --help
uv run --package slim-uniffi-examples group --help
uv run --package slim-uniffi-examples slim --help

# Or after installation
slim-p2p --help
slim-group --help
slim-server --help
```

## Development

### Building the Package

```bash
uv build
```

### Installing Locally

```bash
uv pip install -e .
```

## Docker

Build and run examples in a container:

```bash
# Build image
docker build -t slim-uniffi-examples -f Dockerfile ../../..

# Run container
docker run slim-uniffi-examples p2p --help
```

## Troubleshooting

### "Could not import slim_uniffi_bindings"

Make sure the parent package is built:
```bash
cd .. && task python:bindings:packaging
```

### "Connection refused"

Start the SLIM server:
```bash
task python:example:server
# or from Go examples:
cd ../../go && task example:server
```

### Import Errors

Ensure you're running with `uv` or the package is installed:
```bash
uv run --package slim-uniffi-examples <command>
```

## License

Apache-2.0 - See [LICENSE.md](../../../../LICENSE.md) for details

## See Also

- [Parent Package README](../README.md)
- [Go Examples](../../go/examples/)
- [Original slim_bindings Examples](../../../python/bindings/examples/)

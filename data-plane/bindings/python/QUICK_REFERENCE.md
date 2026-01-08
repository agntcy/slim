# SLIM Python Bindings - Quick Reference

## Installation & Setup

```bash
cd data-plane/bindings/python

# Install and build with Maturin
uv sync
uv run maturin develop

# Or use task
task build
```

## Basic Usage

```python
import slim_uniffi_bindings as slim

# Initialize
slim.initialize_crypto_provider()

# Create app
app_name = {'components': ['org', 'app', 'v1'], 'id': None}
app = slim.create_app_with_secret(app_name, "secret")

# Connect to server
config = {
    'endpoint': 'http://localhost:46357',
    'tls': {'insecure': True, ...}
}
conn_id = app.connect(config)

# Create session
session_config = {
    'session_type': 'PointToPoint',
    'enable_mls': False,
    'initiator': True,
    ...
}
destination = {'components': ['org', 'dest', 'v1'], 'id': None}
session = app.create_session(session_config, destination)

# Send message
session.publish(b"Hello", "text/plain", None)

# Receive message
msg = session.get_message(timeout_ms=5000)

# Cleanup
app.delete_session(session)
app.disconnect(conn_id)
```

## Task Commands

```bash
task                                     # Show help
task build                               # Build with Maturin
task test                                # Run tests
task python:bindings:packaging           # Build wheels for distribution
task clean                               # Clean build artifacts
```

## API Quick Reference

### App Management
```python
app = slim.create_app_with_secret(name, secret)
app_id = app.id()
app_name = app.name()
```

### Connectivity
```python
conn_id = app.connect(client_config)
app.run_server(server_config)
app.disconnect(conn_id)
```

### Sessions
```python
session = app.create_session(config, destination)
session = app.listen_for_session(timeout_ms)
app.delete_session(session)
```

### Messaging
```python
session.publish(data, type, metadata)
completion = session.publish_with_completion(data, type, metadata)
session.publish_to(msg_context, data, type, metadata)
msg = session.get_message(timeout_ms)
```

### Group Operations
```python
session.invite(participant_name)
session.remove(participant_name)
```

### Session Info
```python
dest = session.destination()
src = session.source()
session_id = session.session_id()
session_type = session.session_type()
is_initiator = session.is_initiator()
```

## Directory Structure

```
data-plane/bindings/python/
├── README.md                    # Main documentation
├── QUICK_REFERENCE.md           # This file
├── PACKAGING_GUIDE.md           # Packaging documentation
├── Taskfile.yaml                # Build automation
├── pyproject.toml               # Python project config (Maturin)
├── .gitignore                   # Git ignore rules
├── slim_uniffi_bindings/        # Python package
│   ├── __init__.py              # Package entry point
│   └── py.typed                 # Type hints marker
├── examples/
│   ├── README.md
│   ├── pyproject.toml
│   └── src/
│       └── slim_bindings_examples/
│           ├── __init__.py
│           ├── common.py        # Shared utilities
│           ├── slim.py          # Simple example
│           ├── point_to_point.py
│           └── group.py
└── tests/
    ├── unit_test.py             # Unit tests
    └── integration_test.py      # Integration tests
```

## Common Patterns

### Error Handling
```python
try:
    session = app.create_session(config, dest)
except Exception as e:
    print(f"Failed to create session: {e}")
```

### Timeout Handling
```python
try:
    msg = session.get_message(timeout_ms=5000)
except Exception as e:
    if "timeout" in str(e).lower():
        print("No message received")
```

### Reply Pattern
```python
# Receive message
msg = session.get_message(timeout_ms=5000)

# Send reply
reply = b"ACK"
session.publish_to(msg.context, reply, "text/plain", None)
```

### Delivery Confirmation
```python
completion = session.publish_with_completion(data, "text/plain", None)
completion.wait()  # Blocks until delivered
```

## Testing

```bash
# Unit tests
task test:unit

# Integration tests (requires running server)
task test:integration

# All tests
task test

# With coverage
task python:bindings:test:coverage
```

## Building Wheels

```bash
# Build wheels for all Python versions
task python:bindings:packaging

# Build for specific target
task python:bindings:packaging TARGET=aarch64-apple-darwin

# Build in debug mode
task python:bindings:packaging PROFILE=debug

# Direct Maturin command
uv run maturin build --release -i 3.10 3.11 3.12 3.13
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| ImportError | Run `task build` or `uv run maturin develop` |
| Connection refused | Start server in another terminal |
| Build errors | Run `task clean` then `task build` |
| Missing dependencies | Run `uv sync` |

## Examples Summary

| Example | Purpose | Command |
|---------|---------|---------|
| simple | Basic API demo | `cd examples && task simple` |
| point_to_point | P2P messaging | `cd examples && task p2p:alice` (Terminal 1)<br>`cd examples && task p2p:bob` (Terminal 2) |
| group | Multicast messaging | `cd examples && task group:participant:alice` (Terminal 1)<br>`cd examples && task group:participant:bob` (Terminal 2)<br>`cd examples && task group:moderator` (Terminal 3) |

## Configuration Examples

### Insecure TLS (Testing)
```python
tls_config = {
    'insecure': True,
    'insecure_skip_verify': None,
    'cert_file': None,
    'key_file': None,
    'ca_file': None,
    'tls_version': None,
    'include_system_ca_certs_pool': None
}
```

### Secure TLS (Production)
```python
tls_config = {
    'insecure': False,
    'insecure_skip_verify': False,
    'cert_file': '/path/to/cert.pem',
    'key_file': '/path/to/key.pem',
    'ca_file': '/path/to/ca.pem',
    'tls_version': 'tls1.3',
    'include_system_ca_certs_pool': True
}
```

### Point-to-Point Session
```python
config = {
    'session_type': 'PointToPoint',
    'enable_mls': False,
    'max_retries': 3,
    'interval_ms': 100,
    'initiator': True,
    'metadata': {}
}
```

### Group Session
```python
config = {
    'session_type': 'Group',
    'enable_mls': False,
    'max_retries': 3,
    'interval_ms': 100,
    'initiator': True,
    'metadata': {'room': 'chat-room-1'}
}
```

## Development Workflow

### Quick iteration
```bash
# Make changes to Rust code
# Rebuild and test
uv run maturin develop && uv run pytest tests/unit_test.py -v
```

### Release build
```bash
# Build with optimizations
uv run maturin develop --release
```

### Clean rebuild
```bash
task clean
task build
```

## See Also

- [Full README](README.md)
- [Packaging Guide](PACKAGING_GUIDE.md)
- [Example Documentation](examples/README.md)
- [Maturin Documentation](https://www.maturin.rs/)


# SLIM Python Bindings - Quick Reference

## Installation & Setup

```bash
cd data-plane/bindings/python

# 1. Build Rust library
task rust-build

# 2. Install uniffi-bindgen
uv tool install uniffi-bindgen==0.28.3

# 3. Generate bindings
task generate

# 4. Install dependencies (optional)
uv sync
```

## Basic Usage

```python
import sys
sys.path.insert(0, 'generated')
import slim_bindings as slim

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
task                              # Show help
task rust-build                   # Build Rust library
task generate                     # Generate Python bindings
task test                         # Run tests
task example                      # Run simple example
task example:p2p:alice            # Run P2P receiver
task example:p2p:bob              # Run P2P sender
task example:group:moderator      # Run group moderator
task example:group:participant:alice  # Run group participant
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
├── README.md                # Main documentation
├── QUICK_REFERENCE.md       # This file
├── Taskfile.yaml            # Build automation
├── pyproject.toml           # Python project config
├── requirements-test.txt    # Test dependencies
├── .gitignore               # Git ignore rules
├── generated/               # Auto-generated bindings (gitignored)
├── examples/
│   ├── README.md
│   ├── common/
│   │   ├── __init__.py
│   │   └── common.py        # Shared utilities
│   ├── simple/
│   │   ├── README.md
│   │   └── main.py
│   ├── point_to_point/
│   │   ├── README.md
│   │   └── main.py
│   └── group/
│       ├── README.md
│       └── main.py
└── tests/
    ├── unit_test.py         # Unit tests
    └── integration_test.py  # Integration tests
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
uv run pytest tests/unit_test.py -v

# Integration tests (requires running server)
SLIM_INTEGRATION_TEST=1 uv run pytest tests/integration_test.py -v -s
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| ImportError | Run `task generate` |
| Connection refused | Start server: `cd ../go && task example:server` |
| Library not found | Use `task` commands (handle library path automatically) |
| Timeout | Check server is running and names match |
| uniffi-bindgen not found | Install with: `uv tool install uniffi-bindgen==0.28.3` |

## Examples Summary

| Example | Purpose | Terminals |
|---------|---------|-----------|
| simple | Basic API demo | 1 |
| point_to_point | P2P messaging | 2 (Alice + Bob) |
| group | Multicast messaging | 3 (Alice + Bob + Moderator) |

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

## See Also

- [Full README](README.md)
- [Go Bindings](../go/README.md)
- [Example Documentation](examples/README.md)


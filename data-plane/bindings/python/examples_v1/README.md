# Python Examples - SLIM UniFFI Bindings

This directory contains example applications demonstrating the SLIM UniFFI Python bindings.

## Project Structure

This is a **separate Python project** with its own:
- `pyproject.toml` - Project configuration and dependencies
- `Taskfile.yaml` - Task runner for examples
- Independent from the parent bindings package

## Prerequisites

1. **Parent package must be built first:**
   ```bash
   cd ..  # Go to parent directory
   task build
   ```

2. **SLIM server must be running** (for P2P and group examples):
   ```bash
   cd ../../../go
   task example:server
   ```

## Quick Start

### View Available Tasks

```bash
task
# or
task help
```

### Setup

The examples depend on the parent `slim-uniffi-bindings` package. First-time setup:

```bash
task setup
```

This will build the parent package and set up the examples environment.

## Running Examples

### Simple Example
Basic demonstration of SLIM functionality:

```bash
task simple
```

### Point-to-Point Communication

**Terminal 1 - Alice (Receiver):**
```bash
task p2p:alice
```

**Terminal 2 - Bob (Sender):**
```bash
task p2p:bob
```

### Group Communication

**Terminal 1 - Participant Alice:**
```bash
task group:participant:alice
```

**Terminal 2 - Participant Bob:**
```bash
task group:participant:bob
```

**Terminal 3 - Moderator:**
```bash
task group:moderator
```

## Custom Arguments

You can pass custom arguments to examples:

```bash
# Custom server endpoint
task p2p:alice CLI_ARGS="--server=http://192.168.1.100:46357"

# Custom message
task p2p:bob CLI_ARGS="--message='Custom message' --iterations=10"
```

## Examples Overview

### 1. Simple (`simple/`)
- Initializes SLIM
- Creates an app
- Shows version info
- Demonstrates basic API usage

### 2. Point-to-Point (`point_to_point/`)
- Direct communication between two apps
- Message sending with delivery confirmation
- Request-response pattern
- Proper session management

### 3. Group/Multicast (`group/`)
- Group session creation
- Participant invitation
- Broadcasting to multiple members
- Concurrent message handling

## Common Module

The `common/` directory provides shared utilities:

```python
from common import (
    parse_name,
    format_bytes,
    create_tls_config,
    create_server_config,
    create_client_config,
    create_session_config,
)
```

See `common/common.py` for available helper functions.

## Troubleshooting

### "Could not import slim_uniffi_bindings"
Make sure the parent package is built:
```bash
cd .. && task build
```

### "Connection refused"
Start the SLIM server:
```bash
cd ../../../go && task example:server
```

### Shared secret errors
The examples use a demo shared secret defined in `Taskfile.yaml`. For custom secrets, pass via CLI_ARGS:
```bash
task p2p:alice CLI_ARGS="--secret=your-secret-here"
```

## Development

### Project Dependencies

The examples project depends on:
```toml
dependencies = [
    "slim-uniffi-bindings>=0.7.0",
]
```

This is automatically satisfied when you build the parent package.


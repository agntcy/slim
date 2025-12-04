# SLIM Go Bindings - Examples

This directory contains Go examples demonstrating SLIM's messaging capabilities.

## Prerequisites

Before running any examples, build the Rust library and generate Go bindings:

```bash
cd data-plane/bindings/go
task rust-build    # Build Rust core library (release mode)
task generate      # Generate Go bindings from compiled library
```

## Examples

### 1. Simple Example

Demonstrates basic SLIM operations: app creation, session management, and publishing.

```bash
task example
```

### 2. Point-to-Point Example

Demonstrates 1:1 messaging between two peers with request/reply pattern.

**Terminal 1 - Server:**
```bash
task example:server
```

**Terminal 2 - Alice (Receiver):**
```bash
task example:p2p:alice
```

**Terminal 3 - Bob (Sender):**
```bash
task example:p2p:bob
```

### 3. Group Example

Demonstrates group messaging with moderator-participant pattern and concurrent message handling.

**Terminal 1 - Server:**
```bash
task example:server
```

**Terminal 2 - Moderator:**
```bash
task example:group:moderator
```

**Terminal 3 - Alice (Participant):**
```bash
task example:group:participant:alice
```

**Terminal 4 - Bob (Participant):**
```bash
task example:group:participant:bob
```

## Available Tasks

View all available tasks:
```bash
task help
```

## Example Structure

```
examples/
├── common/          # Shared utilities (ID parsing)
├── simple/          # Basic SLIM usage
├── point_to_point/  # 1:1 messaging
└── group/           # Group messaging
```

## Customizing Examples

All examples support custom parameters via `CLI_ARGS`:

```bash
# Custom point-to-point IDs
task example:p2p:bob CLI_ARGS="--local=org/sender/app --remote=org/receiver/app"

# Custom group channel
task example:group:moderator CLI_ARGS="--remote=org/company/meeting"

# Custom server endpoint
task example:server CLI_ARGS="--endpoint=http://0.0.0.0:46357"
```

## Troubleshooting

**"library not found" errors**: Run `task rust-build` first  
**"undefined symbols" errors**: Run `task generate` to regenerate bindings  
**Connection failures**: Ensure the server is running with `task example:server`


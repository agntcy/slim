# SLIM React Native - Point-to-Point Example

Demonstrates 1:1 messaging between two peers (Alice and Bob) using SLIM.

## Overview

This example shows:
- Creating two separate apps (Alice and Bob)
- Establishing point-to-point sessions
- Sending and receiving messages
- Request/reply pattern

## Running the Example

This example requires a running SLIM server.

### Terminal 1 - Start SLIM Server

```bash
# From data-plane/bindings/go
task example:server
```

### Terminal 2 - Run Alice (Receiver)

```bash
npx react-native run-ios -- --simulator="iPhone 15 Pro"
# Then select "Alice" mode in the app
```

### Terminal 3 - Run Bob (Sender)

```bash
npx react-native run-ios -- --simulator="iPhone 15 Pro Max"
# Then select "Bob" mode in the app
```

## Architecture

```
┌─────────┐                    ┌──────────────┐                   ┌─────────┐
│  Alice  │◄──────────────────►│ SLIM  Server │◄─────────────────►│   Bob   │
│(Receiver)│    Point-to-Point  │              │   Point-to-Point  │(Sender) │
└─────────┘      Session        └──────────────┘     Session       └─────────┘
```

## Code Structure

See [README.md](../simple/README.md) for full code examples.

Key differences from simple example:
- Two app instances
- Bidirectional messaging
- Message reception callbacks
- Concurrent operations

## Expected Output

**Alice (Receiver)**:
```
📱 Alice waiting for messages...
📥 Received from Bob: "Hello Alice!"
📤 Sending reply: "Hello Bob!"
```

**Bob (Sender)**:
```
📱 Bob sending message...
📤 Sent to Alice: "Hello Alice!"
📥 Received reply: "Hello Bob!"
```

## Related Examples

- [Simple Example](../simple/) - Basic usage
- [Group Example](../group/) - Multi-party messaging

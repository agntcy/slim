# SLIM Node.js Examples

Node.js examples demonstrating SLIM's point-to-point messaging capabilities.

## Overview

This directory contains Node.js implementations of SLIM examples, ported from the Go bindings.

### What Works

- ✅ **Server**: Full working Node.js server implementation
- ✅ **Receiver (Alice)**: Can connect, subscribe, and receive messages
- ⚠️  **Sender/Reply**: Currently blocked by `uniffi-bindgen-node` limitations

## Setup

```bash
# Install dependencies
npm install

# Note: This will also trigger binding generation if not already done
```

## Running the Examples

### 1. Start the Node.js Server

```bash
npm run server
```

The server will start on `http://localhost:46357`.

Command line options:
```bash
npm run server -- --endpoint 0.0.0.0:46357
npm run server -- --help
```

### 2. Start Alice (Receiver)

In a second terminal:

```bash
npm run alice
```

Alice will:
- Connect to the server
- Subscribe to incoming sessions
- Wait for messages
- Log received messages (replies are blocked by tooling limitation)

Command line options:
```bash
npm run alice -- --local org/alice/app --server http://localhost:46357
npm run alice -- --secret your-32-char-secret-here
npm run alice -- --help
```

### 3. Start Bob (Sender) from Go

Since message sending is currently blocked in Node.js, use the Go implementation:

```bash
cd ../../go
go run examples/point_to_point/main.go
```

## Expected Behavior

### Server Output

```
🚀 SLIM Server
==============
Endpoint: 0.0.0.0:46357

🌐 Starting server on 0.0.0.0:46357...
   Waiting for clients to connect...

✅ Server running and listening

📡 Clients can now connect

Press Ctrl+C to stop
```

### Alice Output

```
👩 SLIM Point-to-Point Alice (Receiver)
=======================================
Local ID: org/alice/app
Server: http://localhost:46357

[<app-id>] ✅ Created app
[<app-id>] 🔌 Connected to http://localhost:46357 (conn ID: 1)
[<app-id>] ✅ Subscribed to sessions
[<app-id>] 👂 Waiting for incoming sessions...

[<app-id>] 🎉 New session established! (ID: <session-id>)
[<app-id>] 📨 Received: Hello SLIM
[<app-id>] ⚠️  Reply not sent (uniffi-bindgen-node limitation)
```

### Bob Output (Go)

```
👨 SLIM Point-to-Point Bob (Sender)
====================================
Local ID: org/bob/app
Remote ID: org/alice/app
Server: http://localhost:46357

[<app-id>] ✅ Created app
[<app-id>] 🔌 Connected to http://localhost:46357 (conn ID: 1)
[<app-id>] 📍 Route set to org/alice/app via connection 1
[<app-id>] 🔍 Creating session to org/alice/app...
[<app-id>] 📡 Session created
[<app-id>] 📤 Sent message 'Hello SLIM' - 1/5
[<app-id>] ⏱️  No reply for message 1/5: receive timeout
```

Bob will not receive replies from Alice due to the Node.js tooling limitation.

## Known Limitations

### uniffi-bindgen-node Issues

The `uniffi-bindgen-node` tool has bugs that prevent message sending:

- **Error**: `Object property 'capacity' type mismatch`
- **Root cause**: Incompatibility between `uniffi-bindgen-react-native` types and `ffi-rs`
- **Impact**: Cannot send messages or replies from Node.js
- **Workaround**: Use Go or Python bindings for sending

### What This Means

- ✅ Node.js can run servers
- ✅ Node.js can receive messages
- ❌ Node.js cannot send messages or replies
- ❌ Node.js cannot initiate sessions

For full bidirectional communication, use:
- Go bindings (`../go`)
- Python bindings (`../python`)

## Files

- `common.ts` - Shared utilities (splitId, createAndConnectApp, logging)
- `server.ts` - SLIM server implementation
- `point-to-point-alice.ts` - Message receiver (Alice)
- `package.json` - Dependencies and scripts
- `tsconfig.json` - TypeScript configuration

## Configuration

Default values (matching Go examples):

- Server endpoint: `0.0.0.0:46357`
- Client endpoint: `http://localhost:46357`
- Shared secret: `demo-shared-secret-min-32-chars!!`
- Alice ID: `org/alice/app`
- Bob ID: `org/bob/app` (use Go implementation)

## Troubleshooting

### "Cannot find module '../generated/slim-bindings-node.js'"

Run binding generation first:
```bash
cd ..
npm run build   # or: task generate
```

### "FfiConverterBytes is not defined"

This should be automatically patched during generation. If you see this error, the patch failed. Manually run:
```bash
cd ../generated
sed -i.bak 's/FfiConverterBytes/FfiConverterArrayBuffer/g' slim-bindings-node.ts
```

### "Cannot convert X to a BigInt"

This is fixed in the examples with explicit `BigInt()` conversions. If you see this, check that you're using `BigInt(value)` for connection IDs.

## Related Examples

- **Go**: `../../go/examples/` - Full working examples (server, Alice, Bob)
- **Python**: `../../python/examples/` - Full working examples
- **React Native**: `../../react-native/examples/` - Mobile receiver example

## Resources

- [uniffi-bindgen-node](https://github.com/livekit/uniffi-bindgen-node)
- [Node.js Bindings README](../README.md)
- [Go Examples](../../go/examples/README.md)

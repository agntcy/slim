# Point-to-Point Example

Demonstrates direct communication between two SLIM applications.

## What It Does

This example shows:
- Creating sender (Bob) and receiver (Alice) applications
- Connecting to a SLIM server
- Establishing point-to-point sessions
- Sending messages with delivery confirmation
- Receiving messages and sending acknowledgments
- Proper cleanup and disconnection

## Prerequisites

A running SLIM server is required. Start it with:

```bash
cd data-plane/bindings/go
task example:server
```

## Running

### Option 1: Using Task (Recommended)

**Terminal 1 - Alice (Receiver):**
```bash
cd data-plane/bindings/python
task example:p2p:alice
```

**Terminal 2 - Bob (Sender):**
```bash
task example:p2p:bob
```

### Option 2: Direct Python Execution

**Terminal 1 - Alice:**
```bash
cd examples/point_to_point
export DYLD_LIBRARY_PATH=../../../../target/release:$DYLD_LIBRARY_PATH
python main.py --local="org/alice/app" --server="http://localhost:46357"
```

**Terminal 2 - Bob:**
```bash
cd examples/point_to_point
export DYLD_LIBRARY_PATH=../../../../target/release:$DYLD_LIBRARY_PATH
python main.py --local="org/bob/app" --remote="org/alice/app" \
               --message="Hello SLIM" --iterations=5 --server="http://localhost:46357"
```

## Command-Line Arguments

```
--local TEXT       Local app name (required, e.g., "org/alice/app")
--remote TEXT      Remote app name for sender mode (e.g., "org/bob/app")
--server TEXT      SLIM server endpoint (default: "http://localhost:46357")
--message TEXT     Message to send (default: "Hello SLIM")
--iterations INT   Number of messages to send (default: 5)
--secret TEXT      Shared secret for authentication (default: "demo-secret")
```

## How It Works

### Receiver Mode (No --remote specified)
1. Creates SLIM app
2. Connects to server
3. Listens for incoming session
4. Receives messages
5. Sends ACK replies

### Sender Mode (--remote specified)
1. Creates SLIM app
2. Connects to server
3. Creates session to receiver
4. Sends messages with delivery confirmation
5. Waits for ACK replies
6. Deletes session when done

## Example Output

### Alice (Receiver)
```
============================================================
SLIM Point-to-Point Example - RECEIVER (Alice)
============================================================

Creating SLIM app: org/alice/app
✅ App created (ID: 123...)

Connecting to server: http://localhost:46357
✅ Connected (Connection ID: 1)

👂 Listening for incoming session...
✅ Session established!
   Source: org/alice/app
   Destination: org/bob/app
   Session ID: 42
   Is Initiator: False

📨 Receiving messages...

📬 Message #1 received:
   From: org/bob/app
   Payload Type: text/plain
   Payload: Hello SLIM #1
   Connection: 1
   Metadata:
     index: 1
     timestamp: 1234567890.123
   ✅ Sent acknowledgment
```

### Bob (Sender)
```
============================================================
SLIM Point-to-Point Example - SENDER (Bob)
============================================================

Creating SLIM app: org/bob/app
✅ App created (ID: 456...)

Connecting to server: http://localhost:46357
✅ Connected (Connection ID: 1)

🔧 Creating session to: org/alice/app
✅ Session created!
   Session ID: 42
   Is Initiator: True

📤 Sending 5 messages...
   📨 Sent message #1: Hello SLIM #1
      ✅ Delivery confirmed
      📬 Received ACK: ACK #1
```

## Key Concepts

### Session Establishment
- **Initiator** creates session with `create_session()`
- **Responder** receives session with `listen_for_session()`
- Session waits for handshake completion automatically

### Message Flow
1. Bob calls `publish_with_completion()` - returns completion handle
2. `completion.wait()` blocks until delivery confirmed
3. Alice receives with `get_message()`
4. Alice replies with `publish_to(msg.context, ...)`

### Delivery Confirmation
```python
# Fire-and-forget
session.publish(data, "text/plain", None)

# With confirmation
completion = session.publish_with_completion(data, "text/plain", None)
completion.wait()  # Blocks until delivered
```

## Troubleshooting

### "Connection refused"
Make sure the SLIM server is running on the specified endpoint.

### "Timeout waiting for session"
Ensure both applications are using the same server and secret.

### Library not found
Make sure `DYLD_LIBRARY_PATH` (macOS) or `LD_LIBRARY_PATH` (Linux) includes the Rust library path. Using `task` commands handles this automatically.


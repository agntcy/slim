# Group (Multicast) Example

Demonstrates group communication with multiple participants using SLIM.

## What It Does

This example shows:
- Creating a group (multicast) session
- Inviting multiple participants to a group
- Broadcasting messages to all group members
- Receiving messages from multiple sources
- Concurrent message handling

## Prerequisites

A running SLIM server is required. Start it with:

```bash
cd data-plane/bindings/go
task example:server
```

## Running

### Option 1: Using Task (Recommended)

**Terminal 1 - Alice (Participant):**
```bash
cd data-plane/bindings/python
task example:group:participant:alice
```

**Terminal 2 - Bob (Participant):**
```bash
task example:group:participant:bob
```

**Terminal 3 - Moderator:**
```bash
task example:group:moderator
```

### Option 2: Direct Python Execution

**Terminal 1 - Alice:**
```bash
cd examples/group
export DYLD_LIBRARY_PATH=../../../../target/release:$DYLD_LIBRARY_PATH
python main.py --local="org/alice/app" --server="http://localhost:46357"
```

**Terminal 2 - Bob:**
```bash
cd examples/group
export DYLD_LIBRARY_PATH=../../../../target/release:$DYLD_LIBRARY_PATH
python main.py --local="org/bob/app" --server="http://localhost:46357"
```

**Terminal 3 - Moderator:**
```bash
cd examples/group
export DYLD_LIBRARY_PATH=../../../../target/release:$DYLD_LIBRARY_PATH
python main.py --local="org/moderator/app" --remote="org/default/chat-room" \
               --invites="org/alice/app,org/bob/app" --server="http://localhost:46357"
```

## Command-Line Arguments

```
--local TEXT     Local app name (required, e.g., "org/alice/app")
--remote TEXT    Group session name (e.g., "org/default/chat-room")
--invites TEXT   Comma-separated participant names to invite
--server TEXT    SLIM server endpoint (default: "http://localhost:46357")
--secret TEXT    Shared secret for authentication (default: "demo-secret")
```

## How It Works

### Participant Mode (No --remote/--invites)
1. Creates SLIM app
2. Connects to server
3. Waits for group invitation
4. Joins group session
5. Receives messages in background thread
6. Sends periodic messages to group

### Moderator Mode (--remote and --invites specified)
1. Creates SLIM app
2. Connects to server
3. Creates group session
4. Invites each participant
5. Broadcasts welcome message
6. Sends group messages
7. Receives messages from participants

## Example Output

### Alice (Participant)
```
============================================================
SLIM Group Example - PARTICIPANT (Alice)
============================================================

Creating SLIM app: org/alice/app
✅ App created (ID: 123...)

Connecting to server: http://localhost:46357
✅ Connected (Connection ID: 1)

⏳ Waiting for group invitation...
✅ Joined group session!
   Source: org/alice/app
   Destination: org/default/chat-room
   Session ID: 42
   Session Type: Group
   Is Initiator: False

📤 Sending messages to group...
   📨 Sent: Hello from alice #1

📬 Message #1 received:
   From: org/moderator/app
   Payload Type: text/plain
   Payload: Welcome to the group chat!
   Metadata:
     type: welcome
```

### Moderator
```
============================================================
SLIM Group Example - MODERATOR
============================================================

Creating SLIM app: org/moderator/app
✅ App created (ID: 789...)

Connecting to server: http://localhost:46357
✅ Connected (Connection ID: 1)

🔧 Creating group session: org/default/chat-room
✅ Group session created!
   Session ID: 42
   Session Type: Group

👥 Inviting participants...
   Inviting: org/alice/app
      ✅ Invited successfully
   Inviting: org/bob/app
      ✅ Invited successfully

📤 Broadcasting messages to group...
   📨 Sent welcome message
   📨 Sent: Moderator message #1

📨 Receiving messages from participants...

📬 Message #1 received:
   From: org/alice/app
   Payload: Hello from alice #1
```

## Key Concepts

### Group Session Types
- **Moderator**: Creates group and manages invitations
- **Participant**: Receives invitation and joins group
- Messages broadcast to all participants

### Invitation Flow
1. Moderator creates group session with `session_type='Group'`
2. Moderator invites participants: `session.invite(participant_name)`
3. Participants listen: `app.listen_for_session()`
4. Participants automatically join when invited

### Broadcasting
```python
# Send to all group members
session.publish(data, "text/plain", metadata)
```

### Concurrent Receiving
The participant example uses a background thread to continuously receive messages while also sending periodic messages:

```python
def receive_messages():
    while not stop_receiving.is_set():
        msg = session.get_message(timeout_ms=5000)
        process(msg)

receiver_thread = threading.Thread(target=receive_messages, daemon=True)
receiver_thread.start()
```

## Architecture

```
Moderator (org/moderator/app)
    |
    |-- Creates Group: org/default/chat-room
    |
    |-- Invites --> Alice (org/alice/app)
    |-- Invites --> Bob (org/bob/app)
    |
    |-- Broadcasts --> All Participants
    |
    <-- Receives <-- All Participants
```

## Troubleshooting

### "Timeout waiting for invitation"
Make sure:
1. Server is running
2. Moderator has started and sent invitations
3. All apps use the same shared secret
4. Names match exactly in invitations

### Messages not received
- Check that all participants are connected before moderator sends
- Verify the group session name matches
- Ensure participants are properly invited

### Thread issues
The participant mode uses threading for concurrent message handling. If you see threading errors, ensure Python's threading module is available.


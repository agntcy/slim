# SLIM Java Examples

This directory contains Java examples demonstrating the SLIM Java bindings with `CompletableFuture` async patterns.

## Prerequisites

- Java 21 or higher
- Maven 3.8+
- SLIM Java bindings installed to local Maven repository

## Building

```bash
# From examples/ directory
task compile

# Or using Maven directly
mvn clean compile
```

## Examples

### 1. Server Example

Starts a SLIM server that relays messages between clients.

**Run:**
```bash
task server

# With custom address
task server CLI_ARGS="--slim 0.0.0.0:46357"

# With OpenTelemetry tracing
task server CLI_ARGS="--enable-opentelemetry"
```

**Options:**
- `--slim <address>`: Server address (default: `127.0.0.1:46357`)
- `--enable-opentelemetry`: Enable OpenTelemetry tracing to `localhost:4317`

**What it does:**
- Initializes the SLIM service
- Starts server on specified address
- Waits for Ctrl+C to shutdown gracefully

---

### 2. Point-to-Point Example

Demonstrates direct messaging between two peers.

**Run Receiver (Alice):**
```bash
task p2p:alice
```

**Run Sender (Bob) in another terminal:**
```bash
task p2p:bob

# With MLS encryption
task p2p:mls:bob

# With custom iterations
task p2p:bob EXTRA_ARGS="--iterations 20"
```

**Options:**
- `--local <id>`: Local identity (org/namespace/app) - **REQUIRED**
- `--remote <id>`: Remote identity to send to
- `--server <url>`: Server endpoint (default: `http://localhost:46357`)
- `--message <text>`: Message to send (enables sender mode)
- `--iterations <n>`: Number of messages (default: 10)
- `--shared-secret <secret>`: Authentication secret
- `--enable-mls`: Enable MLS encryption

**Modes:**
- **Receiver**: Omit `--message` to listen for incoming sessions
- **Sender**: Provide `--message` and `--remote` to send messages

**What it does:**
1. Creates app and connects to server
2. **Sender**: Creates session, sends N messages, waits for replies
3. **Receiver**: Listens for session, echoes messages back

---

### 3. Group Example

Demonstrates group messaging with a moderator and participants.

**Run Moderator:**
```bash
task group:moderator
```

**Run Participants in separate terminals:**
```bash
task group:client-1
task group:client-2
```

**Options:**
- `--local <id>`: Local identity (org/namespace/app) - **REQUIRED**
- `--remote <id>`: Group channel name (moderator only)
- `--server <url>`: Server endpoint (default: `http://localhost:46357`)
- `--invites <id>`: Participant to invite (can be repeated, moderator only)
- `--shared-secret <secret>`: Authentication secret
- `--enable-mls`: Enable MLS encryption

**Modes:**
- **Moderator**: Provide `--remote` and `--invites` to create group
- **Participant**: Omit `--remote` to wait for invitation

**Interactive Commands:**
- Type a message and press Enter to send to group
- `invite <id>`: Invite new participant (moderator only)
- `remove <id>`: Remove participant (moderator only)
- `exit` or `quit`: Leave the group

**What it does:**
1. Creates app and connects to server
2. **Moderator**: Creates group session, invites participants
3. **Participant**: Waits for invitation, joins group
4. All members can send/receive messages interactively
5. Messages display actual sender (not local identity)

**Key Implementation Details:**
- Uses `ExecutorService` with 2 threads for parallel receive/keyboard loops
- Receive loop displays messages from context's `sourceName` (fix for sender display bug)
- Keyboard loop reads stdin and publishes messages
- Auto-replies to messages (unless message is itself a reply to avoid loops)

---

## Running All Examples

**Terminal 1 - Server:**
```bash
task server
```

**Terminal 2 - P2P Receiver:**
```bash
task p2p:alice
```

**Terminal 3 - P2P Sender:**
```bash
task p2p:bob
```

Or for group messaging:

**Terminal 1 - Server:**
```bash
task server
```

**Terminal 2 - Group Moderator:**
```bash
task group:moderator
```

**Terminal 3 - Participant 1:**
```bash
task group:client-1
```

**Terminal 4 - Participant 2:**
```bash
task group:client-2
```

## Common Utilities

Located in `src/main/java/io/agntcy/slim/examples/common/`:

### Common.java

- `splitId(String id)`: Parse "org/namespace/app" into `Name`
- `createAndConnectApp(...)`: Helper to create app and connect to server
- `AppConnection`: Container for app + connection ID

### Config.java

- `BaseConfig`: Shared configuration fields
- `PointToPointConfig`: P2P-specific config
- `GroupConfig`: Group-specific config
- `ServerConfig`: Server-specific config
- `ArgParser`: Simple CLI argument parser

### Colors.java

- ANSI color codes for terminal output
- `colored(color, message)`: Format text with color
- `instancePrefix(id)`: Format instance ID prefix

## CompletableFuture Patterns

All async operations use `CompletableFuture`:

```java
// Connect to server
CompletableFuture<Long> connFuture = service.connectAsync(config);
Long connId = connFuture.get();  // Block and get result

// Create session
CompletableFuture<Session> sessionFuture = 
    app.createSessionAndWaitAsync(sessionConfig, remoteName);
Session session = sessionFuture.get();

// Send message
session.publishAndWaitAsync(message.getBytes(), null, null).get();

// Receive message
Duration timeout = Duration.ofSeconds(30);
ReceivedMessage msg = session.getMessageAsync(timeout).get();
```

## Parallel Execution

The Group example demonstrates parallel execution using `ExecutorService`:

```java
ExecutorService executor = Executors.newFixedThreadPool(2);

// Launch receive loop
Future<?> receiveTask = executor.submit(() -> receiveLoop(...));

// Launch keyboard loop
Future<?> keyboardTask = executor.submit(() -> keyboardLoop(...));

// Wait for keyboard to finish
keyboardTask.get();

// Cancel receive loop
receiveTask.cancel(true);

// Cleanup
executor.shutdownNow();
```

## Troubleshooting

### JNA Cannot Find Native Library

If you see `UnsatisfiedLinkError`:

```bash
# Check library exists
ls ../generated/native/

# Ensure bindings are installed
cd ..
task install

# Recompile examples
cd examples
task compile
```

### Maven Dependency Issues

```bash
# Clean and rebuild
mvn clean
rm -rf target/
task install:bindings
task compile
```

### Connection Refused

Ensure the server is running:

```bash
# In separate terminal
task server
```

## Example Output

**Point-to-Point (Sender):**
```
[42] ✅ Created app
[42] 🔌 Connected to http://localhost:46357 (conn ID: 1)
[42] 📍 Route set to agntcy/ns/alice via connection 1
[42] 🔍 Creating session to agntcy/ns/alice...
[42] 📡 Session created
[42] 📤 Sent message 'hello from Java' - 1/5
[42] 📥 Received reply 'hello from Java from 43' - 1/5
...
```

**Group (Moderator):**
```
[44] ✅ Created app
[44] 🔌 Connected to http://localhost:46357 (conn ID: 1)
Creating new group session (moderator)... agntcy/ns/moderator
agntcy/ns/moderator -> add agntcy/ns/client-1 to the group
agntcy/ns/moderator -> add agntcy/ns/client-2 to the group

Welcome to the group agntcy/ns/chat!
Commands:
  - Type a message to send it to the group
  - 'remove NAME' to remove a participant
  - 'invite NAME' to invite a participant
  - 'exit' or 'quit' to leave the group

agntcy/ns/moderator > Hello everyone!

agntcy/ns/client-1 > message received by agntcy/ns/client-1
agntcy/ns/client-2 > message received by agntcy/ns/client-2
```

## Resources

- [SLIM Java Bindings README](../README.md)
- [UniFFI Book](https://mozilla.github.io/uniffi-rs/)
- [CompletableFuture Guide](https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/util/concurrent/CompletableFuture.html)

# SLIM Java Examples

This directory contains Java examples demonstrating the SLIM Java bindings using sync methods.

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

## Sync Methods

The examples use sync methods for simplicity:

```java
// Connect to server
Long connId = service.connect(config);

// Create session
Session session = app.createSessionAndWait(sessionConfig, remoteName);

// Send message
session.publishAndWait(message.getBytes(), null, null);

// Receive message
Duration timeout = Duration.ofSeconds(30);
ReceivedMessage msg = session.getMessage(timeout);
```

**Async alternatives:** `*Async` variants returning `CompletableFuture` are available. They work well with **Virtual Threads** (Java 21+). When using OS threads with async methods, set `-Djava.util.concurrent.ForkJoinPool.common.parallelism=16`.

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
# SLIM Kotlin Bindings

Kotlin/JVM bindings for the SLIM (Secure Lightweight Identity Messaging) data plane, generated using [UniFFI](https://mozilla.github.io/uniffi-rs/).

## Overview

This package provides Kotlin bindings for SLIM, enabling secure messaging with:
- **Point-to-point messaging**: Direct communication between two parties
- **Group messaging**: Multi-party communication with MLS encryption support
- **Identity management**: Support for shared secrets, JWT, and SPIRE authentication
- **Coroutine support**: Idiomatic async/await using Kotlin coroutines

## Prerequisites

- **JDK 17 or higher**
- **Gradle 8.5+** (wrapper included)
- **Rust toolchain** (for building native library)
- **uniffi-bindgen** CLI tool (installed automatically via Taskfile)
- **Task** (for running build tasks) - Install from [taskfile.dev](https://taskfile.dev/)

## Project Structure

```
kotlin/
├── Taskfile.yaml           # Build automation tasks
├── uniffi.toml             # UniFFI configuration (package name)
├── patch-bindings.sh       # Post-generation patches
├── build.gradle.kts        # Gradle build configuration
├── settings.gradle.kts     # Gradle settings
├── gradle.properties       # Build properties
├── examples/
│   ├── common/             # Shared utilities
│   │   ├── Config.kt       # Configuration data classes
│   │   ├── Common.kt       # Helper functions
│   │   └── Colors.kt       # Terminal formatting
│   ├── PointToPoint.kt     # P2P messaging example
│   ├── Group.kt            # Group messaging example
│   └── Server.kt           # SLIM server example
└── generated/              # UniFFI-generated code (gitignored)
    └── io/agntcy/slim/bindings/
        └── slim_bindings.kt
```

## Quick Start

### 1. Generate Bindings

The first step is to build the Rust library and generate Kotlin bindings:

```bash
task generate
```

This will:
1. Build the Rust `slim_bindings` library
2. Run `uniffi-bindgen` with `uniffi.toml` config to generate Kotlin code in the `io.agntcy.slim.bindings` package
3. Apply compatibility patches (exception message parameters, wait() method conflicts)
4. Copy the native library to `generated/jniLibs/`

### 2. Build Examples

Build the Kotlin examples:

```bash
task build
```

### 3. Run Examples

#### Server Example

Start a SLIM server:

```bash
task example:server
```

Or with custom address:

```bash
task example:server CLI_ARGS="--slim 127.0.0.1:8080"
```

#### Point-to-Point Messaging

Run Alice (receiver) in one terminal:

```bash
task example:p2p:alice
```

Run Bob (sender) in another terminal:

```bash
task example:p2p:bob
```

With MLS encryption:

```bash
task example:p2p:mls:bob
```

#### Group Messaging

Run the moderator in one terminal:

```bash
task example:group:moderator
```

Run clients in separate terminals:

```bash
task example:group:client-1
task example:group:client-2
```

## Examples

### Point-to-Point Messaging

**Sender Mode** (sends messages to a specific peer):

```kotlin
import io.agntcy.slim.examples.common.*
import io.agntcy.slim.bindings.*
import kotlinx.coroutines.*

suspend fun main() = coroutineScope {
    val config = PointToPointConfig(
        local = "org/alice/app",
        remote = "org/bob/app",
        message = "Hello, Bob!",
        iterations = 5,
        enableMls = true
    )
    
    val (app, connId) = createLocalApp(config)
    val remoteName = splitId(config.remote!!)
    
    app.setRouteAsync(remoteName, connId)
    
    val sessionConfig = SessionConfig(
        sessionType = SessionType.POINT_TO_POINT,
        enableMls = config.enableMls,
        maxRetries = 5u,
        interval = Duration.ofSeconds(5),
        metadata = emptyMap()
    )
    
    val sessionContext = app.createSessionAsync(sessionConfig, remoteName)
    sessionContext.completion.waitAsync()
    val session = sessionContext.session
    
    repeat(config.iterations) {
        session.publishAsync(config.message!!.toByteArray(), null, null)
        val reply = session.getMessageAsync(Duration.ofSeconds(30))
        println("Received: ${String(reply.payload)}")
    }
}
```

**Receiver Mode** (listens for incoming sessions):

```kotlin
suspend fun main() = coroutineScope {
    val config = PointToPointConfig(local = "org/bob/app")
    val (app, _) = createLocalApp(config)
    
    while (true) {
        val session = app.listenForSessionAsync(null)
        launch {
            while (true) {
                val msg = session.getMessageAsync(Duration.ofSeconds(30))
                println("Received: ${String(msg.payload)}")
                session.publishAsync("Echo: ${String(msg.payload)}".toByteArray(), null, null)
            }
        }
    }
}
```

### Group Messaging

**Moderator** (creates group and invites participants):

```kotlin
suspend fun main() = coroutineScope {
    val config = GroupConfig(
        local = "org/moderator/app",
        remote = "org/default/chat-room",
        invites = listOf("org/alice/app", "org/bob/app"),
        enableMls = true
    )
    
    val (app, connId) = createLocalApp(config)
    val channelName = splitId(config.remote!!)
    
    val sessionConfig = SessionConfig(
        sessionType = SessionType.GROUP,
        enableMls = config.enableMls,
        maxRetries = 5u,
        interval = Duration.ofSeconds(5),
        metadata = emptyMap()
    )
    
    val sessionContext = app.createSession(sessionConfig, channelName)
    sessionContext.completion.waitAsync()
    val session = sessionContext.session
    
    config.invites!!.forEach { invite ->
        val inviteName = splitId(invite)
        app.setRouteAsync(inviteName, connId)
        session.inviteAsync(inviteName).waitAsync()
    }
    
    // Now send/receive messages
    session.publishAsync("Hello, everyone!".toByteArray(), null, null)
}
```

**Participant** (waits for invitation):

```kotlin
suspend fun main() = coroutineScope {
    val config = GroupConfig(local = "org/alice/app")
    val (app, _) = createLocalApp(config)
    
    val session = app.listenForSessionAsync(null)
    
    launch {
        while (true) {
            val msg = session.getMessageAsync(Duration.ofSeconds(30))
            println("${msg.context.sourceName} > ${String(msg.payload)}")
        }
    }
    
    // Interactive message sending
    while (true) {
        val input = readlnOrNull() ?: break
        session.publishAsync(input.toByteArray(), null, null)
    }
}
```

### Server

```kotlin
suspend fun main() {
    val service = setupService(enableOpentelemetry = false)
    val serverConfig = newInsecureServerConfig("127.0.0.1:46357")
    
    service.runServerAsync(serverConfig)
    println("Server started on 127.0.0.1:46357")
    
    awaitCancellation()
}
```

## Configuration

### Command-Line Arguments

All examples support the following common arguments:

- `--local <id>` - Local ID (org/namespace/app) - **REQUIRED**
- `--remote <id>` - Remote ID (org/namespace/app-or-channel)
- `--slim <url>` - SLIM server endpoint (default: `http://127.0.0.1:46357`)
- `--shared-secret <secret>` - Shared secret for authentication
- `--enable-mls` - Enable MLS encryption for sessions
- `--enable-opentelemetry` - Enable OpenTelemetry tracing

**Point-to-Point specific:**
- `--message <text>` - Message to send (activates sender mode)
- `--iterations <n>` - Number of request/reply cycles (default: 10)

**Group specific:**
- `--invites <id>` - Invite participant (can be specified multiple times)

**Server specific:**
- `--slim, -s <address>` - Server address (host:port)

### Environment Variables

Configuration can also be provided via environment variables with the `SLIM_` prefix:

- `SLIM_LOCAL` - Local identity
- `SLIM_ENDPOINT` - Server endpoint
- `SLIM_SHARED_SECRET` - Shared secret
- `SLIM_ENABLE_OPENTELEMETRY` - Enable OpenTelemetry (true/false)
- `SLIM_ENABLE_MLS` - Enable MLS encryption (true/false)

### Authentication Modes

The bindings support three authentication modes:

1. **Shared Secret** (default, for development):
   ```kotlin
   val config = BaseConfig(
       local = "org/alice/app",
       sharedSecret = "my-secret-key-32-chars-min"
   )
   ```

2. **JWT with JWKS**:
   ```kotlin
   val config = BaseConfig(
       local = "org/alice/app",
       jwt = "/path/to/jwt.pem",
       spireTrustBundle = "/path/to/bundle.json",
       audience = listOf("slim-service")
   )
   ```

3. **SPIRE** (dynamic identity):
   ```kotlin
   val config = BaseConfig(
       local = "org/alice/app",
       spireSocketPath = "/tmp/spire-agent/public/api.sock",
       spireTargetSpiffeId = "spiffe://example.org/alice",
       spireJwtAudience = listOf("slim-service")
   )
   ```

## Building

### Task Commands

The project uses [Task](https://taskfile.dev/) for build automation:

- `task generate` - Generate Kotlin bindings from Rust library
- `task build` - Build Kotlin code and create JARs
- `task clean` - Remove build artifacts and generated code
- `task example:server` - Run the server example
- `task example:p2p:alice` - Run Alice (P2P receiver)
- `task example:p2p:no-mls:bob` - Run Bob (P2P sender, without encryption)
- `task example:p2p:mls:bob` - Run Bob with MLS encryption
- `task example:group:moderator` - Run group moderator
- `task example:group:client-1` - Run group participant 1
- `task example:group:client-2` - Run group participant 2

## Dependencies

- **JNA 5.14.0+** - Native library loading via Java Native Access
- **kotlinx-coroutines-core 1.8.1+** - Coroutine support for async operations
- **Clikt 4.2.2** - Command-line interface parsing

## Architecture

### UniFFI Code Generation

The Kotlin bindings are generated from the Rust `slim_bindings` crate using UniFFI:

1. Rust code uses `#[uniffi::export]` macros to mark public APIs
2. `uniffi-bindgen` reads the compiled library and generates Kotlin code
3. Generated code uses JNA to call native functions
4. Native library is packaged in `generated/jniLibs/` for runtime loading

### Async Support

Rust async functions are exposed as Kotlin `suspend` functions:

```rust
// Rust
#[uniffi::export]
pub async fn connect_async(&self, config: ClientConfig) -> Result<u64, SlimError>
```

```kotlin
// Generated Kotlin
suspend fun connectAsync(config: ClientConfig): ULong
```

### Error Handling

Rust `Result<T, E>` types are converted to Kotlin exceptions:

```rust
// Rust
pub enum SlimError {
    ConnectionFailed(String),
    SessionClosed(String),
    // ...
}
```

```kotlin
// Kotlin
try {
    session.publishAsync(data, null, null)
} catch (e: SlimException.SessionClosed) {
    println("Session closed: ${e.message}")
}
```

## Development

### Project Layout

- `examples/common/` - Shared utilities and configuration
- `examples/*.kt` - Example applications
- `generated/` - UniFFI-generated Kotlin code (not in git)
- `build.gradle.kts` - Gradle build configuration
- `Taskfile.yaml` - Task automation

## License

Apache 2.0 - See LICENSE.md for details

## Contributing

See CONTRIBUTING.md for contribution guidelines

## Links

- [SLIM Project](https://github.com/agntcy/slim)
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/)
- [Kotlin Coroutines](https://kotlinlang.org/docs/coroutines-overview.html)
- [Task Documentation](https://taskfile.dev/)

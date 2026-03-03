# SLIM Java Bindings

Java bindings for SLIM (Secure Lightweight Interconnected Messaging) generated via [uniffi-bindgen-java](https://github.com/IronCoreLabs/uniffi-bindgen-java).

## Features

- **Native Java Interface**: Idiomatic Java API with sync methods; async variants via `CompletableFuture` available
- **Java 21**: Built for modern Java with records and latest language features
- **JNA Integration**: Seamless native library loading via Java Native Access (JNA)
- **Complete Examples**: Server, Point-to-Point, and Group messaging examples
- **Task Automation**: Integrated with Taskfile for easy builds and testing

## Requirements

- **Java 21** or higher
- **Maven 3.8+**
- **Rust toolchain** (for building native library)
- **uniffi-bindgen-java** (auto-installed via Taskfile)

## Quick Start

### 1. Generate Bindings

```bash
cd data-plane/bindings/java
task generate
```

This will:
- Build the Rust bindings library
- Install uniffi-bindgen-java (if not present)
- Generate Java source files
- Copy native libraries to the correct platform directory

### 2. Build Java Bindings

```bash
task build
```

Creates a JAR at `target/slim-bindings-java-1.2.0.jar`.

### 3. Install to Local Maven Repository

```bash
task install
```

Makes the bindings available to the examples and other local projects.

### 4. Run Examples

```bash
# Start server
task examples:server

# Point-to-point (in separate terminals)
task examples:p2p:alice
task examples:p2p:bob

# Group messaging (in separate terminals)
task examples:group:moderator
task examples:group:client-1
task examples:group:client-2
```

## Architecture

```
┌─────────────────────────┐
│  Java Application       │
│  (Your Code)            │
└───────────┬─────────────┘
            │
            │ Sync / CompletableFuture API
            ▼
┌─────────────────────────┐
│  Generated Java Classes │
│  (uniffi-bindgen-java)  │
└───────────┬─────────────┘
            │
            │ JNA
            ▼
┌─────────────────────────┐
│  Native Library         │
│  libslim_bindings.so    │
│  (Rust + UniFFI)        │
└─────────────────────────┘
```

## Directory Structure

```
data-plane/bindings/java/
├── Taskfile.yaml           # Build automation
├── pom.xml                 # Maven configuration
├── uniffi.toml             # UniFFI Java bindings config
├── README.md               # This file
├── generated/              # Generated code (gitignored)
│   ├── uniffi/slim_bindings/  # Java source files
│   └── native/             # Native libraries by platform
│       ├── darwin-x86_64/
│       ├── darwin-aarch64/
│       ├── linux-x86_64/
│       └── windows-x86_64/
└── examples/               # Java examples
    ├── Taskfile.yaml
    ├── pom.xml
    └── src/main/java/io/agntcy/slim/examples/
        ├── common/         # Shared utilities
        ├── Server.java     # Server example
        ├── PointToPoint.java  # P2P messaging
        └── Group.java      # Group messaging
```

## API Overview

### Initialization

```java
import io.agntcy.slim.bindings.*;

// Initialize with defaults
SlimBindings.initializeWithDefaults();

// Or with OpenTelemetry tracing
SlimBindings.initializeWithTracing("my-app", "localhost:4317");
```

### Creating an App

```java
Service service = SlimBindings.getGlobalService();
Name appName = new Name("org", "namespace", "app");

// Create app with shared secret
App app = service.createAppWithSecret(appName, "my-secret-min-32-chars!!");
```

### Connecting to Server

```java
// Create client config
ClientConfig config = SlimBindings.newInsecureClientConfig("http://localhost:46357");

// Connect (sync)
Long connectionId = service.connect(config);
```

### Creating Sessions

```java
Name remoteName = new Name("org", "namespace", "remote");

// Create point-to-point session
SessionConfig sessionConfig = new SessionConfig(
    SessionType.POINT_TO_POINT,
    true,  // enableMls
    null,  // maxRetries
    null,  // interval
    Map.of()  // metadata
);

Session session = app.createSessionAndWait(sessionConfig, remoteName);
```

### Sending Messages

```java
byte[] message = "Hello, SLIM!".getBytes();

// Publish and wait
session.publishAndWait(message, null, null);
```

### Receiving Messages

```java
Duration timeout = Duration.ofSeconds(30);
ReceivedMessage msg = session.getMessage(timeout);

System.out.println("Received: " + new String(msg.payload()));
```

### Async Methods (CompletableFuture)

Async variants (`*Async`) returning `CompletableFuture` are available for all operations. They can be a good choice when using **Virtual Threads** (Java 21+), where blocking is cheap and you can use `.get()` or `.join()` without tying up OS threads.

When using the **standard Java thread model** (OS threads) with async methods, consider increasing the common pool parallelism:

```bash
java -Djava.util.concurrent.ForkJoinPool.common.parallelism=16 -jar myapp.jar
```

## Available Tasks

Run `task` in the `data-plane/bindings/java/` directory to see all available tasks:

```bash
task                      # List all tasks
task generate             # Generate Java bindings
task build                # Build Java code and create JAR
task install              # Install JAR to local Maven repository
task clean                # Clean build artifacts and generated code
task examples:server      # Run server example
task examples:p2p:alice   # Run P2P receiver
task examples:p2p:bob     # Run P2P sender
task examples:group:*     # Run group examples
```

## Examples

See [examples/README.md](examples/README.md) for detailed example documentation.

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| macOS | x86_64 | ✅ Supported |
| macOS | aarch64 (M1/M2) | ✅ Supported |
| Linux | x86_64 | ✅ Supported |
| Linux | aarch64 | ✅ Supported |
| Windows | x86_64 | 🚧 Experimental |

## Resources

- [UniFFI Book](https://mozilla.github.io/uniffi-rs/)
- [uniffi-bindgen-java GitHub](https://github.com/IronCoreLabs/uniffi-bindgen-java)
- [JNA Documentation](https://github.com/java-native-access/jna)
- [SLIM Project](https://github.com/agntcy/slim)

## License

Copyright AGNTCY Contributors (https://github.com/agntcy)

SPDX-License-Identifier: Apache-2.0

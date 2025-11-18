# SLIM Go Bindings

Go language bindings for the SLIM (Secure Low-Latency Interactive Messaging) system, generated using [UniFFI](https://mozilla.github.io/uniffi-rs/).

## Prerequisites

### 1. Install uniffi-bindgen-go

```bash
cargo install uniffi-bindgen-go --git https://github.com/NordSecurity/uniffi-bindgen-go --tag v0.4.0+v0.28.3
```

### 2. Install Rust toolchain

Make sure you have Rust 1.81 or newer:

```bash
rustc --version
```

### 3. Install Go

Make sure you have Go 1.19 or newer:

```bash
go version
```

## Building the Bindings

### Step 1: Build the Rust library

From the `data-plane/bindings/go` directory:

```bash
# Build in release mode for better performance
cd ../common
cargo build --release

# Or build in debug mode for development
cargo build
```

This will create the shared library at:
- **macOS**: `../common/target/release/libslim_go_bindings.dylib` (or `debug/` for debug builds)
- **Linux**: `../common/target/release/libslim_go_bindings.so`
- **Windows**: `../common/target/release/slim_go_bindings.dll`

### Step 2: Generate Go bindings

```bash
# Generate Go bindings from the UDL file
uniffi-bindgen-go ../common/src/slim_bindings.udl --out-dir generated --config ../common/uniffi.toml
```

This will create Go files in the `generated/` directory.

### Step 3: Set up library path

You need to ensure the Go runtime can find the compiled Rust library:

**macOS/Linux:**
```bash
export LD_LIBRARY_PATH="../common/target/release:$LD_LIBRARY_PATH"
# On macOS, you might also need:
export DYLD_LIBRARY_PATH="../common/target/release:$DYLD_LIBRARY_PATH"
```

**Or copy the library to a standard location:**
```bash
# macOS
sudo cp ../common/target/release/libslim_go_bindings.dylib /usr/local/lib/

# Linux
sudo cp ../common/target/release/libslim_go_bindings.so /usr/local/lib/
sudo ldconfig
```

## Usage Example

```go
package main

import (
    "fmt"
    "log"
    
    slim "github.com/agntcy/slim/bindings/generated/slimbindings"
)

func main() {
    // Initialize crypto provider (required)
    slim.InitializeCrypto()
    
    // Create a service
    service, err := slim.NewService()
    if err != nil {
        log.Fatalf("Failed to create service: %v", err)
    }
    
    // Create an app with shared secret authentication
    appName := slim.Name{
        Components: []string{"org", "myapp", "v1"},
        Id: nil,
    }
    
    app, err := service.CreateApp(appName, "my-shared-secret-value-must-be-long-enough")
    if err != nil {
        log.Fatalf("Failed to create app: %v", err)
    }
    
    fmt.Printf("App created with ID: %d\n", app.Id())
    
    // Create a session configuration
    sessionConfig := slim.SessionConfig{
        SessionType: slim.SessionTypePointToPoint,
        EnableMls: false,
    }
    
    destination := slim.Name{
        Components: []string{"org", "other-app", "v1"},
        Id: nil,
    }
    
    // Create a session
    session, err := app.CreateSession(sessionConfig, destination)
    if err != nil {
        log.Fatalf("Failed to create session: %v", err)
    }
    
    // Publish a message
    message := []byte("Hello from Go!")
    err = session.Publish(
        destination,
        1, // fanout
        message,
        nil, // connection_id
        &[]string{"text/plain"}[0], // payload_type
        nil, // metadata
    )
    if err != nil {
        log.Fatalf("Failed to publish message: %v", err)
    }
    
    fmt.Println("Message sent successfully!")
}
```

## Project Structure

```
bindings/
├── common/                 # Shared Rust implementation
│   ├── src/
│   │   ├── lib.rs              # Main Rust implementation
│   │   ├── error.rs            # Error types
│   │   ├── slim_bindings.udl   # UniFFI interface definition
│   │   └── bin/
│   │       └── uniffi-bindgen.rs
│   ├── Cargo.toml          # Rust package configuration
│   ├── build.rs            # Build script for UniFFI scaffolding
│   └── uniffi.toml         # UniFFI configuration
└── go/                     # Go-specific files
    ├── examples/
    │   └── simple/
    ├── generated/          # Generated Go code (created by uniffi-bindgen-go)
    ├── Makefile
    └── README.md          # This file
```

## API Reference

### Core Types

- **Service**: Main entry point, manages the SLIM data plane
- **App**: Application-level API for session and routing management
- **SessionContext**: Handle for messaging within a session
- **Name**: Hierarchical identity name
- **MessageContext**: Metadata about received messages
- **SessionConfig**: Configuration for creating sessions

### Key Methods

#### Service
- `NewService()`: Create a new SLIM service instance
- `CreateApp(name, secret)`: Create an app with shared secret auth

#### App
- `Id()`: Get the app's unique ID
- `Name()`: Get the app's name
- `CreateSession(config, destination)`: Create a new session
- `Subscribe(name, conn_id)`: Subscribe to a name
- `Unsubscribe(name, conn_id)`: Unsubscribe from a name
- `ListenForSession(timeout_ms)`: Wait for incoming sessions

#### SessionContext
- `Publish(dest, fanout, payload, ...)`: Send a message
- `PublishTo(context, payload, ...)`: Reply to a message
- `GetMessage(timeout_ms)`: Receive a message
- `Invite(peer)`: Invite a peer to the session
- `Remove(peer)`: Remove a peer from the session

## Development

### Rebuilding after changes

After modifying Rust code or the UDL file:

```bash
# Rebuild the Rust library
cargo build --release

# Regenerate Go bindings
uniffi-bindgen-go src/slim_bindings.udl --out-dir generated --config uniffi.toml
```

### Testing

You can create a simple test in Go:

```bash
cd generated
go mod init github.com/agntcy/slim/bindings/generated
go mod tidy
go test
```

# SLIM C# SDK

C# SDK for [SLIM](https://github.com/agntcy/slim) (Secure Low-Latency Interactive Messaging).

This SDK provides an idiomatic C# API built on top of auto-generated bindings from [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs).

## Requirements

- .NET 8.0 or higher
- [Rust toolchain](https://rustup.rs/) (for building native libraries from source)
- [Task](https://taskfile.dev/) (optional, for build automation)

## Quick Start

### Using NuGet Package

```bash
dotnet add package Agntcy.Slim
```

The NuGet package includes native libraries for all supported platforms. No additional setup required!

```csharp
using Agntcy.Slim;

Slim.Initialize();
var service = Slim.GetGlobalService();
// ...
Slim.Shutdown();
```

### Building from Source

```bash
cd data-plane/bindings/dotnet
task setup  # Build rust library and generate C# bindings
task build  # Builds the .NET solution
```

## API Overview

### Main Classes

| Class | Description |
|-------|-------------|
| `Slim` | Static entry point for initialization, shutdown, and global access |
| `SlimService` | Service for managing connections and creating apps |
| `SlimApp` | Application for managing sessions, subscriptions, and routing |
| `SlimSession` | Session for sending and receiving messages |
| `SlimName` | Identity in `org/namespace/app` format |
| `SlimMessage` | Received message with `Payload` (bytes) and `Text` (string) |
| `SlimSessionConfig` | Session configuration (type, MLS, retries) |

## Running Tests

**Smoke tests** (no server required):
```bash
task test:smoke
```

**Integration tests** (requires running server):
```bash
# Terminal 1: Start the server
cd ../../
task run:server

# Terminal 2: Run integration tests
task test
```

**All tests:**
```bash
task test
```

## Development

### Available Tasks

```bash
task                  # List all available tasks
task setup            # Build rust library and generate C# bindings
task build            # Build the .NET solution
task test             # Run all tests
task test:smoke       # Run smoke tests (no server required)
task pack             # Create NuGet package
task clean            # Clean all build artifacts
```

### Regenerating Bindings

If the Rust bindings change:

```bash
task generate
git add Slim/generated/
git commit -m "Regenerate dotnet bindings"
```

## Platform Support

| Platform | Architecture | .NET RID | Status |
|----------|--------------|----------|--------|
| Linux    | x86_64       | linux-x64 | Supported |
| Linux    | aarch64      | linux-arm64 | Supported |
| macOS    | x86_64       | osx-x64 | Supported |
| macOS    | aarch64      | osx-arm64 | Supported |
| Windows  | x86_64       | win-x64 | Supported |

## NuGet Package

The NuGet package includes native libraries for all supported platforms using the standard `runtimes/{rid}/native/` structure. When you install the package, .NET automatically selects the correct native library for your platform.

```bash
# Install the package
dotnet add package Agntcy.Slim

# Build and run - native library is automatically included
dotnet run
```

## License

Apache-2.0 - See [LICENSE](../../../../LICENSE.md) for details.

## Related Projects

- [Go bindings](../go) - Go bindings for SLIM
- [Python bindings](../python) - Python bindings for SLIM
- [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) - C# bindings generator for UniFFI

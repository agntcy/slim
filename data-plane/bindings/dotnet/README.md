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
task build  # Build rust library, generate bindings, and build .NET solution
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

## Examples

The SDK includes complete working examples demonstrating real-world usage:

- **Point-to-Point** (`Slim.Examples.PointToPoint`): 1:1 messaging with request/reply pattern
- **Group Messaging** (`Slim.Examples.Group`): Group sessions with moderator/participant roles

**Quick Start:**
```bash
# Run receiver
dotnet run --project Slim.Examples.PointToPoint -- --local org/alice/v1

# Run sender (in another terminal)
dotnet run --project Slim.Examples.PointToPoint -- \
  --local org/bob/v1 --remote org/alice/v1 --message "Hello"
```

See example source code in `Slim.Examples.*` projects for full implementation details and command-line options.

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
task test:integration
```

**All tests:**
```bash
task test:all
```

## Development

### Available Tasks

```bash
task                     # List all available tasks
task build               # Build rust library, generate bindings, and build .NET solution
task test:smoke          # Run smoke tests (no server required)
task test:integration    # Run integration tests (requires running SLIM server)
task test:all            # Run all tests (smoke + integration)
task pack                # Create NuGet package with all runtime libraries
task publish             # Publish NuGet package to NuGet.org (requires NUGET_API_TOKEN)
task test:verify-package # Verify NuGet package can be installed and works locally
task clean               # Clean all build artifacts
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

### Publishing to NuGet

The NuGet package is automatically published when a release tag is pushed (e.g., `slim-bindings-v0.1.0`). The release workflow:

1. Builds and tests bindings on all supported platforms
2. Creates a NuGet package with native libraries for all platforms
3. Publishes to NuGet.org
4. Tests the published package on 5 platforms (Linux x64 GNU/musl, Windows x64, macOS x64/arm64)
5. Finalizes the release

**Manual Publishing:**
```bash
# Create package
task pack

# Publish to NuGet.org
task publish NUGET_API_TOKEN=<your-api-token>

# Or verify locally before publishing
task test:verify-package
```

## License

Apache-2.0 - See [LICENSE](../../../../LICENSE.md) for details.

## Related Projects

- [Go bindings](../go) - Go bindings for SLIM
- [Python bindings](../python) - Python bindings for SLIM
- [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) - C# bindings generator for UniFFI

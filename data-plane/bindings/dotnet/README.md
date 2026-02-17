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

To build the complete SDK, you need artifacts from the Rust bindings build:

```bash
# From the root of the repository
cd data-plane/bindings/dotnet
task ci BINDGEN_TARGET=x86_64-unknown-linux-gnu PROFILE=release
```

For development on a single platform:

```bash
task generate
task build
task test
```

## API Overview

### Main Classes

| Class               | Description                                                        |
| ------------------- | ------------------------------------------------------------------ |
| `Slim`              | Static entry point for initialization, shutdown, and global access |
| `SlimService`       | Service for managing connections and creating apps                 |
| `SlimApp`           | Application for managing sessions, subscriptions, and routing      |
| `SlimSession`       | Session for sending and receiving messages                         |
| `SlimName`          | Identity in `org/namespace/app` format                             |
| `SlimMessage`       | Received message with `Payload` (bytes) and `Text` (string)        |
| `SlimSessionConfig` | Session configuration (type, MLS, retries)                         |

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

The SDK includes smoke tests that don't require a running SLIM server:

```bash
task test
```

For full integration testing with a SLIM server, you would need to:

1. Start a SLIM server instance
2. Run the example applications to verify functionality

See the `Slim.Examples.*` projects for working integration examples.

## Development

### Available Tasks

```bash
task                  # List all available tasks
task ci               # Run complete CI pipeline: generate, copy runtimes, build, test, pack
task generate         # Generate C# bindings from artifacts (requires ARTIFACTS_DIR)
task copy:runtimes    # Copy runtime libraries from artifacts to runtimes directory
task build            # Build .NET solution (expects bindings already generated)
task test             # Run smoke tests (no server required)
task pack             # Create NuGet package (expects runtimes already copied)
task clean            # Clean all build artifacts
```

### Regenerating Bindings

If the Rust bindings change, you need to rebuild the Rust library first, then regenerate C# bindings:

```bash
task generate
```

## Platform Support

| Platform     | Architecture | .NET RID         | Status    |
| ------------ | ------------ | ---------------- | --------- |
| Linux (GNU)  | x86_64       | linux-x64        | Supported |
| Linux (GNU)  | aarch64      | linux-arm64      | Supported |
| Linux (musl) | x86_64       | linux-musl-x64   | Supported |
| Linux (musl) | aarch64      | linux-musl-arm64 | Supported |
| macOS        | x86_64       | osx-x64          | Supported |
| macOS        | aarch64      | osx-arm64        | Supported |
| Windows      | x86_64       | win-x64          | Supported |
| Windows      | aarch64      | win-arm64        | Supported |

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

1. Builds native libraries for all supported platforms
2. Tests bindings locally with all platform libraries
3. Creates a NuGet package with native libraries for all platforms
4. Publishes to NuGet.org

## License

Apache-2.0 - See [LICENSE](../../../../LICENSE.md) for details.

## Related Projects

- [Go bindings](../go) - Go bindings for SLIM
- [Python bindings](../python) - Python bindings for SLIM
- [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) - C# bindings generator for UniFFI

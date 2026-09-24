# .NET SDK

The SLIM .NET SDK (`Agntcy.Slim`) provides an idiomatic C# API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs), with full async/await support, `CancellationToken` integration, and native libraries bundled for all supported platforms.

## Requirements

| | |
|---|---|
| **Runtime** | .NET 8.0 or higher |
| **Package** | [`Agntcy.Slim`](https://www.nuget.org/packages/Agntcy.Slim) on NuGet |
| **Examples** | [dotnet/](https://github.com/agntcy/slim-bindings/tree/main/dotnet) in slim-bindings |

The NuGet package ships native libraries for Linux (glibc and musl), macOS, and Windows on x64 and arm64. No additional runtime setup is required after install.

## Installation

=== "dotnet CLI"

    ```bash
    dotnet add package Agntcy.Slim
    ```

=== "PackageReference"

    Add to your `.csproj`:

    ```xml
    <ItemGroup>
      <PackageReference Include="Agntcy.Slim" Version="2.1.0" />
    </ItemGroup>
    ```

=== "Package Manager Console"

    ```powershell
    Install-Package Agntcy.Slim
    ```

## Getting Started

The [SDK tutorials](./tutorials/index.md) build a full application step by step — initialising the service, connecting to a node, creating an app, opening sessions, receiving messages, and adding persistence — with .NET snippets shown alongside every other binding.

Start with [Connecting to SLIM](./tutorials/tutorial-connect.md).

## API Overview

| Class | Description |
|---|---|
| `Slim` | Static entry point for initialisation, shutdown, and global service access |
| `SlimService` | Manages connections and creates apps |
| `SlimApp` | Application handle for sessions, subscriptions, and routing |
| `SlimSession` | Session for sending and receiving messages |
| `SlimName` | Identity in `org/namespace/app` format |
| `SlimMessage` | Received message with `Payload` (bytes) and `Text` (string) |
| `SlimSessionConfig` | Session configuration (type, MLS, retries) |
| `SlimClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `SlimServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `SlimOidcConfig` | OIDC transport authentication settings |
| `SlimOidcPolicy` | Claim-based access policy (`Rego`, `RegoFile`, `Cel`) |

Most handle types implement `IDisposable`. Use `using` declarations to ensure native resources are released promptly.

When your process exits, call `Slim.Shutdown()` to release native resources cleanly.

## Transport Authentication

Separate from the app identity passed to `CreateApp`, the gRPC connection to a SLIM node can carry its own credentials.

### OIDC (client credentials)

```csharp
var clientConfig = Slim.NewInsecureClientConfig("http://127.0.0.1:46357")
    .WithOidc(new SlimOidcConfig
    {
        IssuerUrl = "https://auth.example.com",
        ClientId = "my-client",
        ClientSecret = "s3cr3t",
        Scope = "openid profile",
        Timeout = TimeSpan.FromSeconds(30)
    });

var connId = service.Connect(clientConfig);
```

For the refresh-token flow, set `RefreshToken` (or `RefreshTokenFile`, which is rewritten in place as tokens rotate) instead of `ClientSecret`.

### OIDC (server verification)

```csharp
var serverConfig = Slim.NewInsecureServerConfig("127.0.0.1:46357")
    .WithOidc(new SlimOidcConfig
    {
        IssuerUrl = "https://auth.example.com",
        Audience = "slim",
        JwksTtl = TimeSpan.FromHours(1),
        ClaimCacheTtl = TimeSpan.FromMinutes(1),
        Policy = new SlimOidcPolicy.Cel("\"admin\" in claims.groups")
    });

await service.RunServerAsync(serverConfig);
```

`Policy` accepts `SlimOidcPolicy.Cel`, `SlimOidcPolicy.Rego` (which must define `package slim.auth` with `default allow = false`), or `SlimOidcPolicy.RegoFile`.

### JSON configuration

`Slim.NewClientConfigFromJson` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode (`basic`, `static_jwt`, `jwt`, `spire`, `oidc`):

```json
{
  "endpoint": "http://127.0.0.1:46357",
  "tls": { "insecure": true },
  "auth": {
    "type": "oidc",
    "issuer_url": "https://auth.example.com",
    "client_id": "my-client",
    "client_secret": "s3cr3t",
    "audience": "slim",
    "policy": { "cel": "\"admin\" in claims.groups" }
  }
}
```

The schema matches the [client configuration schema](https://github.com/agntcy/slim/blob/slim-v2.0.0/crates/config/src/schema/client-config.schema.json) in the slim repository.

## SLIMRPC

The .NET SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-csharp` plugin and add it to your `buf.gen.yaml` alongside the standard C# protobuf plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

Generated code lives in the `Agntcy.Slim.SlimRpc` namespace.

## Examples

The [slim-bindings/dotnet](https://github.com/agntcy/slim-bindings/tree/main/dotnet) directory includes complete working examples:

| Project | Description |
|---|---|
| `Slim.Examples.PointToPoint` | 1:1 messaging with request/reply |
| `Slim.Examples.Group` | Group sessions with moderator/participant roles |
| `Slim.Examples.SlimRpc` | Protobuf RPC over SLIM |

**Point-to-point:**

```bash
# Receiver
dotnet run --project Slim.Examples.PointToPoint -- --local org/alice/v1

# Sender (in another terminal)
dotnet run --project Slim.Examples.PointToPoint -- \
  --local org/bob/v1 --remote org/alice/v1 --message "Hello"
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task slimrpc:generate-proto   # requires buf and protoc-gen-slimrpc-csharp
dotnet run --project Slim.Examples.SlimRpc -- --mode server
dotnet run --project Slim.Examples.SlimRpc -- --mode client
```

## Platform Support

| Platform | Architecture | .NET RID |
|---|---|---|
| Linux (GNU) | x86_64 | `linux-x64` |
| Linux (GNU) | aarch64 | `linux-arm64` |
| Linux (musl) | x86_64 | `linux-musl-x64` |
| Linux (musl) | aarch64 | `linux-musl-arm64` |
| macOS | x86_64 | `osx-x64` |
| macOS | aarch64 | `osx-arm64` |
| Windows | x86_64 | `win-x64` |
| Windows | aarch64 | `win-arm64` |

When you install the NuGet package, .NET automatically selects the correct native library for your platform from the `runtimes/{rid}/native/` layout.

## Building from Source

To build the .NET SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/dotnet

# Full CI pipeline (generate bindings, copy runtimes, build, test, pack)
task ci BINDGEN_TARGET=x86_64-unknown-linux-gnu PROFILE=release
```

For local development on a single platform:

```bash
task generate   # regenerate C# bindings from Rust artifacts
task build
task test
```

See the [dotnet README](https://github.com/agntcy/slim-bindings/blob/main/dotnet/README.md) for the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM

# Go SDK

The SLIM Go SDK (`github.com/agntcy/slim-bindings-go`) provides an idiomatic Go API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [uniffi-bindgen-go](https://github.com/NordSecurity/uniffi-bindgen-go), with native libraries loaded through CGO.

## Requirements

| | |
|---|---|
| **Runtime** | Go 1.23 or higher |
| **Package** | [`github.com/agntcy/slim-bindings-go`](https://github.com/agntcy/slim-bindings-go) |
| **Build tools** | C compiler (GCC or Clang) for CGO |
| **Examples** | [go/examples](https://github.com/agntcy/slim-bindings/tree/main/go/examples) in slim-bindings |

Go's module system requires source hosting rather than a package registry, so the binding is distributed as a separate module. After `go get`, run the setup tool once to install native libraries for your platform.

!!! warning "CGO Requirement"
    The Go bindings use native libraries via [CGO](https://pkg.go.dev/cmd/cgo). A C compiler must be installed.

## Installation

```bash
go get github.com/agntcy/slim-bindings-go
```

Then run the setup tool to install the native libraries:

```bash
go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
```

Setup is one-time per machine. The tool downloads or copies the native library for your OS and architecture.

## Quick Start

With a SLIM node running locally (see [Getting Started](../../slim-howto.md)), initialise the SDK, connect, and register an application identity:

```go
package main

import (
    "fmt"
    "log"

    slim "github.com/agntcy/slim-bindings-go"
)

func main() {
    slim.InitializeWithDefaults()
    service := slim.GetGlobalService()

    config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")
    connID, err := service.ConnectAsync(config)
    if err != nil {
        log.Fatal(err)
    }

    localName, err := slim.NameFromString("myorg/default/my-service")
    if err != nil {
        log.Fatal(err)
    }

    app, err := service.CreateAppWithSecret(
        localName, "change-me-before-going-to-production",
    )
    if err != nil {
        log.Fatal(err)
    }
    defer app.Destroy()

    if err := app.SubscribeAsync(localName, &connID); err != nil {
        log.Fatal(err)
    }

    fmt.Printf("App ready: %s, id=%s\n", localName, app.Id())
}
```

!!! note "Insecure mode"
    `NewInsecureClientConfig` skips TLS and is for local development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

## API Overview

| Type | Description |
|---|---|
| `slim` package | Static entry point for initialisation and global service access |
| `Service` | Manages connections and creates apps |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with `Payload` (bytes) and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `ClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `ServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `OidcConfig` | OIDC transport authentication settings |
| `OidcPolicyConfig` | Claim-based access policy (`OidcPolicyConfigCel`, `Rego`, `RegoFile`) |

Call `app.Destroy()` when finished to release native resources.

### Initialisation and Connection

```go
slim.InitializeWithDefaults()
service := slim.GetGlobalService()

config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")
connID, err := service.ConnectAsync(config)
```

### Sessions

Point-to-point sessions use `CreateSessionAndWait`:

```go
sessionConfig := slim.SessionConfig{
    SessionType: slim.SessionTypePointToPoint,
    EnableMls:   true,
    MaxRetries:  5,
    Interval:    5 * time.Second,
}

session, err := app.CreateSessionAndWait(sessionConfig, remoteName)
if err != nil {
    log.Fatal(err)
}

session.PublishAndWait([]byte("Hello, SLIM!"), nil, nil)
```

## Transport Authentication

Separate from the app identity passed to `CreateAppWithSecret`, the gRPC connection to a SLIM node can carry its own credentials via `ClientConfig.Auth`.

### OIDC (client credentials)

```go
timeout := 30 * time.Second
var auth slim.ClientAuthenticationConfig = slim.ClientAuthenticationConfigOidc{
    Config: slim.OidcConfig{
        IssuerUrl:    "https://auth.example.com",
        ClientId:     ptr("my-client"),
        ClientSecret: ptr("s3cr3t"),
        Scope:        ptr("openid profile"),
        Timeout:      &timeout,
    },
}

config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")
config.Auth = &auth
connID, err := service.ConnectAsync(config)
```

For the refresh-token flow, set `RefreshToken` (or `RefreshTokenFile`, which is rewritten in place as tokens rotate) instead of `ClientSecret`.

### OIDC (server verification)

```go
jwksTTL := time.Hour
var policy slim.OidcPolicyConfig = slim.OidcPolicyConfigCel{
    Expression: `"admin" in claims.groups`,
}
var auth slim.ServerAuthenticationConfig = slim.ServerAuthenticationConfigOidc{
    Config: slim.OidcConfig{
        IssuerUrl: "https://auth.example.com",
        Audience:  ptr("slim"),
        JwksTtl:   &jwksTTL,
        Policy:    &policy,
    },
}

config := slim.NewInsecureServerConfig("127.0.0.1:46357")
config.Auth = &auth
```

`Policy` accepts `OidcPolicyConfigCel`, `OidcPolicyConfigRego` (which must define `package slim.auth` with `default allow = false`), or `OidcPolicyConfigRegoFile`.

### JSON configuration

`NewConfigFromJson` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode:

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

The Go SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-go` plugin and add it to your `buf.gen.yaml` alongside the standard Go protobuf plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

## Examples

The [slim-bindings/go](https://github.com/agntcy/slim-bindings/tree/main/go) directory includes complete working examples:

| Example | Description |
|---|---|
| `examples/point_to_point` | 1:1 messaging with request/reply |
| `examples/group` | Group sessions with moderator/participant roles |
| `examples/slimrpc/simple` | Protobuf RPC over SLIM |
| `examples/server` | SLIM data plane server |

**Point-to-point:**

```bash
cd go
task examples:p2p:alice       # Receiver
task examples:p2p:no-mls:bob  # Sender (in another terminal)
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task examples:rpc:server
task examples:rpc:client
```

## Platform Support

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86_64 | Supported |
| Linux | aarch64 | Supported |
| macOS | x86_64 | Supported |
| macOS | aarch64 (Apple Silicon) | Supported |
| Windows | x86_64 | Supported |

## Building from Source

To build the Go SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/go

task generate   # regenerate Go bindings from Rust artifacts
task build
task test
```

See the [go README](https://github.com/agntcy/slim-bindings/blob/main/go/slim_bindings/README.md) for the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM

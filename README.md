[![CI](https://github.com/agntcy/slim/actions/workflows/ci.yaml/badge.svg)](https://github.com/agntcy/slim/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/gh/agntcy/slim/branch/main/graph/badge.svg)](https://codecov.io/gh/agntcy/slim)
[![Coverage](https://img.shields.io/badge/Coverage-passing-brightgreen)](https://codecov.io/gh/agntcy/slim)

# SLIM

**SLIM (Secure Low-Latency Interactive Messaging)** is a next-generation
communication framework that provides the secure, scalable transport layer for
AI agent protocols like [A2A (Agent-to-Agent)](https://a2a.ai) and
[MCP (Model Context Protocol)](https://modelcontextprotocol.io).

- 📖 **[Read the full documentation](https://docs.agntcy.org/slim/overview/)**
- 🎓 **[Get Started](https://docs.agntcy.org/slim/slim-howto)**
- 💻 **[Code Examples](https://github.com/agntcy/slim-bindings)** -
  [Python](https://github.com/agntcy/slim-bindings/tree/main/python/examples) |
  [Go](https://github.com/agntcy/slim-bindings/tree/main/go/examples) |
  [Dotnet](https://github.com/agntcy/slim-bindings/tree/main/dotnet) |
  [Java](https://github.com/agntcy/slim-bindings/tree/main/java/examples) |
  [Kotlin](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples)
- 🔌 **[Integrations](https://docs.agntcy.org/slim/slim-rpc/)** -
  [A2A](https://github.com/agntcy/slim-a2a-python) |
  [MCP](https://github.com/agntcy/slim-mcp-python) |
  [OpenTelemetry](https://github.com/agntcy/slim-otel)
- 🚀 **[Deployment Strategies](./deployments/readme.md)**
- 📝 **[Technical blog posts](https://blogs.agntcy.org)**

## Architecture

SLIM uses a
[distributed architecture](https://docs.agntcy.org/slim/overview/#slim-components)
with three main components:

- **Data Plane**: Pure message routing layer that forwards packets based on
  hierarchical names without inspecting application content
- **Session Layer**: Handles reliable delivery, end-to-end MLS encryption, and
  group membership management
- **Control Plane**: Manages configuration, monitoring, and orchestration of
  SLIM routing nodes

This separation enables efficient deployment: SLIM routing nodes run only the
lightweight data plane, while applications use language bindings with the full
stack (data plane client + [session layer](https://docs.agntcy.org/slim/slim-data-plane/) +
[SLIMRPC](https://docs.agntcy.org/slim/slim-rpc/)) for secure, feature-rich
communication.

## Prerequisites

To build the project and work with the code, you will need the following
installed in your system:

### [Taskfile](https://taskfile.dev/)

Taskfile is required to run all the build operations. Follow the
[installation](https://taskfile.dev/installation/) instructions in the Taskfile
documentations to find the best installation method for your system.

<details>
  <summary>with brew</summary>

```bash
brew install go-task
```

</details>
<details>
  <summary>with curl</summary>

```bash
sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b ~/.local/bin
```

</details>

### [Rust](https://rustup.rs/)

SLIM is implemented in Rust. Install with rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Quick Start

### Installation

SLIM consists of multiple components with different installation methods:

- **SLIM Node** (data plane): Docker, Cargo, or Helm
- **Control Plane**: Docker or Helm
- **slimctl CLI**: Download from [releases](https://github.com/agntcy/slim/releases?q=slimctl-v1.&expanded=true)
- **Language Bindings**: Python (pip), Go, C#, JavaScript/TypeScript, Kotlin

📦 **[Complete installation instructions](https://docs.agntcy.org/slim/slim-howto/)**

### Build

Set the build profile you prefer. Available options:

- debug
- release

```bash
PROFILE=release
# PROFILE=debug

task build PROFILE=${PROFILE}
```

This will build all SLIM binaries.

### Container Image

To run a multiarch image of SLIM (linux/arm64 & linux/amd64):

```bash
REPO_ROOT="$(git rev-parse --show-toplevel)"
docker build -t slim -f "${REPO_ROOT}/Dockerfile" --platform linux/amd64,linux/arm64 "${REPO_ROOT}"
```

Or alternatively, with docker buildx bake:

```bash
pushd $(git rev-parse --show-toplevel) && IMAGE_REPO=slim IMAGE_TAG=latest docker buildx bake slim && popd
```

<details>
  <summary>Container Image on Windows</summary>

The container image build process was tested on Windows 10 + Hyper-V and Windows 11 + WSL 2.
The instructions below assume [Powershell 7](<https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows?view=powershell-7.5>).

Set the repo root environment variable:

```Powershell
$env:REPO_ROOT = "C:\Users\<your-user>\slim"
```

Before building, ensure the Dockerfile uses **LF** line endings (not CRLF). In VSCode, click `CRLF` in the bottom-right corner and select `LF`, then save.

Build the container (arm64 is not supported on Windows):

```Powershell
docker buildx build `
    -t slim `
    -f "$env:REPO_ROOT\Dockerfile" `
    --platform linux/amd64 `
    "$env:REPO_ROOT"
```

</details>

## Run SLIM Node

SLIM is run as a binary (typically deployed as a workload in k8s). Language
bindings are maintained separately in the
[slim-bindings](https://github.com/agntcy/slim-bindings) repository.

SLIM can run in server mode, in client mode or both (i.e. spawning a
server and connecting to another SLIM instance at the same time).

### Server

To run SLIM binary as server, a configuration file is needed to setup the
basic runtime options. Some basic examples are provided in the
[config](./config/) folder:

- [reference](./config/reference/config.yaml) is a reference configuration, with
  comments explaining all the available options.
- [base](./config/base/server-config.yaml) is a base configuration for a server
  without encryption and authentication.
- [tls](./config/tls/server-config.yaml) is a configuration for a server with
  encryption enabled, with no authentication.
- [basic-auth](./config/basic-auth/server-config.yaml) is a configuration for a
  server with encryption and basic auth enabled.
- [jwt-auth-hmac](./config/jwt-auth-hmac/server-config.yaml) is a configuration
  for a server with JWT authentication using HMAC keys.
- [jwt-auth-rsa](./config/jwt-auth-rsa/server-config.yaml) is a configuration
  for a server with JWT authentication using RSA keys.
- [jwt-auth-ecdsa](./config/jwt-auth-ecdsa/server-config.yaml) is a configuration
  for a server with JWT authentication using ECDSA keys.
- [mtls](./config/mtls/server-config.yaml) is a configuration for a server
  expecting clients to authenticate with a trusted certificate.
- [spire](./config/spire/example-server.yaml) is a configuration for a server
  using SPIFFE/SPIRE for identity and authentication.
- [unix](./config/unix/server-config.yaml) is a configuration for a server
  listening on a Unix domain socket.
- [websocket](./config/websocket/server-config.yaml) is a configuration for a
  server using WebSocket transport.
- [proxy](./config/proxy/http.yaml) is a configuration for running behind an
  HTTP/HTTPS proxy.
- [logging](./config/logging/example-config.yaml) is a configuration showing
  logging options.
- [telemetry](./config/telemetry/server-config.yaml) is a configuration with
  OpenTelemetry integration.
- [east-west](./config/east-west/) is a multi-node configuration for east-west
  traffic between two SLIM nodes.
- [full-mesh](./config/full-mesh/) is a multi-node configuration for a
  full-mesh topology with multiple replicas.
- [exponential-backoff](./config/exponential-backoff/config.yaml) is a
  configuration demonstrating exponential backoff for reconnections.
- [fixed-interval-backoff](./config/fixed-interval-backoff/config.yaml) is a
  configuration demonstrating fixed-interval backoff for reconnections.

To run SLIM as server:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345  # Unix/Linux/macOS
# MODE=basic-auth; $env:PASSWORD = "12345"  # Windows PowerShell
# MODE=mtls

cargo run --bin slim -- --config ./config/${MODE}/server-config.yaml
```

Or, using the container image (assuming the image name is
`slim`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/server-config.yaml:/config.yaml \
    -p 46357:46357 \
    ghcr.io/agntcy/slim:latest /slim --config /config.yaml
```

---

### Client

To run the SLIM binary as client, you will need to configure it to start one
(or more) clients at startup, and you will need to provide the address of a
remote SLIM server. As usually, some configuration examples are available in
the [config](./config/) folder:

- [reference](./config/reference/config.yaml) is a reference configuration, with
  comments explaining all the available options.
- [base](./config/base/client-config.yaml) is a base configuration for a client
  without encryption and authentication.
- [tls](./config/tls/client-config.yaml) is a configuration for a client with
  encryption enabled, with no authentication.
- [basic-auth](./config/basic-auth/client-config.yaml) is a configuration for a
  client with encryption and basic auth enabled.
- [jwt-auth-hmac](./config/jwt-auth-hmac/client-config.yaml) is a configuration
  for a client with JWT authentication using HMAC keys.
- [jwt-auth-rsa](./config/jwt-auth-rsa/client-config.yaml) is a configuration
  for a client with JWT authentication using RSA keys.
- [jwt-auth-ecdsa](./config/jwt-auth-ecdsa/client-config.yaml) is a configuration
  for a client with JWT authentication using ECDSA keys.
- [mtls](./config/mtls/client-config.yaml) is a configuration for a client
  connecting to a server with a trusted certificate.
- [spire](./config/spire/example-client.yaml) is a configuration for a client
  using SPIFFE/SPIRE for identity and authentication.
- [unix](./config/unix/client-config.yaml) is a configuration for a client
  connecting via a Unix domain socket.
- [websocket](./config/websocket/client-config.yaml) is a configuration for a
  client using WebSocket transport.

To run SLIM as client:

```bash
MODE=base
# MODE=tls
# MODE=basic-auth; export PASSWORD=12345  # Unix/Linux/macOS
# MODE=basic-auth; $env:PASSWORD = "12345"  # Windows PowerShell
# MODE=mtls

cargo run --bin slim -- --config ./config/${MODE}/client-config.yaml
```

Or, using the container image (assuming the image name is
`slim`):

```bash
docker run -it \
    -e PASSWORD=${PASSWORD} \
    -v ./config/base/client-config.yaml:/config.yaml \
    ghcr.io/agntcy/slim:latest /slim --config /config.yaml
```

## Testing

Run the core Rust tests:

```bash
task test
```

## Linting

Run the linter for Rust code:

```bash
task lint
```

## Repo Structure

- **[crates](./crates)**: Rust workspace crates
    - [slim](./crates/slim): SLIM node binary for message forwarding
    - [datapath](./crates/datapath): Core data plane message routing
    - [proto](./crates/proto): Protobuf definitions and generated code
    - [controller](./crates/controller): Routing controller
    - [control-plane](./crates/control-plane): Control plane service
    - [channel-manager](./crates/channel-manager): Channel management
    - [session](./crates/session): Session layer with MLS encryption
    - [service](./crates/service): Service layer (SLIMRPC)
    - [auth](./crates/auth): Authentication and authorization
    - [config](./crates/config): Configuration management
    - [mls](./crates/mls): MLS protocol implementation
    - [slimctl](./crates/slimctl): CLI tool for SLIM management
    - [tracing](./crates/tracing): Tracing and observability
    - [version](./crates/version): Version information
    - [signal](./crates/signal): Signal handling
    - [testing](./crates/testing): Test utilities
    - [examples](./crates/examples): Example applications
    - Language bindings live in
      [slim-bindings](https://github.com/agntcy/slim-bindings):
      [Python](https://github.com/agntcy/slim-bindings/tree/main/python),
      [Go](https://github.com/agntcy/slim-bindings/tree/main/go),
      [Dotnet](https://github.com/agntcy/slim-bindings/tree/main/dotnet),
      [Kotlin](https://github.com/agntcy/slim-bindings/tree/main/kotlin),
      [Java](https://github.com/agntcy/slim-bindings/tree/main/java)

- **[charts](./charts)**: Kubernetes deployment
    - [slim](./charts/slim): Helm chart for data-plane nodes
    - [slim-control-plane](./charts/slim-control-plane): Helm chart for
      control-plane services

## Community & Resources

- 📚 [Documentation](https://docs.agntcy.org/slim/overview)
- 📖 [IETF Specification](https://datatracker.ietf.org/doc/draft-slim-protocol/)
- 💬 [Slack Community](https://join.slack.com/t/agntcy/shared_invite/)
- 🎥 [YouTube Channel](https://www.youtube.com/@agntcy-lf)

## License

[Copyright Notice and License](./LICENSE.md)

Distributed under Apache 2.0 License. See LICENSE for more information.

Copyright AGNTCY Contributors (https://github.com/agntcy)

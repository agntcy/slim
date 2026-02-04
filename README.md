[![Data-plane CI](https://github.com/agntcy/slim/actions/workflows/data-plane.yaml/badge.svg)](https://github.com/agntcy/slim/actions/workflows/data-plane.yaml)
[![Control-plane CI](https://github.com/agntcy/slim/actions/workflows/control-plane.yaml/badge.svg)](https://github.com/agntcy/slim/actions/workflows/control-plane.yaml)
[![codecov](https://codecov.io/gh/agntcy/slim/branch/main/graph/badge.svg)](https://codecov.io/gh/agntcy/slim)
[![Coverage](https://img.shields.io/badge/Coverage-passing-brightgreen)](https://codecov.io/gh/agntcy/slim)

# SLIM

**SLIM (Secure Low-Latency Interactive Messaging)** is a next-generation
communication framework that provides the secure, scalable transport layer for
AI agent protocols like [A2A (Agent-to-Agent)](https://a2a.ai) and
[MCP (Model Context Protocol)](https://modelcontextprotocol.io).

- 📖 **[Read the full documentation](https://docs.agntcy.org/slim/overview/)**
- 🎓 **[Get Started](https://docs.agntcy.org/slim/slim-howto)**
- 💻 **[Code Examples](./data-plane/bindings)** - 
  [Python](./data-plane/bindings/python/examples) | 
  [Go](./data-plane/bindings/go/examples)
- 🔌 **[Integrations](https://docs.agntcy.org/slim/slim-rpc/)** - 
  [A2A](https://github.com/agntcy/slim-a2a-python) | 
  [MCP](https://github.com/agntcy/slim-mcp-python) | 
  [OpenTelemetry](https://github.com/agntcy/slim-otel)
- 🚀 **[Deployment Strategies](./deployments/readme.md)**
- 📝 **[Technical blog posts](https://blogs.agntcy.org)**

## Architecture

SLIM uses a distributed architecture with three main components:

- **Data Plane**: Pure message routing layer that forwards packets based on
  hierarchical names without inspecting application content
- **Session Layer**: Handles reliable delivery, end-to-end MLS encryption, and
  group membership management
- **Control Plane**: Manages configuration, monitoring, and orchestration of
  SLIM routing nodes

This separation enables efficient deployment: SLIM routing nodes run only the
lightweight data plane, while applications use language bindings with the full
stack (data plane client + session layer + SRPC) for secure, feature-rich
communication.

## Quick Start

### Installation

SLIM consists of multiple components with different installation methods:

- **SLIM Node** (data plane): Docker, Cargo, or Helm
- **Control Plane**: Docker or Helm
- **slimctl CLI**: Download from releases
- **Language Bindings**: Python (pip), Go, C#, JavaScript/TypeScript, Kotlin

📦 **[Complete installation instructions](https://docs.agntcy.org/slim/slim-howto/)**

### Building from Source

Build all components using [Taskfile](https://taskfile.dev/):

```bash
# Build data-plane (Rust)
task data-plane:build PROFILE=release

# Build control-plane (Go)
task control-plane:build

# Run tests
task data-plane:test
task control-plane:test
```

### Running SLIM

Start a SLIM server node:

```bash
# Run with basic configuration
cd data-plane && cargo run --bin slim -- --config ./config/base/server-config.yaml

# Or using Docker
docker run -it \
    -v ./data-plane/config/base/server-config.yaml:/config.yaml \
    -p 46357:46357 \
    ghcr.io/agntcy/slim:latest /slim --config /config.yaml
```

See the [data-plane README](./data-plane/README.md) for detailed configuration
options including TLS, authentication, and mTLS.

## Components

- **[data-plane](./data-plane)**: Rust-powered message routing and client
  libraries
    - SLIM node binary for message forwarding
    - Session layer with MLS encryption
    - SRPC (SLIM RPC) for request-response patterns
    - Language bindings: [Python](./data-plane/python/bindings), Go
      (coming soon)

- **[control-plane](./control-plane)**: Go-based management services
    - Configuration management for SLIM nodes
    - `slimctl` CLI tool for operations

- **[charts](./charts)**: Kubernetes deployment
    - [slim](./charts/slim): Helm chart for data-plane nodes
    - [slim-control-plane](./charts/slim-control-plane): Helm chart for
      control-plane services

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

The data-plane components are implemented in rust. Install with rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### [Golang](https://go.dev/doc/install)

The control-plane components are implemented in golang. Follow the installation
instructions in the golang website.

## Use Cases

- **Multi-agent AI systems**: Enable secure communication between specialized
  agents collaborating on complex tasks
- **A2A Protocol Transport**: Serve as the secure transport layer for A2A SDKs
  across Python, JavaScript, Java, C#, and Go
- **MCP Integration**: Support Model Context Protocol for context sharing
  between agents and tools
- **Distributed AI workflows**: Orchestrate agents across data centers, edge
  devices, and browsers
- **Real-time agent collaboration**: Low-latency communication for interactive
  agent coordination

## Community & Resources

- 📚 [Documentation](https://docs.agntcy.org/slim/overview)
- 🐙 [GitHub Repository](https://github.com/agntcy/slim)
- 📖 [IETF Specification](https://datatracker.ietf.org/doc/draft-slim-protocol/)
- 💬 [Slack Community](https://join.slack.com/t/agntcy/shared_invite/)
- 🎥 [YouTube Channel](https://www.youtube.com/@agntcy)

## License

[Copyright Notice and License](./LICENSE.md)

Distributed under Apache 2.0 License. See LICENSE for more information.

Copyright AGNTCY Contributors (https://github.com/agntcy)

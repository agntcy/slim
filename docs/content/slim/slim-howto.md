---
icon: material/play-circle-outline
---

# Getting Started with SLIM

This guide walks you through installing and running all SLIM components. For detailed installation instructions for a specific component, see the links in each section.

## Components

SLIM consists of four components. Install the ones you need:

| Component | What it does | Installation |
|-----------|-------------|-------------|
| **SLIM Node** (Data Plane) | Routes and forwards messages | [Data Plane Installation](./components/data-plane/install.md) |
| **SLIM Controller** | Manages and configures SLIM nodes | [Controller Installation](./components/controller/install.md) |
| **slimctl** | CLI for operators and local development | [SLIM CLI Installation](./components/cli/install.md) |
| **SLIM SDK** (Bindings) | Integrate SLIM into your applications | [SDK Installation](./components/sdk/install.md) |

## Quick Start

The fastest way to get a SLIM node running locally is with `slimctl`:

**1. Install slimctl**

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_darwin_arm64.tar.gz
    tar -xzf slimctl_1.2.0_darwin_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "macOS (Intel)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_darwin_amd64.tar.gz
    tar -xzf slimctl_1.2.0_darwin_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "Linux (AMD64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_linux_amd64.tar.gz
    tar -xzf slimctl_1.2.0_linux_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "macOS (Homebrew)"

    ```bash
    brew tap agntcy/slim https://github.com/agntcy/slim.git
    brew install slimctl
    ```

See [SLIM CLI Installation](./components/cli/install.md) for all platforms and build-from-source instructions.

**2. Start a SLIM Node**

```bash
slimctl slim start
```

This starts a SLIM node on `0.0.0.0:46357` with default settings (no TLS). The node is ready to accept connections from SDK applications.

**3. Install the SDK for your language**

=== "Python"

    ```bash
    pip install slim-bindings
    ```

=== "Go"

    ```bash
    go get github.com/agntcy/slim-bindings-go@v1.1.0
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
    ```

=== "Kotlin"

    ```kotlin
    // build.gradle.kts
    dependencies {
        implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
    }
    ```

=== ".NET"

    ```bash
    dotnet add package Agntcy.Slim.Bindings
    ```

See [SDK Installation](./components/sdk/install.md) for full per-language instructions.

**4. Connect your application**

Follow the [SDK tutorials](./components/sdk/tutorial-connect.md) to connect your application to the running SLIM node.

## Building from Source

To build all components from source:

**Prerequisites**: [Taskfile](https://taskfile.dev/), [Rust](https://rustup.rs/), [Go](https://go.dev/doc/install)

```bash
git clone https://github.com/agntcy/slim
cd slim

# Build the data plane (Rust)
task data-plane:build

# Build the control plane (Go)
task control-plane:build
```

## Next Steps

- [Architecture](./architecture/index.md) — Understand how the components work together
- [SDK Tutorials](./components/sdk/tutorial-connect.md) — Build your first SLIM application
- [Configuration Reference](./components/data-plane/config.md) — Full SLIM node configuration options
- [Community](../community.md) — Get help and connect with other SLIM users

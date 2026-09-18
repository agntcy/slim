---
icon: material/play-circle-outline
---

# Getting Started

By the end of this guide you will have a SLIM node running locally and an application connected to it, ready to start exchanging messages.

**Time to complete:** ~5 minutes

## Prerequisites

- macOS, Linux, or Windows
- One of: Python 3.9+, Go 1.21+, Java 17+, Kotlin, Node.js 18+, .NET 8+

## Step 1: Install slimctl

`slimctl` is the quickest way to start a local SLIM node. Install it for your platform:

=== "macOS (Homebrew)"

    ```bash
    brew tap agntcy/slim https://github.com/agntcy/slim.git
    brew install slimctl
    ```

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-darwin-arm64.tar.gz
    tar -xzf slimctl-darwin-arm64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

    !!! note "Gatekeeper"
        If macOS blocks the binary, run:
        ```bash
        sudo xattr -rd com.apple.quarantine /usr/local/bin/slimctl
        ```

=== "macOS (Intel)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-darwin-amd64.tar.gz
    tar -xzf slimctl-darwin-amd64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

    !!! note "Gatekeeper"
        If macOS blocks the binary, run:
        ```bash
        sudo xattr -rd com.apple.quarantine /usr/local/bin/slimctl
        ```

=== "Linux (AMD64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-linux-amd64-gnu.tar.gz
    tar -xzf slimctl-linux-amd64-gnu.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

=== "Linux (ARM64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-linux-arm64-gnu.tar.gz
    tar -xzf slimctl-linux-arm64-gnu.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

=== "Windows"

    Download the binary from the [GitHub releases page](https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-windows-amd64.zip) and add it to your `PATH`. See [CLI Installation](./components/cli/install.md) for full Windows instructions.

Verify the install:

```bash
slimctl help
```

See [CLI Installation](./components/cli/install.md) for build-from-source instructions.

## Step 2: Start a SLIM node

```bash
slimctl slim start
```

This starts a SLIM node on `127.0.0.1:46357` with insecure (no TLS) defaults. The node runs in the foreground — press **Ctrl-C** to stop it.

## Step 3: Install the SDK

Install the SLIM bindings for your language:

=== "Python"

    ```bash
    pip install slim-bindings
    ```

=== "Go"

    ```bash
    go get github.com/agntcy/slim-bindings-go@v2.0.0
    # Run the setup tool to download the native library
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
    ```

=== "Java"

    ```kotlin
    // build.gradle.kts
    dependencies {
        implementation("io.agntcy.slim:slim-bindings-java:2.0.0")
    }
    ```

=== "Kotlin"

    ```kotlin
    // build.gradle.kts
    dependencies {
        implementation("io.agntcy.slim:slim-bindings-kotlin:2.0.0")
    }
    ```

=== "Node.js"

    ```bash
    npm install @agntcy/slim-bindings
    ```

=== ".NET"

    ```bash
    dotnet add package Agntcy.Slim
    ```

    Or add to your `.csproj`:

    ```xml
    <PackageReference Include="Agntcy.Slim" Version="2.0.0" />
    ```

    The NuGet package includes native libraries for all supported platforms — no additional setup required. See the [.NET SDK guide](./components/sdk/dotnet.md) for API details and examples.

=== "React Native"

    ```bash
    npm install @agntcy/slim-bindings-react-native
    ```

See [SDK Installation](./components/sdk/install.md) for full per-language setup details.

## Step 4: Connect your first app

With the node running, initialise the SLIM service, connect, and register an application identity:

{% include-markdown "slim/components/sdk/tutorials/_snippets/putting-it-together.md" %}

!!! note "Insecure mode"
    `new_insecure_client_config` skips TLS and is for local development only. See [Authentication](./architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

## Next Steps

You have a running SLIM node and a connected application. Continue with the SDK tutorials to start exchanging messages:

- [Creating an App](./components/sdk/tutorials/tutorial-app.md) — register application identities for both sides of a conversation
- [Creating a Session](./components/sdk/tutorials/tutorial-session.md) — open a point-to-point or group session and send your first message
- [Receiving a Session](./components/sdk/tutorials/tutorial-receive.md) — listen for incoming sessions, receive messages, and reply

Or explore further:

- [Architecture](./architecture/index.md) — understand the four-layer SLIM stack
- [Deployment](./deploy/index.md) — deploy SLIM with Docker or Kubernetes
- [Authentication](./architecture/authentication.md) — secure your connections

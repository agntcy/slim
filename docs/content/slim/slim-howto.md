---
icon: material/play-circle-outline
---

# Getting Started

By the end of this guide you will have a SLIM node running locally and a Python or Go application connected to it, ready to start exchanging messages.

**Time to complete:** ~5 minutes

## Prerequisites

- macOS or Linux (Windows supported via pre-built binaries)
- Python 3.9+ **or** Go 1.21+ for the SDK step

## Step 1: Install slimctl

`slimctl` is the quickest way to start a local SLIM node. Install it for your platform:

=== "macOS (Homebrew)"

    ```bash
    brew tap agntcy/slim https://github.com/agntcy/slim.git
    brew install slimctl
    ```

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_darwin_arm64.tar.gz
    tar -xzf slimctl_1.2.0_darwin_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

    !!! note "Gatekeeper"
        If macOS blocks the binary, run:
        ```bash
        sudo xattr -rd com.apple.quarantine /usr/local/bin/slimctl
        ```

=== "macOS (Intel)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_darwin_amd64.tar.gz
    tar -xzf slimctl_1.2.0_darwin_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

=== "Linux (AMD64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_linux_amd64.tar.gz
    tar -xzf slimctl_1.2.0_linux_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

=== "Linux (ARM64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_linux_arm64.tar.gz
    tar -xzf slimctl_1.2.0_linux_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/
    ```

Verify the install:

```bash
slimctl help
```

See [CLI Installation](./components/cli/install.md) for Windows binaries and build-from-source instructions.

## Step 2: Start a SLIM node

```bash
slimctl slim start
```

This starts a SLIM node on `0.0.0.0:46357` with insecure (no TLS) defaults. The node is ready immediately.

```bash
# Check it is running
slimctl slim status
```

To stop it later:

```bash
slimctl slim stop
```

## Step 3: Install the SDK

Install the SLIM bindings for your language:

=== "Python"

    ```bash
    pip install slim-bindings
    ```

=== "Go"

    ```bash
    go get github.com/agntcy/slim-bindings-go@v1.1.0
    # Run the setup tool to download the native library
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

See [SDK Installation](./components/sdk/install.md) for full per-language setup details.

## Step 4: Connect your first app

With the node running, initialise the SLIM service and connect:

=== "Python"

    ```python
    import asyncio
    import slim_bindings

    async def main():
        # Required for UniFFI async bindings
        slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())

        # Initialise the global runtime
        slim_bindings.initialize_with_defaults()
        service = slim_bindings.get_global_service()

        # Connect to the local SLIM node
        conn_id = await service.connect_async(
            slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")
        )

        # Register an application identity
        local_name = slim_bindings.Name("myorg", "default", "my-app")
        app = service.create_app_with_secret(local_name, "my-shared-secret")

        print(f"Connected — app id: {app.id()}")

    asyncio.run(main())
    ```

=== "Go"

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) in the slim-bindings-go repository for the equivalent connection pattern.

Running the Python script should print your app's SLIM identity:

```text
Connected — app id: myorg/default/my-app/<client-id>
```

!!! note "Insecure mode"
    `new_insecure_client_config` skips TLS and is for local development only. See [Authentication](./architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

## Next Steps

You have a running SLIM node and a connected application. Continue with the SDK tutorials to start exchanging messages:

- [Creating an App](./components/sdk/tutorial-app.md) — register application identities for both sides of a conversation
- [Creating a Session](./components/sdk/tutorial-session.md) — open a point-to-point or group session and send your first message
- [Receiving a Session](./components/sdk/tutorial-receive.md) — listen for incoming sessions, receive messages, and reply

Or explore further:

- [Architecture](./architecture/index.md) — understand the four-layer SLIM stack
- [Deployment](./deploy/index.md) — deploy SLIM with Docker or Kubernetes
- [Authentication](./architecture/authentication.md) — secure your connections

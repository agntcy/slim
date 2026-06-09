# Getting Started with SLIM

## Installation

SLIM is composed of multiple components, each with its own installation instructions. Choose the components you need based on your use case.

### SLIM Node

The SLIM Node is the core component that handles messaging operations.

You can install the SLIM Node using Docker, Cargo, Helm, or the CLI binary. Choose the method that best fits your infrastructure.

=== "Docker"

    Pull the SLIM container image and run it with a configuration file:

    ```bash
    docker pull ghcr.io/agntcy/slim:1.0.0
    ```

    Create a configuration file:

    ```yaml
    # config.yaml
    tracing:
      log_level: info
      display_thread_names: true
      display_thread_ids: true

    runtime:
      n_cores: 0
      thread_name: "slim-data-plane"
      drain_timeout: 10s

    services:
      slim/0:
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                insecure: true

          clients: []
    ```

    Run the container:

    ```bash
    docker run -it \
        -v ./config.yaml:/config.yaml -p 46357:46357 \
        ghcr.io/agntcy/slim:1.0.0 /slim --config /config.yaml
    ```

=== "Cargo"

    Install SLIM using Rust's package manager:

    ```bash
    RUSTFLAGS="--cfg mls_build_async" cargo install agntcy-slim
    ```

    Create a configuration file:

    ```yaml
    # config.yaml
    tracing:
      log_level: info
      display_thread_names: true
      display_thread_ids: true

    runtime:
      n_cores: 0
      thread_name: "slim-data-plane"
      drain_timeout: 10s

    services:
      slim/0:
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                insecure: true

          clients: []
    ```

    Run SLIM:

    ```bash
    ~/.cargo/bin/slim --config ./config.yaml
    ```

=== "Helm"

    For Kubernetes deployments, use the official Helm chart:

    ```bash
    helm pull oci://ghcr.io/agntcy/slim/helm/slim --version v1.1.0
    ```

    !!! note "Configuration"
        For detailed configuration options, see the [values.yaml](https://github.com/agntcy/slim/blob/slim-v1.1.0/charts/slim/values.yaml) in the repository.

=== "CLI Binary"

    For local development and testing, use the `slimctl` binary.
    
    Install the `slimctl` binary following the [instructions below](#slimctl).

    === "Default Configuration"

        Run with default settings:

        ```bash
        slimctl slim start
        ```

    === "Custom Configuration"

        Create a configuration file:

        ```yaml
        # config.yaml
        tracing:
          log_level: info
          display_thread_names: true
          display_thread_ids: true

        runtime:
          n_cores: 0
          thread_name: "slim-data-plane"
          drain_timeout: 10s

        services:
          slim/0:
            dataplane:
              servers:
                - endpoint: "0.0.0.0:46357"
                  tls:
                    insecure: true

              clients: []
        ```

        Start SLIM with the configuration:

        ```bash
        slimctl slim start --config ./config.yaml
        ```

For more configuration options, see the [SLIM Configuration reference](./slim-data-plane-config.md).

### SLIM Controller

The SLIM Controller manages SLIM Nodes and provides a user-friendly interface for configuration.

=== "Docker"

    Pull the controller image:

    ```bash
    docker pull ghcr.io/agntcy/slim/control-plane:1.0.0
    ```

    Create a configuration file:

    ```yaml
    # slim-control-plane.yaml
    northbound:
      httpHost: 0.0.0.0
      httpPort: 50051
      logging:
        level: INFO

    southbound:
      httpHost: 0.0.0.0
      httpPort: 50052
      logging:
        level: INFO

    reconciler:
      maxRequeues: 15
      maxNumOfParallelReconciles: 1000

    logging:
      level: INFO

    database:
      filePath: /db/controlplane.db
    ```

    Run the controller:

    ```bash
    docker run -it \
        -v ./slim-control-plane.yaml:/config.yaml -v .:/db \
        -p 50051:50051 -p 50052:50052                      \
        ghcr.io/agntcy/slim/control-plane:1.0.0           \
        -config /config.yaml
    ```

=== "Helm"

    For Kubernetes deployments:

    ```bash
    helm pull oci://ghcr.io/agntcy/slim/helm/slim-control-plane --version v1.1.0
    ```

### SLIM Bindings

Language bindings allow you to integrate SLIM with your applications.

=== "Python"

    Install using pip:

    ```bash
    pip install slim-bindings
    ```

    Or add to your `pyproject.toml`:

    ```toml
    [project]
    # ...
    dependencies = ["slim-bindings~=1.0"]
    ```

    For more information on the SLIM bindings, see the [Messaging Layer Tutorial](./slim-data-plane.md) and the [Python Examples](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples).

=== "Go"

    Install the Go bindings:

    ```bash
    go get github.com/agntcy/slim-bindings-go@v1.1.0
    ```

    Run the setup tool to install native libraries:

    ```bash
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
    ```

    Add to your `go.mod`:

    ```go
    require github.com/agntcy/slim-bindings-go v1.1.0
    ```

    !!! warning "C Compiler Required"
        The Go bindings use native libraries via [CGO](https://pkg.go.dev/cmd/cgo), so you'll need a C compiler installed on your system.

    For more information on the Go bindings, see the [Go Examples](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/go/examples).

=== "Kotlin"

    Add the Kotlin bindings to your Gradle project:

    === "Maven Central"

        Add to your `build.gradle.kts`:

        ```kotlin
        dependencies {
            implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
        }
        ```

        `mavenCentral()` is the default repository in Gradle, so no additional repository configuration is needed.

    === "GitHub Packages"

        Add the GitHub Packages repository and dependency to your `build.gradle.kts`:

        ```kotlin
        repositories {
            maven {
                url = uri("https://maven.pkg.github.com/agntcy/slim")
                credentials {
                    username = project.findProperty("gpr.user") as String? ?: System.getenv("GITHUB_ACTOR")
                    password = project.findProperty("gpr.key") as String? ?: System.getenv("GITHUB_TOKEN")
                }
            }
        }
        dependencies {
            implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
        }
        ```

        !!! note "GitHub Token Required"
            To use GitHub Packages, you need a personal access token with `read:packages` scope. Set `GITHUB_ACTOR` (your username) and `GITHUB_TOKEN` (your token) as environment variables, or use `gpr.user` and `gpr.key` in `gradle.properties`.

    !!! note "JDK 17+ Required"
        The Kotlin bindings use [JNA](https://github.com/java-native-access/jna) for native library loading and require JDK 17 or higher.

    For more information on the Kotlin bindings, see the [Kotlin Examples](https://github.com/agntcy/slim/tree/slim-bindings-v1.2.0/data-plane/bindings/kotlin/examples).

### Slimctl

`slimctl` is a command-line tool for managing SLIM Nodes and Controllers.

#### Installation

Choose your platform:

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_darwin_arm64.tar.gz
    tar -xzf slimctl_1.2.0_darwin_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

    !!! warning "macOS Security"
        You may need to allow the binary to run if blocked by Gatekeeper:

        ```bash
        sudo xattr -rd com.apple.quarantine /usr/local/bin/slimctl
        ```

        Alternatively, go to **System Settings > Privacy & Security** and allow the application when prompted.

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

=== "Linux (ARM64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl_1.2.0_linux_arm64.tar.gz
    tar -xzf slimctl_1.2.0_linux_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "Windows (AMD64)"

    Download and extract the Windows binary:

    ```powershell
    # Using PowerShell
    Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-windows-amd64.zip" -OutFile "slimctl.zip"
    Expand-Archive -Path "slimctl.zip" -DestinationPath "."
    
    # Move to a directory in your PATH (e.g., C:\Program Files\slimctl\)
    # Or add the current directory to your PATH
    ```

    Alternatively, download directly from the [releases page](https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-windows-amd64.zip).

=== "Windows (ARM64)"

    Download and extract the Windows binary:

    ```powershell
    # Using PowerShell
    Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-windows-arm64.zip" -OutFile "slimctl.zip"
    Expand-Archive -Path "slimctl.zip" -DestinationPath "."
    
    # Move to a directory in your PATH (e.g., C:\Program Files\slimctl\)
    # Or add the current directory to your PATH
    ```

    Alternatively, download directly from the [releases page](https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-windows-arm64.zip).

Check the [slimctl documentation](./slim-controller.md) for additional installation methods.

#### Verification

Verify the installation:

```bash
slimctl help
```

This should display help information and available commands.

## Building from Source

You can build SLIM from source.

### Prerequisites

Install the following tools on your system:

- [Taskfile](https://taskfile.dev/)
- [Rust](https://rustup.rs/)
- [Go](https://go.dev/doc/install)

### Building SLIM

Once all prerequisites are installed, clone the repository and build the components:

```bash
# Clone the SLIM repository
git clone https://github.com/agntcy/slim
cd slim

# Build the data plane (Rust)
task data-plane:build

# Build the control plane (Go)
task control-plane:build
```

For more information on the build system and development workflow, see the [SLIM repository](https://github.com/agntcy/slim).

## Next Steps

You've installed SLIM! Here's what to do next:

1. Read the [messaging layer documentation](./slim-data-plane.md)
2. Explore the [example applications](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/)
3. Learn about [configuration options](./slim-data-plane-config.md)
4. Join us on [Slack](https://join.slack.com/t/agntcy/shared_invite/zt-3hb4p7bo0-5H2otGjxGt9OQ1g5jzK_GQ)

## Need Help?

If you get stuck, check the [detailed documentation](../index.md), ask questions in our [community forums](https://join.slack.com/t/agntcy/shared_invite/zt-3hb4p7bo0-5H2otGjxGt9OQ1g5jzK_GQ), or report issues on [GitHub](https://github.com/agntcy/slim).

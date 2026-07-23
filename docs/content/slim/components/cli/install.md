# SLIM CLI Installation

`slimctl` is a command-line tool for managing SLIM Nodes and Controllers. It supports local development, route management, connection monitoring, and direct node access.

## Pre-built Binaries

Download the binary for your platform from the [GitHub releases page](https://github.com/agntcy/slim/releases).

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl_2.0.0-alpha.7_darwin_arm64.tar.gz
    tar -xzf slimctl_2.0.0-alpha.7_darwin_arm64.tar.gz
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
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl_2.0.0-alpha.7_darwin_amd64.tar.gz
    tar -xzf slimctl_2.0.0-alpha.7_darwin_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "Linux (AMD64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl_2.0.0-alpha.7_linux_amd64.tar.gz
    tar -xzf slimctl_2.0.0-alpha.7_linux_amd64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "Linux (ARM64)"

    ```bash
    curl -LO https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl_2.0.0-alpha.7_linux_arm64.tar.gz
    tar -xzf slimctl_2.0.0-alpha.7_linux_arm64.tar.gz
    sudo mv slimctl /usr/local/bin/slimctl
    sudo chmod +x /usr/local/bin/slimctl
    ```

=== "Windows (AMD64)"

    ```powershell
    Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl-windows-amd64.zip" -OutFile "slimctl.zip"
    Expand-Archive -Path "slimctl.zip" -DestinationPath "."

    # Move to a directory in your PATH (e.g., C:\Program Files\slimctl\)
    ```

    Alternatively, download directly from the [releases page](https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl-windows-amd64.zip).

=== "Windows (ARM64)"

    ```powershell
    Invoke-WebRequest -Uri "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl-windows-arm64.zip" -OutFile "slimctl.zip"
    Expand-Archive -Path "slimctl.zip" -DestinationPath "."

    # Move to a directory in your PATH (e.g., C:\Program Files\slimctl\)
    ```

    Alternatively, download directly from the [releases page](https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.7/slimctl-windows-arm64.zip).

## Homebrew (macOS)

```bash
brew tap agntcy/slim https://github.com/agntcy/slim.git
brew install slimctl
```

## Building from Source

**Prerequisites**: Go 1.24+, [Taskfile](https://taskfile.dev/)

```bash
# From repository root
cd control-plane
task control-plane:slimctl:build

# Binary location: .dist/bin/slimctl
```

## Verify Installation

```bash
slimctl help
```

This displays help information and available commands.

## Configuration

`slimctl` reads its configuration from:

- `$HOME/.slimctl/config.yaml`
- `./config.yaml` (current directory)
- Via `--config` flag

An example `config.yaml` for connecting to a Controller:

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

## Next Steps

- [Command Reference](./reference/index.md) — Full reference for all `slimctl` commands
- [SLIM Controller Overview](../controller/index.md) — Learn how the Controller manages SLIM nodes

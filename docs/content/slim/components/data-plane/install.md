# SLIM Data Plane Installation

The SLIM Data Plane is the core routing and forwarding component. Install it using Docker, Cargo, Helm, or the `slimctl` CLI binary.

!!! tip "Getting Started"
    For a quickstart covering all SLIM components together, see the [Getting Started](../../slim-howto.md) guide.

## Docker

Pull the SLIM container image and run it with a configuration file:

```bash
docker pull ghcr.io/agntcy/slim:2.0.0-alpha.7
```

Create a minimal configuration file:

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
    ghcr.io/agntcy/slim:2.0.0-alpha.7 /slim --config /config.yaml
```

## Cargo

Install SLIM using Rust's package manager:

```bash
RUSTFLAGS="--cfg mls_build_async" cargo install agntcy-slim
```

Create a configuration file (see the Docker example above), then run SLIM:

```bash
~/.cargo/bin/slim --config ./config.yaml
```

## Helm

For Kubernetes deployments, use the official Helm chart:

```bash
helm pull oci://ghcr.io/agntcy/slim/helm/slim --version v2.0.0-alpha.7
```

!!! note "Configuration"
    For detailed Helm configuration options, see the [values.yaml](https://github.com/agntcy/slim/blob/slim-v2.0.0-alpha.7/charts/slim/values.yaml) in the repository.

## CLI Binary (`slimctl`)

For local development and testing, use the `slimctl` binary. First [install slimctl](../cli/install.md), then:

=== "Default Configuration"

    Run with default settings:

    ```bash
    slimctl slim start
    ```

=== "Custom Configuration"

    Create a configuration file and start SLIM with it:

    ```bash
    slimctl slim start --config ./config.yaml
    ```

## Building from Source

**Prerequisites**: Rust toolchain (pinned to 1.93.0), [Taskfile](https://taskfile.dev/)

```bash
# Clone the repository
git clone https://github.com/agntcy/slim
cd slim

# Build the data plane (release build)
task data-plane:build PROFILE=release
```

The binary will be available at `data-plane/target/release/slim`.

## Next Steps

- [Configuration Reference](./config.md) — Full YAML configuration reference for all data plane options
- [Getting Started](../../slim-howto.md) — Install all SLIM components and run a first example

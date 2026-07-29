# SLIM Controller Installation

The SLIM Controller is the management component for SLIM infrastructure. Install it using Docker, Helm, or build from source.

!!! tip "Getting Started"
    For a quickstart covering all SLIM components together, see the [Getting Started](../../slim-howto.md) guide.

## Docker

Pull the controller image:

```bash
docker pull ghcr.io/agntcy/slim/control-plane:2.0.0-alpha.7
```

Create a configuration file:

```yaml
# slim-control-plane.yaml
northbound:
  endpoint: "0.0.0.0:50051"

southbound:
  endpoint: "0.0.0.0:50052"

reconciler:
  max_requeues: 15
  workers: 4

tracing:
  log_level: info

database:
  type: sqlite
  path: /db/controlplane.db
```

Run the controller:

```bash
docker run -it \
    -v ./slim-control-plane.yaml:/config.yaml -v .:/db \
    -p 50051:50051 -p 50052:50052                      \
    ghcr.io/agntcy/slim/control-plane:2.0.0-alpha.7    \
    -config /config.yaml
```

## Helm

For Kubernetes deployments:

```bash
helm pull oci://ghcr.io/agntcy/slim/helm/slim-control-plane --version v2.0.0-alpha.7
```

## Building from Source

**Prerequisites**: Rust toolchain (pinned to 1.95.0), [Taskfile](https://taskfile.dev/)

```bash
# Clone the repository
git clone https://github.com/agntcy/slim
cd slim

# Build the controller binary
task control-plane:build
```

Start the controller directly:

```bash
cargo run --bin slim-control-plane
```

## Next Steps

- [Configuration Reference](./config.md) — Full YAML configuration reference for the Controller
- [SLIM CLI Installation](../cli/install.md) — Install `slimctl` to manage the Controller
- [SLIM Controller Overview](./index.md) — Learn how the Controller manages SLIM nodes

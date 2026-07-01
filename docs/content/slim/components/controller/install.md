# SLIM Controller Installation

The SLIM Controller is the management component for SLIM infrastructure. Install it using Docker, Helm, or build from source.

!!! tip "Getting Started"
    For a quickstart covering all SLIM components together, see the [Getting Started](../../slim-howto.md) guide.

## Docker

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

## Helm

For Kubernetes deployments:

```bash
helm pull oci://ghcr.io/agntcy/slim/helm/slim-control-plane --version v1.1.0
```

## Building from Source

**Prerequisites**: Go 1.24+, [Taskfile](https://taskfile.dev/)

```bash
# Clone the repository
git clone https://github.com/agntcy/slim
cd slim

# Build all control plane binaries
task control-plane:build

# Or build just the controller binary
task control-plane:control-plane:build
```

Start the controller directly using the task runner:

```bash
task control-plane:control-plane:run
```

## Next Steps

- [Configuration Reference](./config.md) — Full YAML configuration reference for the Controller
- [SLIM CLI Installation](../cli/install.md) — Install `slimctl` to manage the Controller
- [SLIM Controller Overview](./index.md) — Learn how the Controller manages SLIM nodes

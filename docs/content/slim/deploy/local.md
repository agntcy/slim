# Deployment: Local

Running SLIM as a local binary is the simplest way to deploy a SLIM node on a single machine. This is well suited for running local agents — such as coding agents, automation tools, or any application services that need to communicate on a single host. No Kubernetes or Docker required.

## Prerequisites

- `slimctl` installed — see [SLIM CLI Installation](../components/cli/install.md)
- Or the `slim` binary built from source — see [Data Plane Installation](../components/data-plane/install.md)

## Option 1: Quick Start with slimctl

The fastest path: `slimctl slim start` launches a SLIM node on `0.0.0.0:46357` with sensible defaults.

```bash
slimctl slim start
```

The node is ready immediately. Connect SDK applications to `http://127.0.0.1:46357`.

To stop the node:

```bash
slimctl slim stop
```

## Option 2: Run with a Config File

For more control, create a configuration file and run the `slim` binary directly.

**Minimal config** (`config.yaml`):

```yaml
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
    node_id: slim-local
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true
      clients: []
```

Run the node:

```bash
./slim --config config.yaml
```

Or with Cargo after building from source:

```bash
cargo run -p agntcy-slim -- --config config.yaml
```

## Running Multiple Local Nodes

To test multi-node routing locally, run two SLIM instances on different ports and connect them as peers.

**Node A** (`node-a.yaml`):

```yaml
services:
  slim/0:
    node_id: node-a
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true
      clients:
        - endpoint: "http://127.0.0.1:46358"
          tls:
            insecure: true
```

**Node B** (`node-b.yaml`):

```yaml
services:
  slim/0:
    node_id: node-b
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46358"
          tls:
            insecure: true
      clients: []
```

Start both:

```bash
./slim --config node-a.yaml &
./slim --config node-b.yaml &
```

Applications connected to either node can now route messages to each other.

## Running the Channel Manager Locally

To use operator-managed group channels locally, run the Channel Manager alongside the SLIM node. See [Channel Manager Installation](../components/channel-manager/install.md) for build instructions.

```bash
# Start the SLIM node first
./slim --config config.yaml &

# Then start the Channel Manager
./channel-manager --config-file channel-manager.yaml
```

Minimal Channel Manager config pointing at the local node:

```yaml
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true
  local-name: "agntcy/local/channel-manager"
  auth:
    type: shared_secret
    secret: "dev-secret-replace-in-production"
```

## Next Steps

- [SDK Tutorials](../components/sdk/tutorials/tutorial-connect.md) — Connect your application to the running node
- [Data Plane Configuration Reference](../components/data-plane/config.md) — All available config options
- [Docker Deployment](./docker.md) — Run SLIM in containers

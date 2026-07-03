# Control Plane Configuration

Reference for configuring the SLIM control plane (`slim-control-plane` binary).
The control plane coordinates SLIM data-plane nodes: node registration, topology
management, link creation, and route reconciliation.

For operational commands, see [Controller Reference](./slim-controller-reference.md).
For an architecture overview, see [SLIM Controller](./slim-controller.md).

## Configuration file

The control plane loads configuration from:

1. `-c` / `--config <file>` if specified
2. `./config.yaml` in the working directory
3. Built-in defaults

Example minimal configuration:

```yaml
tracing:
  log_level: info

northbound:
  endpoint: "0.0.0.0:50051"
  tls:
    insecure: true

southbound:
  endpoint: "0.0.0.0:50052"
  tls:
    insecure: true

database:
  type: sqlite
  path: /var/lib/slim/controlplane.db

reconciler:
  max_requeues: 15
  base_retry_delay: 200ms
  reconcile_period: 60s
  enable_orphan_detection: false
  workers: 4
```

## Top-level sections

### `tracing`

```yaml
tracing:
  log_level: info  # trace, debug, info, warn, error
```

Uses the same tracing configuration as SLIM nodes. See
[Data Plane Configuration](./slim-data-plane-config.md#tracing-configuration).

### `northbound` and `southbound`

Both use the same `ServerConfig` structure as data-plane servers:

| Endpoint | Default | Purpose |
|----------|---------|---------|
| `northbound` | `0.0.0.0:50051` | Management API (`slimctl controller`) |
| `southbound` | `0.0.0.0:50052` | Node registration and configuration push |

```yaml
northbound:
  endpoint: "0.0.0.0:50051"
  tls:
    insecure: true
```

### `database`

```yaml
database:
  type: in_memory
```

Or for persistent storage (required for API-managed topology across restarts):

```yaml
database:
  type: sqlite
  path: "/var/lib/slim/cp.db"
```

### `reconciler`

Controls the link and route reconciliation loops:

| Field | Default | Description |
|-------|---------|-------------|
| `max_requeues` | `15` | Max retry attempts per item before dropping |
| `base_retry_delay` | `200ms` | Base delay for first retry (exponential backoff, cap 30s) |
| `reconcile_period` | `60s` | Full reconciliation sweep interval (`0s` disables) |
| `enable_orphan_detection` | `false` | Delete data-plane connections not tracked by the control plane |
| `workers` | `4` | Parallel reconciler workers per queue |

## Topology

The `topology` section controls how groups are connected. Two modes are
mutually exclusive:

### Config mode (`links` or `segments` configured)

The config file is the **single source of truth**. Links are defined as an
adjacency list (or per-segment link graphs). Mutation APIs
(`slimctl controller link add`, `segment add`, and related topology changes)
are disabled.

```yaml
topology:
  links:
    - group: cloud
      neighbors: ["*"]  # connects to all registered groups
    - group: edge-a
      neighbors: [cloud]
    - group: edge-b
      neighbors: [cloud]
```

The wildcard `"*"` matches all registered groups and is resolved at node
registration time. All links are bidirectional.

#### Segments (route isolation)

Segments create isolated routing domains within the topology:

```yaml
topology:
  segments:
    - name: default
      links:
        - group: group-a
          neighbors: [group-b]
```

### API mode (topology absent or empty)

The database is the source of truth. Manage topology via slimctl:

```bash
slimctl controller segment add my-segment
slimctl controller link add group-a group-b -s my-segment
slimctl controller link remove group-a group-b
```

Requires a persistent database (`type: sqlite`) to preserve topology across
restarts.

## Node registration

Data-plane nodes register with the control plane when configured with a
southbound client and `group_name`:

```yaml
services:
  slim/0:
    group_name: edge-a
    controller:
      clients:
        - endpoint: "http://control-plane:50052"
          tls:
            insecure: true
          connection_type: remote
```

Routes are **reconciled** from topology — they are not added imperatively via
the CLI. Client subscriptions on a node trigger route expansion within the
reconciled topology.

## JSON schema

Server endpoint configuration follows the same schema as data-plane servers:
[server-config.schema.json](https://github.com/agntcy/slim/blob/main/crates/config/src/schema/server-config.schema.json).

For the full architecture and reconciliation behavior, see the
[control plane README](https://github.com/agntcy/slim/blob/main/crates/control-plane/README.md)
in the repository.

# SLIM Controller Configuration Reference

The SLIM Controller is configured through a YAML file passed at startup with the `--config` flag.

## Minimal Configuration

```yaml
northbound:
  endpoint: "0.0.0.0:50051"

southbound:
  endpoint: "0.0.0.0:50052"

tracing:
  log_level: info
```

## Multiple Listeners

`northbound` and `southbound` each accept either a single server mapping or a
list of them. With a list, the same API is served on every configured address,
each with its own TLS settings — useful for exposing plaintext on loopback for
local tooling while requiring mTLS on a routable address, or for binding one
listener per network interface.

```yaml
northbound:
  - endpoint: "127.0.0.1:50051"    # local tooling, plaintext
    tls:
      insecure: true
  - endpoint: "0.0.0.0:50451"      # routable, TLS
    tls:
      insecure: false
      cert_file: /etc/slim/tls.crt
      key_file: /etc/slim/tls.key

southbound:
  - endpoint: "0.0.0.0:50052"
    tls:
      insecure: true
```

An empty list is rejected at startup; omit the key entirely to use the default
single listener. Reusing the same address across two listeners fails at startup
with the offending endpoint named.

## Full Configuration Reference

```yaml
# Northbound interface — used by slimctl and external management tools
northbound:
  endpoint: "0.0.0.0:50051"   # Address to bind the northbound gRPC server
  tls:
    insecure: true             # Set to false to enable TLS

# Southbound interface — used by SLIM nodes to register and receive config updates
southbound:
  endpoint: "0.0.0.0:50052"   # Address to bind the southbound gRPC server
  tls:
    insecure: true             # Set to false to enable TLS

# Reconciler settings
reconciler:
  max_requeues: 15             # Maximum retries for a failed node reconciliation
  base_retry_delay: "200ms"   # Initial delay; retries use exponential backoff (capped at 30s)
  reconcile_period: "60s"     # How often all nodes are re-enqueued for a full reconciliation sweep
  workers: 4                  # Number of concurrent reconciler worker tasks
  enable_orphan_detection: false  # Delete data-plane connections not tracked by the controller

# Tracing and logging
tracing:
  log_level: info             # Log level (trace, debug, info, warn, error)
  display_thread_names: false
  display_thread_ids: false

# Database backend (default: in-memory; all state is lost on restart)
database:
  type: in_memory

# SQLite-backed persistent store (state survives restarts)
# database:
#   type: sqlite
#   path: /db/controlplane.db

# Topology mode (default: API-managed — topology is built via slimctl at runtime)
topology: {}
```

## Topology Configuration

The `topology` key controls how the Controller creates inter-domain links and routes:

### API-Managed (Default)

No topology declared. The Controller manages all links and routes via the gRPC/CLI API. Use `slimctl controller link add` to create links at runtime.

```yaml
topology: {}
```

### Config-Managed: Links

Define the link graph directly in the configuration file. Changes require a config reload.

```yaml
topology:
  links:
    - domain: hub
      neighbors: ["*"]   # hub connects to all other domains
```

### Config-Managed: Segments

Define multiple independent routing domains. Domains in separate segments cannot route to each other.

```yaml
topology:
  segments:
    - name: customer-1
      links:
        - domain: cloud
          neighbors: [cluster-a]
    - name: customer-2
      links:
        - domain: cloud
          neighbors: [cluster-b]
```

Use `$domain` for dynamic per-tenant segment expansion:

```yaml
topology:
  segments:
    - name: segment-$domain
      links:
        - domain: platform
          neighbors: [$domain]
```

## Registration Authentication

The `topology.registration_auth` field configures how data-plane nodes authenticate when registering with the controller:

### Shared Secret

```yaml
topology:
  registration_auth:
    type: shared_secret
    secrets:
      cluster-a: "secret-for-cluster-a"
      cluster-b: "secret-for-cluster-b"
```

### SPIRE

```yaml
topology:
  registration_auth:
    type: spire
    socket_path: "unix:///run/spire/agent-sockets/api.sock"
```

## SLIM Node Configuration for Self-Registration

To have SLIM nodes automatically register with the Controller on startup, configure the node's `controller.clients` section with the Controller's southbound address:

```yaml
services:
  slim/1:
    dataplane:
      servers: []
      clients: []
    controller:
      servers: []
      clients:
        - endpoint: "http://<controller-address>:50052"
          tls:
            insecure: true
```

## Node Connection Enforcement

The `enforce_node_connection` key lets the Controller push connection parameters to all nodes at registration time. Any field set here overrides the node's local configuration; omitted fields leave the node's own settings untouched.

```yaml
enforce_node_connection:
  # Fixed-interval reconnect backoff (milliseconds).
  backoff: 2000

  # Connect timeout (milliseconds).
  timeout: 5000

  # Keepalive settings pushed to every connecting node.
  keepalive:
    tcp_keepalive: "60s"
    http2_keepalive: "60s"
    timeout: "10s"
    keep_alive_while_idle: true
```

All three fields are optional. A minimal example that only enforces backoff:

```yaml
enforce_node_connection:
  backoff: 3000
```

!!! note
    Values are stamped onto a node's connection record at registration time and
    persist until the node re-registers. Nodes that are already connected pick
    up changes on their next reconnect.

## Related

- [SLIM Controller Overview](./index.md)
- [SLIM Controller Installation](./install.md)
- [Authentication](../../architecture/authentication.md) — Identity management including SPIRE integration
- [Routing](../../architecture/routing.md) — Topology configuration in depth

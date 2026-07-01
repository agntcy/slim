# SLIM Controller Configuration Reference

The SLIM Controller is configured through a YAML file passed at startup with the `-config` flag.

## Minimal Configuration

```yaml
northbound:
  httpHost: localhost
  httpPort: 50051
  logging:
    level: DEBUG

southbound:
  httpHost: localhost
  httpPort: 50052
  reconciler:
    threads: 3

logging:
  level: DEBUG
```

## Full Configuration Reference

```yaml
# Northbound interface — used by slimctl and external management tools
northbound:
  httpHost: 0.0.0.0          # IP address to bind the northbound gRPC server
  httpPort: 50051             # Port for the northbound interface
  logging:
    level: INFO               # Log level for northbound requests (DEBUG, INFO, WARN, ERROR)

# Southbound interface — used by SLIM nodes to register and receive config updates
southbound:
  httpHost: 0.0.0.0          # IP address to bind the southbound gRPC server
  httpPort: 50052             # Port for the southbound interface
  logging:
    level: INFO
  tls:
    useSpiffe: false          # Enable SPIFFE/SPIRE-based mTLS on the southbound interface
  spire:
    socketPath: ""            # SPIRE agent socket path, e.g. "unix:///run/spire/agent-sockets/api.sock"

# Reconciler settings
reconciler:
  maxRequeues: 15             # Maximum retries for a failed node reconciliation
  maxNumOfParallelReconciles: 1000  # Maximum concurrent reconciliations across nodes

# Top-level logging
logging:
  level: INFO                 # Global log level (DEBUG, INFO, WARN, ERROR)

# SQLite database for persisting control plane state
database:
  filePath: controlplane.db  # Path to the SQLite database file

# SPIRE integration settings
spire:
  enabled: false              # Enable SPIRE for issuing SVIDs to controller components
  trustedDomains: []          # Trust domains to federate with
    # - cluster-a.example.org
    # - cluster-b.example.org
```

## mTLS with SPIRE (Southbound)

To secure the southbound interface with mTLS using [SPIRE](https://spiffe.io/docs/latest/spire-about/spire-concepts/):

```yaml
northbound:
  httpHost: 0.0.0.0
  httpPort: 50051

southbound:
  httpHost: 0.0.0.0
  httpPort: 50052
  tls:
    useSpiffe: true
  spire:
    socketPath: "unix:///run/spire/agent-sockets/api.sock"

logging:
  level: DEBUG

reconciler:
  maxRequeues: 15
  maxNumOfParallelReconciles: 1000

database:
  filePath: controlplane.db

spire:
  enabled: false
  trustedDomains: []
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

## Related

- [SLIM Controller Overview](./index.md)
- [SLIM Controller Installation](./install.md)
- [Authentication](../../architecture/authentication.md) — Identity management including SPIRE integration

# slimctl Commands

`slimctl` is organized into command groups, each serving a specific purpose:

## `slim` - Local SLIM Instance Management

Start and manage standalone SLIM instances for development and testing.

```bash
# Start in insecure mode (development)
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Start with configuration file
slimctl slim start --config examples/insecure-server.yaml

# Start with TLS
slimctl slim start --config examples/tls-server.yaml

# Override configuration with CLI flags
slimctl slim start --config examples/env-override.yaml --endpoint 0.0.0.0:8443
```

**Key Features:**
- Uses full production SLIM configuration format (runtime, tracing, services)
- Environment variable override system for flexible configuration
- TLS support with automatic certificate validation
- Production parity - behaves exactly like production SLIM

**Available Commands:**
- `slimctl slim start` - Start a SLIM instance

**📖 Detailed Documentation:**
- [SLIM Instance Management Guide](SLIM_INSTANCE.md) - Complete command reference, configuration format, TLS setup
- [Examples](examples/) - Sample configuration files with detailed explanations

---

## `route` - Route Management

Manage routes on SLIM nodes via the Control Plane.

```bash
# List routes for a node
slimctl route list --node-id=my-node

# Add a route
slimctl route add org/ns/app/instance via config.json --node-id=my-node

# Delete a route
slimctl route del org/ns/app/instance via http://host:port --node-id=my-node
```

---

## `connection` - Connection Management

Manage connections on SLIM nodes via the Control Plane.

```bash
# List connections for a node
slimctl connection list --node-id=my-node
```

---

## `node-connect` - Direct Node Management

Connect directly to a SLIM node's control endpoint (bypasses central Control Plane).

```bash
# List routes directly on a node
slimctl node-connect route list --server=<node_control_endpoint>

# Add route directly to a node
slimctl node-connect route add org/ns/app/instance via config.json --server=<node_control_endpoint>
```

---

## General Commands

### Version

```bash
slimctl version
```

Print version and build information.

### Help

```bash
slimctl --help
slimctl slim start --help
slimctl route --help
```

Get detailed help for any command.

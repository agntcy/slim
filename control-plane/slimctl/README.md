# slimctl

Command-line interface for managing SLIM instances and the SLIM control plane.

## Overview

`slimctl` is a unified CLI tool that provides commands for:

- **Local Development** - Run standalone SLIM instances for development and testing using production configurations
- **Route Management** - Configure message routing between services via the SLIM Control Plane
- **Connection Monitoring** - View and manage active connections on SLIM nodes
- **Direct Node Access** - Manage SLIM nodes directly without going through the Control Plane

### Command Groups

| Command | Purpose |
|---------|---------|
| `slim` | Start and manage local SLIM instances |
| `route` | Manage routes via Control Plane |
| `connection` | Monitor connections via Control Plane |
| `node-connect` | Direct node management (bypass Control Plane) |
| `version` | Display version information |

---

## Installation

### Pre-built Binaries

Download from the [GitHub releases page](https://github.com/agntcy/slim/releases):

1. Download the binary for your OS and architecture
2. Extract the archive
3. Move `slimctl` to a directory in your `PATH`

### Homebrew (macOS)

```bash
brew tap agntcy/slim
brew install slimctl
```

### Building from Source

**Prerequisites**: Go 1.20+, Task (taskfile.dev)

```bash
# From repository root
cd control-plane
task control-plane:slimctl:build

# Binary location: .dist/bin/slimctl
```

---

## Command Usage and Examples

### `slim` - Local SLIM Instances

Run standalone SLIM instances for development and testing using production configurations.

**Start with a configuration file:**

```bash
# Start with base configuration (insecure)
slimctl slim start --config data-plane/config/base/server-config.yaml

# Start with TLS configuration
slimctl slim start --config data-plane/config/tls/server-config.yaml
```

**Override the server endpoint:**

```bash
slimctl slim start --config data-plane/config/base/server-config.yaml --endpoint 127.0.0.1:9090
```

**Control log level:**

```bash
RUST_LOG=debug slimctl slim start --config data-plane/config/base/server-config.yaml
```

**Available flags:**
- `--config` - Path to YAML configuration file (production SLIM format)
- `--endpoint` - Server endpoint (sets `SLIM_ENDPOINT` environment variable)

**Configuration files:** Use production configs from `data-plane/config/`:
- `base/server-config.yaml` - Basic insecure configuration
- `tls/server-config.yaml` - TLS-enabled server
- `mtls/` - Mutual TLS authentication
- `basic-auth/` - HTTP Basic authentication
- `jwt-auth-*/` - JWT authentication (RSA, ECDSA, HMAC)
- `spire/` - SPIFFE/SPIRE workload identity
- `proxy/` - HTTP proxy configuration
- `telemetry/` - OpenTelemetry integration

---

### `route` - Route Management

Manage message routes on SLIM nodes via the Control Plane.

**List routes:**

```bash
slimctl route list --node-id=my-node
```

**Add a route:**

```bash
# Create connection configuration
cat > connection_config.json <<EOF
{
  "endpoint": "http://127.0.0.1:46357"
}
EOF

# Add the route
slimctl route add org/namespace/service/0 via connection_config.json --node-id=my-node
```

**Delete a route:**

```bash
slimctl route del org/namespace/service/0 via http://localhost:46357 --node-id=my-node
```

**Control Plane Configuration:**

slimctl looks for Control Plane configuration at:
- `$HOME/.slimctl/config.yaml`
- `./config.yaml` (current directory)
- Via `--config` flag

Example configuration:

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

---

### `connection` - Connection Management

Monitor active connections on SLIM nodes via the Control Plane.

**List connections:**

```bash
slimctl connection list --node-id=my-node
```

---

### `node-connect` - Direct Node Management

Connect directly to a SLIM node's control endpoint, bypassing the central Control Plane.

**List routes directly on a node:**

```bash
slimctl node-connect route list --server=<node_control_endpoint>
```

**Add route directly to a node:**

```bash
slimctl node-connect route add org/namespace/service/0 via config.json --server=<node_control_endpoint>
```

---

### `version` - Version Information

Display version and build information:

```bash
slimctl version
```

---

### Getting Help

Get detailed help for any command:

```bash
slimctl --help
slimctl slim --help
slimctl slim start --help
slimctl route --help
```

---

## Additional Resources

- **SLIM Project**: [https://github.com/agntcy/slim](https://github.com/agntcy/slim)
- **Issues & Questions**: [GitHub Issues](https://github.com/agntcy/slim/issues)
- **Configuration Examples**: See `data-plane/config/` directory for production configurations
- **SLIM Go Bindings**: [https://github.com/agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go)

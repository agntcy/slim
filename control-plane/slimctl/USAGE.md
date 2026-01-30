# slimctl Usage Guide

This guide provides practical usage examples for all slimctl commands.

## Local SLIM Instance Usage

### Quick Start

```bash
# Start in insecure mode (development)
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Start with configuration file
slimctl slim start --config examples/insecure-server.yaml

# Start with TLS
slimctl slim start --config examples/tls-server.yaml

# Override configuration with CLI flags
slimctl slim start --config examples/env-override.yaml --endpoint 127.0.0.1:9090
```

📖 **Detailed Guide**: [SLIM Instance Management](SLIM_INSTANCE.md)
📖 **Configuration Examples**: [examples/](examples/)

---

## Control Plane Usage

### Route Management

**Add a route:**

```bash
# Create connection config
cat > connection_config.json <<EOF
{
  "endpoint": "http://127.0.0.1:46357"
}
EOF

# Add route
slimctl route add org/default/alice/0 via connection_config.json --node-id=node-1
```

**List routes:**

```bash
slimctl route list --node-id=node-1
```

**Delete a route:**

```bash
slimctl route del org/default/alice/0 via http://localhost:46367 --node-id=node-1
```

### Connection Management

**List connections:**

```bash
slimctl connection list --node-id=my-node
```

### Direct Node Management

**Connect directly to a node (bypass Control Plane):**

```bash
# List routes on a node
slimctl node-connect route list --server=<node_control_endpoint>

# Add route directly
slimctl node-connect route add org/ns/app/instance via config.json --server=<node_control_endpoint>
```

📖 **Command Reference**: [COMMANDS.md](COMMANDS.md)

---

## slimctl Tool Configuration

For `route`, `connection`, and other Control Plane commands, `slimctl` looks for configuration at:
- `$HOME/.slimctl/config.yaml`
- Current working directory `./config.yaml`
- Via `--config` flag

### Example Control Plane Config

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

### Connection Config File Format

When adding routes, you'll need to specify connection configuration:

```json
{
  "endpoint": "http://127.0.0.1:46357"
}
```

> Full schema reference: [client-config-schema.json](https://github.com/agntcy/slim/blob/main/data-plane/core/config/src/grpc/schema/client-config.schema.json)

---

## Configuration Notes

- The `server` endpoint in the slimctl config should point to the SLIM Control service
- For `node-connect` commands, use the `--server` flag to specify the node's control endpoint directly
- TLS configuration is optional but recommended for production deployments
- Connection timeout defaults to 10 seconds if not specified

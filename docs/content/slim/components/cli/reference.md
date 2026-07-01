# SLIM Controller Reference

## `slimctl` Commands

### `slim` - Local SLIM Instances

Run standalone SLIM instances for development and testing using production configurations.

**Start with a configuration file:**

```bash
# Start with base configuration (insecure)
slimctl slim start --config data-plane/config/base/server-config.yaml

# Start with TLS configuration
slimctl slim start --config data-plane/config/tls/server-config.yaml
```

**Quick start just specifying a custom listening endpoint:**

```bash
slimctl slim start --endpoint 127.0.0.1:12345
```

!!! note
    The server will start without TLS when using this command.

**Control log level:**

```bash
RUST_LOG=debug slimctl slim start --config data-plane/config/base/server-config.yaml
```

The log level can also be set directly in the configuration file via the `tracing.log_level` field.
See [data-plane/config/base/server-config.yaml](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/config/base/server-config.yaml)
for an example.

**Available flags:**

- `--config` - Path to YAML configuration file (production SLIM format)
- `--endpoint` - Server endpoint (sets `SLIMCTL_SLIM_ENDPOINT` environment variable)

**Configuration files:** See example configs from [data-plane/config/](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config):

- [base](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/config/base) - Basic insecure configuration
- [tls](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/tls) - TLS-enabled server
- [mtls](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/config/mtls) - Mutual TLS authentication
- [basic-auth](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/config/basic-auth) - HTTP Basic authentication
- `jwt-auth-*` - JWT authentication ([RSA](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/jwt-auth-rsa),
    [ECDSA](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/jwt-auth-ecdsa),
    [HMAC](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/jwt-auth-hmac))
- [spire](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/spire) - SPIFFE/SPIRE workload identity
- [proxy](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/proxy) - HTTP proxy configuration
- [telemetry](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/config/telemetry) - OpenTelemetry integration

### `route` - Route Management

Manage message routes on SLIM nodes via the Control Plane.

**List routes:**

```bash
slimctl controller route list --node-id=my-node
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
slimctl controller route add org/namespace/service via connection_config.json --node-id=my-node
```

**Delete a route:**

```bash
slimctl controller route del org/namespace/service/0 via http://localhost:46357 --node-id=my-node
```

### `connection` - Connection Management

Monitor active connections on SLIM nodes via the Control Plane.

**List connections:**

```bash
slimctl controller connection list --node-id=my-node
```

### `node` - Node Management

Manage and view SLIM nodes via the Control Plane.

**List registered nodes:**

```bash
slimctl controller node list
```

### `node` - Direct Node Management

Connect directly to a SLIM node's control endpoint, bypassing the central Control Plane.

**List routes directly on a node:**

```bash
slimctl node route list --server=<node_control_endpoint>
```

**Add route directly to a node:**

```bash
slimctl node route add org/namespace/service/0 via config.json --server=<node_control_endpoint>
```

### `version` - Version Information

Display version and build information:

```bash
slimctl version
```

### Getting Help

Get detailed help for any command:

```bash
slimctl --help
slimctl slim --help
slimctl slim start --help
slimctl route --help
```

## Usage Examples

### Example 1: Create, Delete Route using node-id

Add route for node `slim/a` to forward messages for `org/default/alice/0` to node `slim/b`.

```bash
# List available nodes
slimctl controller node list
2 node(s) found
Node ID: slim/b status: CONNECTED
  Connection details:
  - Endpoint: 127.0.0.1:46457
    MtlsRequired: false
    ExternalEndpoint: test-slim.default.svc.cluster.local:46457
Node ID: slim/a status: CONNECTED
  Connection details:
  - Endpoint: 127.0.0.1:46357
    MtlsRequired: false
    ExternalEndpoint: test-slim.default.svc.cluster.local:46357

# Add route to node slim/a
slimctl controller route add org/default/alice/0 via slim/b --node-id slim/a

# Delete an existing route
slimctl controller route del org/default/alice/0 via slim/b --node-id slim/a
```

### Example 2: Create, Delete Route Using `connection_config.json`

```bash
# Create connection configuration
cat > connection_config.json <<EOF
{
  "endpoint": "http://127.0.0.1:46357"
}
EOF

# Add a new route
slimctl controller route add org/default/alice/0 via connection_config.json --node-id=my-node

# Delete an existing route
slimctl controller route del org/default/alice/0 via http://localhost:46357 --node-id=my-node
```

For full reference of connection_config.json, see the [client-config-schema.json](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/core/config/src/grpc/schema/client-config.schema.json).

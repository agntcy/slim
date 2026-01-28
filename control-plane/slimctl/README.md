# slimctl

`slimctl` is the command-line interface for managing SLIM instances and the SLIM control plane.

## Overview

`slimctl` provides two main capabilities:

1. **Local SLIM Instance Management** - Start and configure standalone SLIM instances for development and testing
2. **Control Plane Management** - Manage routes and connections on SLIM instances via the SLIM Control service

## Quick Start

### Building from Source

```bash
# From repository root
cd control-plane
task control-plane:slimctl:build

# Binary will be at: .dist/bin/slimctl (from repository root)
```

### Usage

**For managing local SLIM instances:**
```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```
📖 See [SLIM Instance Management Guide](SLIM_INSTANCE.md) for detailed documentation.

**For managing SLIM via Control Plane:**
```bash
slimctl route list --node-id=my-node
slimctl route add org/ns/app/instance via config.json --node-id=my-node
```
📖 See [Control Plane Management](#control-plane-management-commands) section below.

---

## Commands

### Local SLIM Instance Management

The `slim` command allows you to start and manage local SLIM instances for development and testing.

**Quick Example:**

```bash
# Start in development mode
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Start with TLS
slimctl slim start --config config.yaml
```

**📖 For complete documentation, see [SLIM Instance Management Guide](SLIM_INSTANCE.md)**

Topics covered:
- Command reference and flags
- Configuration file format
- TLS certificate setup and requirements
- Testing workflows
- Troubleshooting
- Advanced deployment options

---

## Control Plane Management Commands

These commands interact with the SLIM Control service to manage routes and connections across SLIM nodes.

### Configuration

`slimctl` looks for configuration at:
- `$HOME/.slimctl/config.yaml`
- Current working directory `./config.yaml`
- Via `--config` flag

**Example Control Plane Config:**

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

The `server` endpoint should point to:
- **SLIM Control service** - Central service managing SLIM node configurations
- **SLIM node control endpoint** - Direct connection to a SLIM instance (using `node-connect` subcommand)

### Route Management

#### List Routes

```bash
slimctl route list --node-id=<slim_node_id>
```

#### Add Route

```bash
slimctl route add <org/namespace/app/instance> via <config_file> --node-id=<slim_node_id>
```

**Connection Config File Example (`connection_config.json`):**

```json
{
  "endpoint": "http://127.0.0.1:46357"
}
```

> Full reference: [client-config-schema.json](https://github.com/agntcy/slim/blob/main/data-plane/core/config/src/grpc/schema/client-config.schema.json)

#### Delete Route

```bash
slimctl route del <org/namespace/app/instance> via <host:port> --node-id=<slim_node_id>
```

### Connection Management

#### List Connections

```bash
slimctl connection list --node-id=<slim_node_id>
```

### Direct Node Connection

Connect directly to a SLIM node control endpoint (bypassing central control service):

```bash
# List connections
slimctl node-connect connection list --server=<node_control_endpoint>

# List routes
slimctl node-connect route list --server=<node_control_endpoint>

# Add route
slimctl node-connect route add <org/ns/app/instance> via <config_file> --server=<node_control_endpoint>

# Delete route
slimctl node-connect route del <org/ns/app/instance> via <host:port> --server=<node_control_endpoint>
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

---

## Development and Testing

### Prerequisites

- Go 1.20+
- Task (taskfile.dev)
- OpenSSL (for certificate generation)

### Building

```bash
# From repository root
cd control-plane
task slimctl:build

# Or from slimctl directory
cd control-plane/slimctl
task build
```

### Testing

**For testing local SLIM instances:**

See the [SLIM Instance Testing Guide](SLIM_INSTANCE.md#testing) for complete manual testing workflows, test automation tasks, and validation steps.

```bash
cd control-plane/slimctl
task test:manual  # Guided testing workflow
```

**For general development:**

```bash
task validate     # Validate environment
task --list       # Show all available tasks
```

### Project Structure

```
slimctl/
├── cmd/
│   └── main.go              # Entry point
├── internal/
│   ├── cmd/
│   │   ├── slim/            # Local SLIM instance commands
│   │   ├── route/           # Route management commands
│   │   ├── connection/      # Connection management commands
│   │   └── ...              # Other commands
│   ├── config/
│   │   └── slim.go          # SLIM instance configuration
│   └── manager/
│       └── slim.go          # SLIM instance manager
├── examples/
│   ├── insecure-server.yaml # Example insecure config
│   ├── tls-server.yaml      # Example TLS config
│   └── README.md            # Example configurations guide
├── testdata/                # Generated test data (git-ignored)
│   ├── certs/               # Generated test certificates
│   └── *.yaml               # Generated test configurations
├── Taskfile.yaml            # Task automation
├── README.md                # This file
└── SLIM_INSTANCE.md         # SLIM instance management guide
```

---

## Examples

### Local SLIM Instance Examples

For complete examples of starting and configuring local SLIM instances, see the [SLIM Instance Management Guide](SLIM_INSTANCE.md#usage-examples).

### Managing Routes via Control Plane

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

**List and verify:**

```bash
slimctl route list --node-id=node-1
```

**Delete route:**

```bash
slimctl route del org/default/alice/0 via http://localhost:46367 --node-id=node-1
```

---

## Troubleshooting

### SLIM Instance Issues

For troubleshooting local SLIM instances (certificates, connections, TLS issues), see the [SLIM Instance Troubleshooting Guide](SLIM_INSTANCE.md#troubleshooting).

### Control Plane Connection Issues

If you're having issues connecting to the SLIM Control Plane:

1. Verify the server endpoint is correct
2. Check TLS configuration matches server requirements
3. Ensure network connectivity to the control plane
4. Verify authentication credentials if required

---

## Contributing

### Running Validation Locally

```bash
cd control-plane/slimctl
task validate
```

This runs:
- Binary verification
- Certificate validation
- Environment checks

### Adding New Commands

1. Create command file in `internal/cmd/<command>/`
2. Implement using Cobra command structure
3. Register command in `cmd/main.go`
4. Test manually using `task test:manual`
5. Update documentation

---

## License

See the [LICENSE](../../LICENSE) file in the repository root.

## Support

For issues, questions, or contributions:
- GitHub Issues: https://github.com/agntcy/slim/issues
- Documentation: https://github.com/agntcy/slim

---

## See Also

- [SLIM Instance Management Guide](SLIM_INSTANCE.md) - Detailed guide for managing local SLIM instances
- [Examples Directory](examples/) - Sample configurations and usage examples
- [SLIM Documentation](https://github.com/agntcy/slim) - Complete SLIM project documentation
- [SLIM Control Plane](https://github.com/agntcy/slim/tree/main/control-plane/control-plane) - Central control service
- [SLIM Go Bindings](https://github.com/agntcy/slim-bindings-go) - Go language bindings

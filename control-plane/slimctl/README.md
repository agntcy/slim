# slimctl

`slimctl` is the command-line interface for managing SLIM instances and the SLIM control plane.

## Overview

`slimctl` is organized into command groups, each providing specific capabilities for working with SLIM:

### Command Groups

**`slim`** - Start and configure standalone SLIM instances for development and testing
- Uses full production SLIM configuration format
- TLS support with automatic certificate validation
- Environment variable override system
- Production parity guarantee

**`route`** - Manage message routes on SLIM nodes via the Control Plane
- List, add, and delete routes
- Configure service-to-service connectivity
- Full Control Plane integration

**`connection`** - Monitor and manage active connections on SLIM nodes
- View all active connections
- Monitor connection health and status

**`node-connect`** - Connect directly to SLIM node control endpoints
- Bypass central Control Plane for direct node access
- Direct route and connection management

## Installation

📖 **[Installation Guide](installation.md)** - Pre-built binaries, Homebrew, and building from source

## Documentation

📖 **[Usage Guide](USAGE.md)** - Quick usage examples and tool configuration

📖 **[Commands Reference](COMMANDS.md)** - All command groups and their usage

📖 **[SLIM Instance Guide](SLIM_INSTANCE.md)** - Comprehensive guide for local SLIM instances

📖 **[Configuration Examples](examples/)** - Sample configurations with detailed explanations

---

## Development

### Prerequisites

- Go 1.20+
- Task (taskfile.dev)
- OpenSSL (for certificate generation)

### Testing

```bash
cd control-plane/slimctl
task test:manual  # Guided testing workflow
task validate     # Validate environment
task --list       # Show all available tasks
```

See [SLIM Instance Testing Guide](SLIM_INSTANCE.md#testing) for complete testing workflows.

---

## Support

- **Issues & Questions**: [GitHub Issues](https://github.com/agntcy/slim/issues)
- **Documentation**: [SLIM Project](https://github.com/agntcy/slim)
- **Contributing**: See [SLIM_INSTANCE.md#testing](SLIM_INSTANCE.md#testing) for testing workflows
- **Troubleshooting**: See [SLIM Instance Guide](SLIM_INSTANCE.md#troubleshooting) and [Usage Guide](USAGE.md)

## Related Documentation

- [Installation Guide](installation.md) - Pre-built binaries, Homebrew, and building from source
- [Usage Guide](USAGE.md) - Quick usage examples and tool configuration
- [Commands Reference](COMMANDS.md) - All command groups and their usage
- [SLIM Instance Guide](SLIM_INSTANCE.md) - Comprehensive guide for local SLIM instances
- [Configuration Examples](examples/) - Sample configurations with detailed explanations
- [SLIM Control Plane](https://github.com/agntcy/slim/tree/main/control-plane/control-plane) - Central control service
- [SLIM Go Bindings](https://github.com/agntcy/slim-bindings-go) - Go language bindings

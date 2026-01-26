# slimctl

`slimctl` is the command-line interface for managing SLIM (Secure Local Interconnect Mesh) instances and the SLIM control plane.

## Overview

`slimctl` provides two main capabilities:

1. **Local SLIM Instance Management** - Start and configure standalone SLIM instances for development and testing
2. **Control Plane Management** - Manage routes and connections on SLIM instances via the SLIM Control service

## Quick Start

### Building from Source

```bash
# From repository root
cd control-plane
task slimctl:build

# Binary will be at: .dist/bin/slimctl (from repository root)
```

### Running a Local SLIM Instance

```bash
# Start in insecure mode (development)
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Start with TLS (production-like)
slimctl slim start --config config.yaml
```

### Managing SLIM via Control Plane

```bash
# List routes on a SLIM node
slimctl route list --node-id=my-node

# Add a route
slimctl route add org/ns/app/instance via config.json --node-id=my-node
```

---

## Local SLIM Instance Commands

### `slim start`

Start a local SLIM instance with customizable configuration.

#### Usage

```bash
slimctl slim start [flags]
```

#### Flags

| Flag | Short | Type | Description |
|------|-------|------|-------------|
| `--config` | `-c` | string | Path to YAML configuration file |
| `--endpoint` | | string | Endpoint to bind (e.g., `127.0.0.1:8080`) |
| `--port` | | string | Port to listen on (default: `8080`, used if --endpoint not specified) |
| `--insecure` | | bool | Disable TLS and run in insecure mode |
| `--tls-cert` | | string | Path to TLS certificate file |
| `--tls-key` | | string | Path to TLS private key file |

#### Configuration File Format

Create a YAML configuration file to define SLIM instance settings:

```yaml
# Endpoint to bind
endpoint: "127.0.0.1:8443"

# TLS Configuration
tls:
  insecure: false
  cert_file: "/path/to/server-cert.pem"
  key_file: "/path/to/server-key.pem"
```

**Configuration Precedence:** CLI flags override YAML configuration values.

#### Examples

**1. Start in Insecure Mode (Development)**

```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

**2. Load Configuration from YAML**

```bash
slimctl slim start --config config.yaml
```

**3. Start with TLS via CLI Flags**

```bash
slimctl slim start \
  --endpoint 127.0.0.1:8443 \
  --tls-cert /path/to/cert.pem \
  --tls-key /path/to/key.pem
```

**4. Override YAML Config with CLI Flags**

```bash
# config.yaml specifies TLS, but we override to insecure
slimctl slim start --config config.yaml --insecure
```

#### TLS Certificate Requirements

- Certificates must be **X.509 v3** format (required by rustls)
- Must include Subject Alternative Names (SAN)
- Recommended: Use SHA-256 or stronger signature algorithm

**Generate Test Certificates:**

```bash
cd control-plane/slimctl
task certs:setup
```

Or manually:

```bash
# See examples in examples/test-certs/ or use:
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout server-key.pem -out server-cert.pem \
  -days 365 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

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

**Run Unit Tests:**

```bash
cd control-plane/slimctl
task test
```

**Run Tests with Coverage:**

```bash
task test:coverage
```

**Manual Testing - Complete Workflow:**

```bash
# Setup environment and see testing options
task test:manual

# Then run a specific test
task test:manual:insecure  # Test insecure mode
task test:manual:tls       # Test TLS mode
task test:manual:quick     # Quick test
```

**Available Test Tasks:**

```bash
task --list
```

Key tasks:
- `task test` - Run all unit tests
- `task test:coverage` - Run tests with coverage
- `task test:manual` - Complete guided manual testing workflow
- `task setup:test-env` - Setup test certificates and configs
- `task run:insecure` - Quick run in insecure mode
- `task run:tls` - Quick run with TLS
- `task ci` - Run CI pipeline
- `task clean` - Clean generated files

### Project Structure

```
slimctl/
├── cmd/
│   └── main.go              # Entry point
├── internal/
│   ├── cmd/
│   │   ├── root.go          # Root command
│   │   ├── slim/
│   │   │   └── slim.go      # Local SLIM instance commands
│   │   ├── route/           # Route management commands
│   │   └── connection/      # Connection management commands
│   ├── config/
│   │   ├── slim.go          # SLIM instance configuration
│   │   └── slim_test.go     # Configuration tests
│   └── manager/
│       ├── slim.go          # SLIM instance manager
│       └── slim_test.go     # Manager tests
├── examples/
│   ├── insecure-server.yaml # Example insecure config
│   ├── tls-server.yaml      # Example TLS config
│   └── test-certs/          # Generated test certificates
├── Taskfile.yaml            # Task automation
└── README.md                # This file
```

---

## Examples

### Example 1: Development with Insecure SLIM Instance

```bash
# Terminal 1: Start SLIM server
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Terminal 2: Run client application
cd data-plane/bindings/go/examples/point_to_point
go run . --local=org/alice/app --server=http://127.0.0.1:8080
```

### Example 2: Production-like with TLS

**Create configuration:**

```yaml
# config.yaml
endpoint: "0.0.0.0:8443"
tls:
  insecure: false
  cert_file: "/etc/slim/cert.pem"
  key_file: "/etc/slim/key.pem"
```

**Start SLIM:**

```bash
slimctl slim start --config config.yaml
```

### Example 3: Managing Routes via Control Plane

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

### UnsupportedCertVersion Error

**Error:** `InvalidCertificate(Other(OtherError(UnsupportedCertVersion)))`

**Cause:** Certificate is not X.509 v3

**Solution:** Regenerate certificates:

```bash
task certs:clean
task certs:setup
```

### Certificate File Not Found

**Cause:** Incorrect path or wrong working directory

**Solution:** Use absolute paths or verify working directory:

```bash
# Use absolute paths
slimctl slim start --tls-cert /full/path/to/cert.pem --tls-key /full/path/to/key.pem

# Or use paths relative to config file location
slimctl slim start --config /path/to/config.yaml
```

### Address Already in Use

**Cause:** Port is already bound by another process

**Solution:** Use a different port or stop the existing process:

```bash
# Use different port
slimctl slim start --endpoint 127.0.0.1:9090 --insecure

# Or find and stop existing process
lsof -ti:8080 | xargs kill -9
```

### Connection Refused

**Cause:** SLIM instance not running or wrong endpoint

**Solution:** Verify server is running and check endpoint:

```bash
# Check if port is listening
netstat -an | grep 8080

# Verify endpoint matches between server and client
```

---

## Contributing

### Running CI Pipeline Locally

```bash
cd control-plane/slimctl
task ci
```

This runs:
- Linting
- Unit tests with coverage
- Build validation

### Adding New Commands

1. Create command file in `internal/cmd/<command>/`
2. Implement using Cobra command structure
3. Add tests in `<command>_test.go`
4. Register command in `cmd/root.go`
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

- [SLIM Documentation](https://github.com/agntcy/slim)
- [SLIM Control Plane](https://github.com/agntcy/slim/tree/main/control-plane/control-plane)
- [SLIM Go Bindings](https://github.com/agntcy/slim-bindings-go)

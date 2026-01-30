# Local SLIM Instance Management

This guide covers how to use `slimctl slim start` to run local SLIM instances for development and testing.

## Overview

The `slimctl slim` command runs a standalone SLIM instance using the **production configuration format**. This ensures development and testing environments match production behavior exactly.

**Use cases:**

- **Development** - Test applications with production-like SLIM configuration
- **Integration Testing** - Validate SLIM integration in CI/CD pipelines
- **Local Debugging** - Debug SLIM behavior with full runtime and tracing control
- **Demos** - Quick demonstrations of SLIM functionality

## Quick Start

### Start with a Configuration File

```bash
# Using a production config from the repository
slimctl slim start --config data-plane/config/base/config.yaml
```

### Start with Endpoint Override

```bash
# Override the endpoint from the config
slimctl slim start --config data-plane/config/base/config.yaml --endpoint 127.0.0.1:9090
```

### Start with Environment Variables

```bash
# Set endpoint via environment variable
export SLIM_ENDPOINT=127.0.0.1:8443
slimctl slim start --config data-plane/config/base/config.yaml
```

---

## `slim start` Command

Start a local SLIM instance with production configuration.

### Usage

```bash
slimctl slim start [flags]
```

### Flags

| Flag | Short | Type | Description |
|------|-------|------|-------------|
| `--config` | `-c` | string | Path to YAML configuration file (production SLIM format) |
| `--endpoint` | | string | Server endpoint (sets `SLIM_ENDPOINT` environment variable) |

### Examples

```bash
# Start with base configuration
slimctl slim start --config data-plane/config/base/config.yaml

# Override endpoint
slimctl slim start --config data-plane/config/base/config.yaml --endpoint 0.0.0.0:8443

# Use TLS configuration
slimctl slim start --config data-plane/config/tls/config.yaml

# Set log level
RUST_LOG=debug slimctl slim start --config data-plane/config/base/config.yaml
```

---

## Configuration

### YAML Configuration Format

`slimctl` uses the **full SLIM production configuration format** with three main sections:

```yaml
runtime:
  n_cores: 0              # 0 = use all available cores
  thread_name: "slim"     # Thread name prefix
  drain_timeout: 10s      # Graceful shutdown timeout

tracing:
  log_level: info         # trace, debug, info, warn, error
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:                 # Service instance ID
    dataplane:
      servers:
        - endpoint: "127.0.0.1:8080"
          tls:
            insecure: true
      clients: []
```

### Production Configuration Examples

Instead of maintaining separate examples, `slimctl` uses the production configurations from `data-plane/config/`:

| Configuration | Location | Description |
|--------------|----------|-------------|
| **Base** | `data-plane/config/base/` | Minimal insecure configuration |
| **TLS** | `data-plane/config/tls/` | TLS-enabled server |
| **mTLS** | `data-plane/config/mtls/` | Mutual TLS authentication |
| **Basic Auth** | `data-plane/config/basic-auth/` | HTTP Basic authentication |
| **JWT Auth** | `data-plane/config/jwt-auth-*/` | JWT authentication (RSA, ECDSA, HMAC) |
| **SPIRE** | `data-plane/config/spire/` | SPIFFE/SPIRE workload identity |
| **Proxy** | `data-plane/config/proxy/` | HTTP proxy configuration |
| **Telemetry** | `data-plane/config/telemetry/` | OpenTelemetry integration |

See [examples/README.md](examples/README.md) for complete documentation.

### Environment Variable Substitution

SLIM configuration files support environment variable substitution using `${env:VARNAME}` or `${env:VARNAME:-default}` syntax.

**Example:**

```yaml
services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "${env:SLIM_ENDPOINT:-127.0.0.1:8080}"
          tls:
            insecure: true
```

The `--endpoint` flag sets the `SLIM_ENDPOINT` environment variable, which is then substituted by the SLIM bindings when loading the configuration.

### Supported Environment Variables

| Variable | Purpose | Set By |
|----------|---------|--------|
| `SLIM_ENDPOINT` | Server listen address | `--endpoint` flag or manual export |
| `RUST_LOG` | Log level (trace, debug, info, warn, error) | Manual export |

Any environment variable can be referenced in configuration files using the `${env:VARNAME}` syntax.

---

## Usage Examples

### Example 1: Basic Development Server

```bash
slimctl slim start --config data-plane/config/base/config.yaml
```

Starts SLIM with insecure configuration on the default endpoint.

### Example 2: Custom Endpoint

```bash
slimctl slim start --config data-plane/config/base/config.yaml --endpoint 0.0.0.0:9090
```

Overrides the endpoint to listen on all interfaces at port 9090.

### Example 3: TLS-Enabled Server

```bash
slimctl slim start --config data-plane/config/tls/config.yaml
```

Starts SLIM with TLS enabled using certificates specified in the config.

### Example 4: Debug Logging

```bash
RUST_LOG=debug slimctl slim start --config data-plane/config/base/config.yaml
```

Starts SLIM with debug-level logging for troubleshooting.

### Example 5: Environment Variable Configuration

```bash
# Set endpoint via environment variable
export SLIM_ENDPOINT=127.0.0.1:8443

# Create config that references it
cat > my-config.yaml <<EOF
runtime:
  n_cores: 0
  thread_name: "slim"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "\${env:SLIM_ENDPOINT}"
          tls:
            insecure: true
      clients: []
EOF

# Start with the config
slimctl slim start --config my-config.yaml
```

---

## Testing

### Manual Testing Workflow

The `slimctl` Taskfile provides automated testing tasks:

```bash
cd control-plane/slimctl

# Run manual testing workflow
task test:manual
```

This task guides you through testing the SLIM instance with different configurations.

### Available Test Tasks

```bash
# List all available tasks
task --list

# Validate environment
task validate

# Generate test certificates (if needed for TLS configs)
task certs:setup

# Clean up test artifacts
task clean
```

### Testing with Client Applications

To test SLIM with client applications:

1. **Start SLIM**:
   ```bash
   slimctl slim start --config data-plane/config/base/config.yaml --endpoint 127.0.0.1:8080
   ```

2. **Connect a client** (in another terminal):
   ```bash
   # Use your SLIM client application
   # Point it to 127.0.0.1:8080
   ```

3. **Verify connectivity** in the SLIM logs

---

## Troubleshooting

### Configuration Validation Errors

**Symptom**: SLIM fails to start with a configuration error

**Solution**:
1. Check the error message - SLIM provides detailed validation errors
2. Verify all file paths (certificates, keys) exist and are readable
3. Ensure environment variables are set if referenced in the config
4. Compare your config structure with examples in `data-plane/config/`

### Environment Variable Not Substituted

**Symptom**: `${env:VARNAME}` is not being replaced in the config

**Solution**:
1. Ensure the environment variable is exported: `export VARNAME=value`
2. Check the variable name matches exactly (case-sensitive)
3. Verify the syntax: `${env:VARNAME}` or `${env:VARNAME:-default}`
4. Use the `--endpoint` flag instead of manually setting `SLIM_ENDPOINT`

### Address Already in Use

**Symptom**: Error message about port already in use

**Solution**:
1. Check if another SLIM instance is running: `lsof -i :8080`
2. Kill the existing process or use a different port
3. Use `--endpoint` to specify a different port: `--endpoint 127.0.0.1:9090`

### Connection Refused from Client

**Symptom**: Client cannot connect to SLIM

**Solution**:
1. Verify SLIM is running and listening on the expected endpoint
2. Check firewall rules allow connections to the port
3. Ensure the endpoint in your client matches the SLIM server endpoint
4. For `127.0.0.1`, clients must be on the same machine; use `0.0.0.0` to listen on all interfaces

### TLS Handshake Failures

**Symptom**: TLS connection errors between client and server

**Solution**:
1. Verify certificates are X.509 v3 format (required by rustls)
2. Check certificate paths are correct in the config
3. Ensure certificates are not expired: `openssl x509 -in cert.pem -noout -dates`
4. Verify client and server certificates are compatible
5. Check certificate permissions are readable
6. Use `RUST_LOG=debug` to see detailed TLS error messages

### Certificate Version Errors

**Symptom**: `UnsupportedCertVersion` error

**Solution**:
Generate X.509 v3 certificates (required by rustls):

```bash
# Generate v3 certificate
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout server-key.pem -out server-cert.pem \
  -days 365 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

---

## Advanced Topics

### Running as a Service

To run SLIM as a system service, create a systemd unit file:

```ini
[Unit]
Description=SLIM Service
After=network.target

[Service]
Type=simple
User=slim
WorkingDirectory=/opt/slim
ExecStart=/usr/local/bin/slimctl slim start --config /etc/slim/config.yaml
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

### Docker Deployment

To run SLIM in Docker:

```dockerfile
FROM debian:bookworm-slim
COPY slimctl /usr/local/bin/
COPY config.yaml /etc/slim/
CMD ["slimctl", "slim", "start", "--config", "/etc/slim/config.yaml"]
```

### Monitoring and Observability

Enable telemetry for monitoring:

```bash
# Use telemetry configuration
slimctl slim start --config data-plane/config/telemetry/config.yaml
```

See `data-plane/config/telemetry/` for OpenTelemetry integration examples.

---

## Configuration Best Practices

1. **Start with production configs**: Use configurations from `data-plane/config/` as templates
2. **Use environment variables**: Reference `${env:VARNAME}` for values that change between environments
3. **Test locally first**: Validate configurations with `slimctl` before deploying to production
4. **Enable appropriate logging**: Use `info` for production, `debug` for troubleshooting
5. **Secure TLS properly**: Use valid certificates and proper key management in production
6. **Document custom configs**: Add comments explaining non-standard settings

---

## See Also

- [Configuration Examples](examples/README.md) - Detailed guide to all configuration options
- [slimctl Usage Guide](USAGE.md) - Quick usage examples
- [slimctl Commands](COMMANDS.md) - Complete command reference
- [Production Configurations](../../data-plane/config/) - Production-ready SLIM configurations
- [SLIM Documentation](../../README.md) - Complete SLIM project documentation

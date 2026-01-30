# slimctl Examples

This directory provides guidance on using `slimctl slim start` with SLIM configuration files.

## Overview

The `slimctl slim start` command uses the **production SLIM configuration format** to ensure development and testing environments match production behavior exactly. Instead of maintaining separate example files, we reference the production-ready configurations from the `data-plane/config/` directory.

## Configuration Format

SLIM configurations follow a structured YAML format with three main sections:

```yaml
runtime:
  n_cores: 0
  thread_name: "slim-worker"
  drain_timeout: 10s

tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "127.0.0.1:8080"
          tls:
            insecure: true
      clients: []
```

## Production Configuration Examples

The SLIM repository contains production-ready configuration examples in the `data-plane/config/` directory. These configurations demonstrate various deployment scenarios and features.

### Available Configurations

| Configuration | Location | Description |
|--------------|----------|-------------|
| **Base** | `data-plane/config/base/` | Minimal insecure configuration for development |
| **TLS** | `data-plane/config/tls/` | TLS-enabled server configuration |
| **mTLS** | `data-plane/config/mtls/` | Mutual TLS authentication |
| **Basic Auth** | `data-plane/config/basic-auth/` | HTTP Basic authentication |
| **JWT Auth (RSA)** | `data-plane/config/jwt-auth-rsa/` | JWT authentication with RSA keys |
| **JWT Auth (ECDSA)** | `data-plane/config/jwt-auth-ecdsa/` | JWT authentication with ECDSA keys |
| **JWT Auth (HMAC)** | `data-plane/config/jwt-auth-hmac/` | JWT authentication with HMAC |
| **SPIRE Integration** | `data-plane/config/spire/` | SPIFFE/SPIRE workload identity |
| **Proxy** | `data-plane/config/proxy/` | HTTP proxy configuration |
| **Logging** | `data-plane/config/logging/` | Advanced logging configuration |
| **Telemetry** | `data-plane/config/telemetry/` | OpenTelemetry integration |
| **Backoff Strategies** | `data-plane/config/exponential-backoff/`, `data-plane/config/fixed-interval-backoff/` | Retry and backoff configurations |

## Usage Examples

### Example 1: Start with Base Configuration

```bash
# From repository root
slimctl slim start --config data-plane/config/base/config.yaml
```

This starts SLIM with a minimal insecure configuration, suitable for local development.

### Example 2: Start with TLS

```bash
# From repository root
slimctl slim start --config data-plane/config/tls/config.yaml
```

This starts SLIM with TLS enabled. Ensure the certificate paths in the config are correct for your environment.

### Example 3: Override Endpoint

```bash
# Override the endpoint from the config file
slimctl slim start --config data-plane/config/base/config.yaml --endpoint 127.0.0.1:9090
```

The `--endpoint` flag sets the `SLIM_ENDPOINT` environment variable, which can be referenced in config files using `${env:SLIM_ENDPOINT}`.

### Example 4: Start with Environment Variables

```bash
# Set endpoint via environment variable
export SLIM_ENDPOINT=127.0.0.1:8443
slimctl slim start --config data-plane/config/base/config.yaml
```

Any environment variable can be referenced in SLIM config files using the `${env:VARNAME}` syntax.

### Example 5: Control Log Level

```bash
# Set log level via RUST_LOG environment variable
export RUST_LOG=debug
slimctl slim start --config data-plane/config/base/config.yaml
```

The `RUST_LOG` environment variable controls the logging level for SLIM.

## Environment Variable Substitution

SLIM configuration files support environment variable substitution using the `${env:VARNAME}` or `${env:VARNAME:-default}` syntax.

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

This configuration will use the `SLIM_ENDPOINT` environment variable if set, otherwise defaults to `127.0.0.1:8080`.

## Creating Custom Configurations

To create your own configuration:

1. **Start with a base**: Copy a configuration from `data-plane/config/` that matches your use case
2. **Customize**: Modify endpoints, TLS settings, authentication, etc.
3. **Use environment variables**: Reference `${env:VARNAME}` for values that change between environments
4. **Test**: Run with `slimctl slim start --config your-config.yaml`

## Configuration Sections

### Runtime Configuration

Controls SLIM's runtime behavior:

```yaml
runtime:
  n_cores: 0              # Number of worker threads (0 = all available cores)
  thread_name: "slim"     # Thread name prefix
  drain_timeout: 10s      # Graceful shutdown timeout
```

### Tracing Configuration

Controls logging and observability:

```yaml
tracing:
  log_level: info                 # Log level: trace, debug, info, warn, error
  display_thread_names: true      # Show thread names in logs
  display_thread_ids: true        # Show thread IDs in logs
```

### Service Configuration

Defines SLIM dataplane servers and clients:

```yaml
services:
  slim/0:                          # Service instance ID
    dataplane:
      servers:                     # Inbound server configurations
        - endpoint: "0.0.0.0:8080"
          tls:
            insecure: true         # Or configure TLS with cert/key
      clients: []                  # Outbound client configurations
```

## TLS Configuration

For TLS-enabled servers, configure the `tls` section:

```yaml
tls:
  insecure: false
  source:
    type: file
    cert: "/path/to/cert.pem"
    key: "/path/to/key.pem"
```

See `data-plane/config/tls/` and `data-plane/config/mtls/` for complete examples.

## Authentication

SLIM supports multiple authentication mechanisms:

- **Basic Auth**: HTTP Basic authentication (see `data-plane/config/basic-auth/`)
- **JWT**: JSON Web Token authentication with RSA, ECDSA, or HMAC (see `data-plane/config/jwt-auth-*/`)
- **mTLS**: Mutual TLS with client certificates (see `data-plane/config/mtls/`)
- **SPIRE**: SPIFFE workload identity (see `data-plane/config/spire/`)

## Testing Configurations

To test a configuration without starting a full server:

```bash
# The bindings will validate the config on startup
slimctl slim start --config your-config.yaml

# Watch for validation errors in the output
# SLIM will exit immediately if the config is invalid
```

## Troubleshooting

### Configuration Validation Errors

If SLIM fails to start with a configuration error:

1. Check the error message - SLIM provides detailed validation errors
2. Verify all file paths (certificates, keys) exist and are readable
3. Ensure environment variables are set if referenced in the config
4. Compare your config structure with examples in `data-plane/config/`

### Environment Variable Not Substituted

If `${env:VARNAME}` is not being replaced:

1. Ensure the environment variable is exported: `export VARNAME=value`
2. Check the variable name matches exactly (case-sensitive)
3. Verify the syntax: `${env:VARNAME}` or `${env:VARNAME:-default}`

### TLS Certificate Errors

For TLS-related errors:

1. Verify certificate and key files exist at the specified paths
2. Ensure certificates are in PEM format
3. Check that certificates are X.509 v3 (required by rustls)
4. Verify file permissions allow reading

## Additional Resources

- **Production Configurations**: `data-plane/config/` directory in the repository
- **SLIM Instance Guide**: [SLIM_INSTANCE.md](../SLIM_INSTANCE.md) - Comprehensive documentation
- **Main Documentation**: [README.md](../README.md) - slimctl overview
- **SLIM Documentation**: [Repository root](../../../README.md) - Complete SLIM project documentation

## Quick Reference

```bash
# Start with a config file
slimctl slim start --config path/to/config.yaml

# Override endpoint
slimctl slim start --config config.yaml --endpoint 127.0.0.1:9090

# Set log level
RUST_LOG=debug slimctl slim start --config config.yaml

# Use environment variables in config
export SLIM_ENDPOINT=127.0.0.1:8443
slimctl slim start --config config.yaml
```

For more detailed usage information, see [SLIM_INSTANCE.md](../SLIM_INSTANCE.md).

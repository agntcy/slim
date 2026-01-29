# slimctl Configuration Examples

This directory contains example configuration files for running local SLIM instances via `slimctl slim start`.

## Overview

The `slimctl slim start` command starts a local SLIM data plane server for development and testing. It uses the full SLIM production configuration format and delegates all configuration processing to the SLIM bindings library.

## Quick Start

```bash
# Start in insecure mode (development only)
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Start with a configuration file
slimctl slim start --config examples/insecure-server.yaml

# Start with TLS (after generating certificates)
cd control-plane/slimctl
task certs:setup
slimctl slim start --config examples/tls-server.yaml
```

## Configuration Files

### 📄 insecure-server.yaml
**Minimal insecure configuration for local development**

Runs SLIM without TLS on `127.0.0.1:8080`. Suitable for local testing only.

```bash
slimctl slim start --config examples/insecure-server.yaml
```

### 📄 tls-server.yaml
**TLS-enabled server configuration**

Runs SLIM with TLS on `0.0.0.0:8443`. Requires X.509 v3 certificates compatible with rustls.

```bash
# Generate test certificates first
task certs:setup

# Update certificate paths in the YAML, then:
slimctl slim start --config examples/tls-server.yaml
```

### 📄 production.yaml
**Production-ready configuration example**

Shows a typical production setup with:
- TLS enabled
- Optimized runtime settings (4 cores)
- Enhanced keepalive configuration
- Production-grade endpoints

### 📄 debug.yaml
**Debug configuration with verbose logging**

Enables detailed debug logging for troubleshooting. Runs on port 9090.

```bash
slimctl slim start --config examples/debug.yaml
```

### 📄 env-vars.yaml
**Configuration using environment variables**

Demonstrates using `${env:VARIABLE}` syntax for flexible deployments:

```bash
# Development
export LOG_LEVEL=debug
slimctl slim start --config examples/env-vars.yaml

# Production
export SLIM_ENDPOINT=0.0.0.0:443
export SLIM_TLS_INSECURE=false
export SLIM_TLS_CERT=/etc/slim/certs/server-cert.pem
export SLIM_TLS_KEY=/etc/slim/certs/server-key.pem
slimctl slim start --config examples/env-vars.yaml
```

## Configuration Structure

All configuration files follow the full SLIM production format:

```yaml
runtime:
  n_cores: 0              # CPU cores (0 = all available)
  thread_name: "slim-worker"
  drain_timeout: 10s      # Graceful shutdown timeout

tracing:
  log_level: info         # debug, info, warn, error
  display_thread_names: true
  display_thread_ids: true

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "127.0.0.1:8080"
          tls:
            insecure: true  # or configure TLS below
            source:
              type: file
              cert: "path/to/cert.pem"
              key: "path/to/key.pem"
      clients: []
```

## Command-Line Flags

Override configuration values using CLI flags:

| Flag | Description | Environment Variable |
|------|-------------|---------------------|
| `--config`, `-c` | Path to YAML config file | - |
| `--endpoint` | Override server endpoint | `SLIM_OVERRIDE_ENDPOINT` |
| `--insecure` | Disable TLS | `SLIM_OVERRIDE_INSECURE` |
| `--tls-cert` | Override TLS certificate | `SLIM_OVERRIDE_TLS_CERT` |
| `--tls-key` | Override TLS key | `SLIM_OVERRIDE_TLS_KEY` |

The log level can be controlled via the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug slimctl slim start --config examples/insecure-server.yaml
```

## Usage Examples

### 1. Quick Start (No Config File)

Start a server using CLI flags only:

```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

This creates a temporary configuration with your specified settings.

### 2. Development with Config File

```bash
slimctl slim start --config examples/insecure-server.yaml
```

### 3. Override Endpoint

```bash
slimctl slim start --config examples/insecure-server.yaml --endpoint 127.0.0.1:9090
```

### 4. TLS from Configuration

```bash
# Generate test certificates
cd control-plane/slimctl
task certs:setup

# Start with TLS
slimctl slim start --config examples/tls-server.yaml
```

### 5. TLS with CLI Overrides

```bash
slimctl slim start --config examples/insecure-server.yaml \
  --tls-cert testdata/certs/server-cert.pem \
  --tls-key testdata/certs/server-key.pem
```

### 6. Debug Mode

```bash
RUST_LOG=debug slimctl slim start --config examples/debug.yaml
```

### 7. Using Environment Variables

```bash
export SLIM_ENDPOINT=127.0.0.1:9000
export LOG_LEVEL=debug
slimctl slim start --config examples/env-vars.yaml
```

## Generating TLS Certificates

For testing and development, use the built-in certificate generation:

```bash
cd control-plane/slimctl
task certs:setup
```

This generates X.509 v3 certificates compatible with rustls:
- Certificate: `testdata/certs/server-cert.pem`
- Private Key: `testdata/certs/server-key.pem`

### Manual Certificate Generation

If you prefer to generate certificates manually:

```bash
# Create OpenSSL config for v3 extensions
cat > openssl.cnf <<'EOF'
[req]
default_bits = 2048
prompt = no
default_md = sha256
distinguished_name = dn
x509_extensions = v3_req

[dn]
C = US
ST = Test
L = Test
O = Test Organization
CN = localhost

[v3_req]
subjectAltName = @alt_names
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth

[alt_names]
DNS.1 = localhost
IP.1 = 127.0.0.1
EOF

# Generate private key
openssl genrsa -out server-key.pem 2048

# Generate X.509 v3 certificate
openssl req -new -x509 -key server-key.pem \
  -out server-cert.pem -days 365 \
  -config openssl.cnf -extensions v3_req
```

**Important:** The certificate must be X.509 v3 (not v1) for rustls compatibility.

## Configuration Behavior

### Delegation to Bindings

All configuration validation and processing is delegated to the SLIM bindings library:

✅ **slimctl does NOT:**
- Validate YAML syntax
- Check file existence
- Validate configuration structure
- Parse configuration values

✅ **SLIM bindings handle:**
- YAML parsing and validation
- Configuration structure validation
- Environment variable substitution (`${env:VAR}`)
- File loading and watching
- Error reporting

This ensures **production parity** - the development command behaves exactly like production SLIM.

### Environment Variable Substitution

The bindings support `${env:VARIABLE}` syntax in any configuration value:

```yaml
services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "${env:SLIM_ENDPOINT:-127.0.0.1:8080}"
          tls:
            insecure: "${env:SLIM_TLS_INSECURE:-true}"
```

CLI flags are converted to environment variables automatically:
- `--endpoint 127.0.0.1:8080` → `SLIM_OVERRIDE_ENDPOINT=127.0.0.1:8080`
- `--insecure` → `SLIM_OVERRIDE_INSECURE=true`
- `--tls-cert path/to/cert.pem` → `SLIM_OVERRIDE_TLS_CERT=path/to/cert.pem`
- `--tls-key path/to/key.pem` → `SLIM_OVERRIDE_TLS_KEY=path/to/key.pem`

## Taskfile Automation

The `slimctl` directory includes a Taskfile with helpful commands:

```bash
# Generate certificates
task certs:setup

# Generate test configurations
task configs:setup

# Run in insecure mode
task run:insecure

# Run with TLS
task run:tls

# Clean up test artifacts
task clean
```

View all available tasks:

```bash
task --list
```

## Troubleshooting

### "Failed to create config loader: IoError (NotFound)"

The configuration file doesn't exist. Check the path:

```bash
ls -l examples/insecure-server.yaml
```

### "Failed to create config loader: YamlError"

The YAML syntax is invalid. Errors show line and column numbers:

```
YamlError(Error { kind: SCANNER, problem: "...", problem_mark: Mark { line: 10, column: 5 } })
```

### "UnsupportedCertVersion"

Your certificate is X.509 v1, but rustls requires v3. Regenerate with:

```bash
task certs:setup
```

### Port Already in Use

Another process is using the port. Either stop that process or use a different port:

```bash
slimctl slim start --config examples/insecure-server.yaml --endpoint 127.0.0.1:9090
```

### TLS Handshake Failures

Ensure:
1. Certificate is X.509 v3
2. Certificate and key match
3. Certificate includes proper SAN (Subject Alternative Name) entries
4. Certificate is not expired

Verify certificate details:

```bash
openssl x509 -in server-cert.pem -noout -text
```

## Additional Resources

- [SLIM Documentation](https://github.com/agntcy/slim)
- [SLIM Instance Documentation](../SLIM_INSTANCE.md) - Detailed command reference
- [slimctl README](../README.md) - Main documentation
- [Go Bindings](https://github.com/agntcy/slim-bindings-go)

## Notes

- **Development Only**: The `slim start` command is intended for developers and testing
- **Production Parity**: Uses the same configuration format and processing as production SLIM
- **Security**: Never use insecure mode or self-signed certificates in production
- **Performance**: The local server has the same performance characteristics as production SLIM

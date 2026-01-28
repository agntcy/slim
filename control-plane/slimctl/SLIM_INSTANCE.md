# Local SLIM Instance Management

This guide covers how to use `slimctl` to start and manage local SLIM instances for development and testing.

## Overview

The `slimctl slim` command allows you to run a standalone SLIM instance on your local machine. This is useful for:

- **Development** - Test applications without deploying infrastructure
- **Integration Testing** - Validate SLIM integration in CI/CD pipelines
- **Local Debugging** - Debug SLIM behavior with full control
- **Demos** - Quick demonstrations of SLIM functionality

## Quick Start

### Start in Insecure Mode (Development)

```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

### Start with TLS (Production-like)

```bash
slimctl slim start --config config.yaml
```

---

## `slim start` Command

Start a local SLIM instance with customizable configuration.

### Usage

```bash
slimctl slim start [flags]
```

### Flags

| Flag | Short | Type | Description |
|------|-------|------|-------------|
| `--config` | `-c` | string | Path to YAML configuration file |
| `--endpoint` | | string | Endpoint to bind (e.g., `127.0.0.1:8080`) |
| `--port` | | string | Port to listen on (default: `8080`, used if --endpoint not specified) |
| `--insecure` | | bool | Disable TLS and run in insecure mode |
| `--tls-cert` | | string | Path to TLS certificate file |
| `--tls-key` | | string | Path to TLS private key file |

---

## Configuration

### YAML Configuration File

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

### Configuration Precedence

CLI flags override YAML configuration values:

1. **CLI flags** (highest priority)
2. **YAML configuration file**
3. **Default values** (lowest priority)

### Configuration Examples

See the [`examples/`](examples/) directory for sample configurations:

- [`insecure-server.yaml`](examples/insecure-server.yaml) - Development mode without TLS
- [`tls-server.yaml`](examples/tls-server.yaml) - Production-like with TLS enabled

---

## Usage Examples

### Example 1: Start in Insecure Mode (Development)

```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

**Output:**
```
🌐 Starting server on 127.0.0.1:8080...
   (Insecure mode - TLS disabled)
   Waiting for clients to connect...

✅ Server running and listening

📡 Endpoint: 127.0.0.1:8080

Press Ctrl+C to stop
```

### Example 2: Load Configuration from YAML

```bash
slimctl slim start --config config.yaml
```

Uses all settings from the YAML file.

### Example 3: Start with TLS via CLI Flags

```bash
slimctl slim start \
  --endpoint 127.0.0.1:8443 \
  --tls-cert /path/to/cert.pem \
  --tls-key /path/to/key.pem
```

### Example 4: Override YAML Config with CLI Flags

```bash
# config.yaml specifies TLS, but we override to insecure
slimctl slim start --config config.yaml --insecure
```

CLI flags always take precedence over configuration file values.

### Example 5: Development Workflow

```bash
# Terminal 1: Start SLIM instance
slimctl slim start --endpoint 127.0.0.1:8080 --insecure

# Terminal 2: Run your application
cd data-plane/bindings/go/examples/point_to_point
go run . --local=org/alice/app --server=http://127.0.0.1:8080
```

---

## TLS Configuration

### Certificate Requirements

- **Format:** X.509 v3 (required by rustls)
- **Extensions:** Must include Subject Alternative Names (SAN)
- **Algorithm:** SHA-256 or stronger signature algorithm recommended
- **Key Size:** RSA 2048-bit minimum, or ECDSA P-256+

### Generating Test Certificates

**Using Task (Recommended):**

```bash
cd control-plane/slimctl
task certs:setup
```

This generates:
- Private key: `testdata/certs/server-key.pem`
- Certificate: `testdata/certs/server-cert.pem`
- X.509 v3 compliant with proper extensions

**Manual Generation:**

```bash
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout server-key.pem -out server-cert.pem \
  -days 365 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

**Verify Certificate Version:**

```bash
openssl x509 -in server-cert.pem -noout -text | grep Version
# Should show: Version: 3 (0x2)
```

### Production Certificates

For production use:
- Obtain certificates from a trusted Certificate Authority (CA)
- Use proper domain names in CN and SAN
- Implement certificate rotation
- Store private keys securely

---

## Testing

### Manual Testing Workflow

Use the built-in task automation for comprehensive testing:

```bash
cd control-plane/slimctl

# Complete guided workflow
task test:manual

# Or run specific tests
task test:manual:insecure  # Test insecure mode
task test:manual:tls       # Test TLS mode
```

### Available Test Tasks

| Task | Description |
|------|-------------|
| `task test:manual` | Complete guided testing workflow |
| `task test:manual:insecure` | Test insecure mode |
| `task test:manual:tls` | Test TLS mode with certificates |
| `task test:manual:quick` | Quick insecure mode test |
| `task setup:test-env` | Setup test certificates and configs |
| `task certs:setup` | Generate test TLS certificates |
| `task validate` | Validate test environment |

### Testing with Client Applications

**Terminal 1 (Server):**

```bash
slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

**Terminal 2 (Client):**

```bash
cd data-plane/bindings/go/examples/point_to_point
go run . --local=org/alice/app --server=http://127.0.0.1:8080
```

---

## Troubleshooting

### UnsupportedCertVersion Error

**Error:**
```
InvalidCertificate(Other(OtherError(UnsupportedCertVersion)))
```

**Cause:** Certificate is not X.509 v3

**Solution:**

```bash
cd control-plane/slimctl
task certs:clean
task certs:setup
```

### Certificate File Not Found

**Cause:** Incorrect path or wrong working directory

**Solution:** Use absolute paths or verify paths relative to working directory:

```bash
# Use absolute paths
slimctl slim start \
  --tls-cert /full/path/to/cert.pem \
  --tls-key /full/path/to/key.pem

# Or use config file with absolute paths
slimctl slim start --config /path/to/config.yaml
```

### Address Already in Use

**Cause:** Port is already bound by another process

**Solution:**

```bash
# Use different port
slimctl slim start --endpoint 127.0.0.1:9090 --insecure

# Or find and stop existing process
lsof -ti:8080 | xargs kill -9
```

### Connection Refused from Client

**Cause:** SLIM instance not running or wrong endpoint

**Solution:**

1. Verify server is running:
   ```bash
   netstat -an | grep 8080
   ```

2. Check endpoint matches between server and client

3. Ensure firewall allows connections

### TLS Handshake Failures

**Cause:** Certificate/key mismatch or invalid certificate

**Solution:**

1. Verify certificate and key match:
   ```bash
   openssl x509 -noout -modulus -in cert.pem | openssl md5
   openssl rsa -noout -modulus -in key.pem | openssl md5
   # MD5 hashes should match
   ```

2. Check certificate validity:
   ```bash
   openssl x509 -in cert.pem -noout -dates
   ```

3. Verify SAN includes the endpoint hostname/IP

---

## Advanced Topics

### Running as a Service

For production deployments, run SLIM as a system service:

**systemd example:**

```ini
[Unit]
Description=SLIM Instance
After=network.target

[Service]
Type=simple
User=slim
ExecStart=/usr/local/bin/slimctl slim start --config /etc/slim/config.yaml
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

### Docker Deployment

```dockerfile
FROM alpine:latest
COPY slimctl /usr/local/bin/
COPY config.yaml /etc/slim/
EXPOSE 8443
CMD ["slimctl", "slim", "start", "--config", "/etc/slim/config.yaml"]
```

### Monitoring and Observability

The SLIM instance provides logging through the configured logger. Monitor:

- Connection events
- Route additions/deletions
- TLS handshake status
- Error conditions

---

## Performance Tuning

### Resource Limits

SLIM instance resource usage depends on:
- Number of concurrent connections
- Message throughput
- Routing table size

Monitor system resources and adjust as needed.

### Network Configuration

- Use localhost (`127.0.0.1`) for local-only access
- Bind to `0.0.0.0` for network-accessible instances (with proper security)
- Configure firewall rules appropriately

---

## See Also

- [Main README](README.md) - Overview of slimctl
- [Examples Directory](examples/) - Sample configurations
- [Taskfile](Taskfile.yaml) - Available automation tasks
- [SLIM Documentation](https://github.com/agntcy/slim) - Complete SLIM docs

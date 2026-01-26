# slimctl Configuration Examples

This directory contains example configuration files for running SLIM instances via `slimctl`.

## Configuration Files

### insecure-server.yaml
Basic configuration for running SLIM in insecure mode (no TLS). This is suitable for local development and testing only.

**Usage:**
```bash
slimctl slim start --config examples/insecure-server.yaml
```

### tls-server.yaml
Configuration for running SLIM with TLS enabled. Requires valid certificate and key files.

**Usage:**
```bash
# Update the cert_file and key_file paths in the YAML first, then:
slimctl slim start --config examples/tls-server.yaml
```

## Configuration Options

### Endpoint
The network address and port to bind the SLIM server to.

```yaml
endpoint: "127.0.0.1:8080"
```

### TLS Configuration

#### Insecure Mode (No TLS)
```yaml
tls:
  insecure: true
```

#### TLS Enabled
```yaml
tls:
  insecure: false
  cert_file: "/path/to/server-cert.pem"
  key_file: "/path/to/server-key.pem"
```

## Command-Line Flags

You can override configuration file values using command-line flags:

### Basic Flags
- `--config`, `-c`: Path to YAML configuration file
- `--endpoint`: Override the endpoint address
- `--insecure`: Disable TLS (insecure mode)
- `--tls-cert`: Override TLS certificate file path
- `--tls-key`: Override TLS key file path

## Usage Examples

**Note:** All examples assume `slimctl` is built and in your PATH, or you're running from `control-plane/slimctl` directory using `./slimctl`.

To build slimctl:
```bash
# From repository root
cd control-plane
task slimctl:build
cd slimctl
```

### 1. Insecure Mode (Default)
Start a SLIM server without TLS on localhost:

```bash
./slimctl slim start --endpoint 127.0.0.1:8080 --insecure
```

### 2. TLS from Configuration File
Start a SLIM server using a YAML configuration file:

```bash
./slimctl slim start --config examples/tls-server.yaml
```

### 3. TLS with Command-Line Overrides
Start with a base configuration but override certificate paths:

```bash
./slimctl slim start --config examples/base-config.yaml \
  --tls-cert ./certs/server.crt \
  --tls-key ./certs/server.key
```

### 4. TLS without Configuration File
Start a TLS-enabled server using only command-line flags:

```bash
./slimctl slim start \
  --endpoint 0.0.0.0:8443 \
  --tls-cert ./certs/server.crt \
  --tls-key ./certs/server.key
```

## Generating Test Certificates

For testing purposes, you can generate self-signed certificates using OpenSSL:

```bash
# Generate a private key
openssl genrsa -out server-key.pem 2048

# Generate a self-signed certificate
openssl req -new -x509 -key server-key.pem -out server-cert.pem -days 365 \
  -subj "/C=US/ST=State/L=City/O=Organization/CN=localhost"
```

**Note:** Self-signed certificates should only be used for testing and development. For production use, obtain certificates from a trusted Certificate Authority (CA).

## Configuration Validation

When starting a SLIM server, the configuration is validated:

1. **Endpoint**: Must not be empty
2. **TLS Enabled**: When `insecure: false`, both `cert_file` and `key_file` must be provided
3. **File Existence**: Certificate and key files must exist and be readable

If validation fails, slimctl will display an error message explaining the issue.

## Future Configuration Options

The configuration system is designed to be extensible. Future versions may support:

- **Authentication**: Basic Auth, JWT authentication
- **HTTP/2 Settings**: Frame size, concurrent streams, header list size
- **Keepalive Parameters**: Connection idle times, ping frequency
- **Client CA**: For mutual TLS (mTLS) authentication
- **SPIRE Integration**: For SPIFFE-based identity and authentication

## Troubleshooting

### "certificate file not found" Error
Ensure the paths in `cert_file` and `key_file` are absolute or relative to the current working directory, and that the files exist.

### "tls.cert_file is required when TLS is enabled" Error
When `insecure: false`, you must provide both certificate and key file paths.

### Connection Refused
Ensure the endpoint address is correct and not already in use by another process.

## Additional Resources

- [SLIM Documentation](https://github.com/agntcy/slim)
- [Go Bindings Examples](../../data-plane/bindings/go/examples/)
- [TLS Configuration Guide](../../data-plane/config/tls/)

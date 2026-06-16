# SLIM Data Plane Configuration

This document provides comprehensive documentation for configuring the SLIM data plane. The configuration is written in YAML format and defines how the data plane runtime, services, authentication, and observability components operate.

This documentation corresponds to the JSON schemas in the SLIM repository:
  
- [Client Configuration Schema](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/core/config/src/grpc/schema/client-config.schema.json)
- [Server Configuration Schema](https://github.com/agntcy/slim/blob/slim-v1.1.0/data-plane/core/config/src/grpc/schema/server-config.schema.json)

## Configuration Structure Overview

The SLIM configuration file consists of three main sections:

=== "Tracing"
    Observability and logging configuration
    ```yaml
    tracing:
      log_level: info
      opentelemetry:
        enabled: true
    ```

=== "Runtime"
    Runtime behavior configuration
    ```yaml
    runtime:
      n_cores: 0
      drain_timeout: "10s"
    ```

=== "Services"
    Services configuration
    ```yaml
    services:
      slim/0:
        dataplane:
          servers: [...]
          clients: [...]
    ```

## Top-Level Configuration Sections

### Tracing Configuration

The `tracing` section configures logging, observability, and OpenTelemetry integration.

#### Basic Tracing Options

```yaml
tracing:
  # Logging level - controls verbosity of log output
  # Available options: trace, debug, info, warn, error
  # Default: info
  log_level: debug

  # Whether to display thread names in log output
  # Default: true
  display_thread_names: true

  # Whether to display thread IDs in log output
  # Default: false
  display_thread_ids: true

  # Additional log filtering (optional)
  # Can be used to filter logs by module or target
  # Default: "info"
  filter: "slim=debug"
```

#### OpenTelemetry Configuration

```yaml
tracing:
  opentelemetry:
    # Enable OpenTelemetry integration for distributed tracing
    # Default: false
    enabled: true

    # Service name for telemetry identification
    # Default: "slim-data-plane"
    service_name: "slim-data-plane"

    # Service version for telemetry
    # Default: "v0.1.0"
    service_version: "v0.2.0"

    # Environment identifier (e.g., prod, staging, dev)
    # Default: "development"
    environment: "production"

    # Metrics collection interval in seconds
    # Default: 30
    metrics_interval_secs: 60

    # gRPC configuration for OpenTelemetry exporter
    grpc:
      endpoint: "http://otel-collector:4317"
      tls:
        insecure: true
```

### Runtime Configuration

The `runtime` section configures the async runtime behavior and resource allocation.

```yaml
runtime:
  # Number of worker threads for the async runtime
  # 0 = use all available CPU cores
  # Default: 0
  n_cores: 4

  # Thread name prefix for runtime worker threads
  # Default: "slim"
  thread_name: "slim-data-plane"

  # Timeout for graceful shutdown - how long to wait for tasks to complete
  # Format: "<number>s", "<number>ms", "<number>m", "<number>h"
  # Default: "10s"
  drain_timeout: "30s"
```

### Services Configuration

The `services` section defines SLIM service instances and their network configurations. Each service is identified by a unique ID in the format `slim/<instance_number>`.

## Service Configuration

### Basic Service Structure

```yaml
services:
  # Service identifier - format: slim/<instance_number>
  slim/0:
    # Optional node ID override
    # If not specified, uses the service identifier
    node_id: "my-node-01"

    # Data plane API configuration
    dataplane:
      servers: [...] # Server endpoints this instance will listen on
      clients: [...] # Client connections this instance will make

    # Control plane API configuration (optional)
    controller:
      servers: [...] # Control plane server endpoints
      clients: [...] # Control plane client connections
```

## TLS Configuration

TLS configuration is used throughout SLIM for securing connections. The same TLS configuration structure applies to:

- Servers (`dataplane.servers[].tls`, `controller.servers[].tls`)
- Clients (`dataplane.clients[].tls`, `controller.clients[].tls`)
- Proxies (`dataplane.clients[].proxy.tls`)

All TLS options documented in this section can be used in any context that accepts a `tls` configuration block. The behavior adapts based on the context (server vs client).

### TLS Modes

=== "Insecure Mode (Development)"
    ```yaml
    tls:
      insecure: true # Disable TLS (not recommended for production)
    ```

=== "Secure Mode (Production)"
    ```yaml
    tls:
      insecure: false # Default: false - requires certificates
      source:
        type: file
        cert: "./certs/cert.pem"
        key: "./certs/key.pem"
    ```

!!! warning "TLS Configuration Required"
    When `insecure: false` (the default), you must provide certificates via `source`. The service does not start without them.

### Certificate Sources (`source`)

The `source` field provides the certificate and private key for the TLS endpoint.

Certificate source usage varies by component type:

- Servers provide the server's identity certificate
- Clients provide the client certificate for mutual TLS (mTLS)

=== "File-based Certificates"
    ```yaml
    tls:
      source:
        type: file
        cert: "./certs/cert.pem"
        key: "./certs/key.pem"
    ```

=== "Inline PEM Certificates"
    ```yaml
    tls:
      source:
        type: pem
        cert: |
          -----BEGIN CERTIFICATE-----
          ...
          -----END CERTIFICATE-----
        key: |
          -----BEGIN PRIVATE KEY-----
          ...
          -----END PRIVATE KEY-----
    ```

=== "SPIRE Integration"
    ```yaml
    tls:
      source:
        type: spire
        socket_path: "/run/spire/sockets/agent.sock"
        jwt_audiences: ["slim", "dataplane"]
        target_spiffe_id: "spiffe://example.org/service"
        trust_domains: ["example.org"]
    ```

=== "No Certificate"
    ```yaml
    tls:
      source:
        type: none  # No certificate configured
    ```

### CA Certificate Sources

CA sources are used for certificate verification. The field name differs based on context:

- Servers use `client_ca` to verify client certificates (for mTLS)
- Clients use `ca_source` to verify server certificates
- Proxies use `ca_source` to verify proxy server certificates

#### Server Client CA (`client_ca`)

Used by servers to verify client certificates when mTLS is required.

=== "File-based CA"
    ```yaml
    tls:
      client_ca:
        type: file
        path: "./certs/ca-cert.pem"
    ```

=== "Inline PEM CA"
    ```yaml
    tls:
      client_ca:
        type: pem
        data: |
          -----BEGIN CERTIFICATE-----
          ...
          -----END CERTIFICATE-----
    ```

=== "SPIRE Bundle"
    ```yaml
    tls:
      client_ca:
        type: spire
        socket_path: "/run/spire/sockets/agent.sock"
        trust_domains: ["example.org"]
    ```

=== "No Client Verification"
    ```yaml
    tls:
      client_ca:
        type: none  # No client certificate verification
    ```

#### Client CA Source (`ca_source`)

Used by clients to verify server certificates.

=== "File-based CA"
    ```yaml
    tls:
      ca_source:
        type: file
        path: "./certs/ca-cert.pem"
    ```

=== "Inline PEM CA"
    ```yaml
    tls:
      ca_source:
        type: pem
        data: |
          -----BEGIN CERTIFICATE-----
          ...
          -----END CERTIFICATE-----
    ```

=== "SPIRE Bundle"
    ```yaml
    tls:
      ca_source:
        type: spire
        socket_path: "/run/spire/sockets/agent.sock"
        trust_domains: ["example.org"]
    ```

=== "No CA"
    ```yaml
    tls:
      ca_source:
        type: none  # No CA configured
    ```

### TLS Options

```yaml
tls:
  # TLS version constraint
  # Options: "tls1.2", "tls1.3"
  # Default: "tls1.3"
  tls_version: "tls1.3"
  
  # Include system CA certificates (CLIENT ONLY)
  # Default: true
  # Only used by clients when verifying servers
  include_system_ca_certs_pool: true
  
  # Skip server name verification (CLIENT ONLY - INSECURE)
  # Default: false
  insecure_skip_verify: false
  
  # Reload client CA file when modified (SERVER ONLY - NOT YET IMPLEMENTED)
  # Default: false
  # reload_client_ca_file: false
```

The following options are context-specific:

- `include_system_ca_certs_pool` - Only used by clients, ignored by servers
- `insecure_skip_verify` - Only used by clients, ignored by servers
- `client_ca` - Only used by servers, not available for clients
- `ca_source` - Used by clients and proxies, exists in server schema but unused

### TLS Examples by Context

=== "Server with mTLS"
    ```yaml
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: false
            # Server's certificate
            source:
              type: file
              cert: "./certs/server-cert.pem"
              key: "./certs/server-key.pem"
            # Verify client certificates
            client_ca:
              type: file
              path: "./certs/ca-cert.pem"
            tls_version: "tls1.3"
    ```

=== "Client with mTLS"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          tls:
            insecure: false
            # Client's certificate for mTLS
            source:
              type: file
              cert: "./certs/client-cert.pem"
              key: "./certs/client-key.pem"
            # Verify server certificate
            ca_source:
              type: file
              path: "./certs/ca-cert.pem"
            include_system_ca_certs_pool: true
            tls_version: "tls1.3"
    ```

=== "HTTPS Proxy"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          proxy:
            url: "https://proxy.example.com:8443"
            tls:
              insecure: false
              # Verify proxy server
              ca_source:
                type: file
                path: "./certs/proxy-ca.crt"
    ```

## Authentication Configuration

Authentication configuration is used throughout SLIM for securing API access. The same authentication structure applies to:

- Servers (`dataplane.servers[].auth`, `controller.servers[].auth`)
- Clients (`dataplane.clients[].auth`, `controller.clients[].auth`)

All authentication options documented in this section can be used in any context that accepts an `auth` configuration block. The behavior adapts based on the context (server vs client).

### Authentication Types

=== "Basic Authentication"
    Username and password authentication.
  
    **Server (Verification):**
    ```yaml
    auth:
      type: basic
      username: "admin"
      password: "secret123"
      # Or use environment variables:
      # password: "${env:ADMIN_PASSWORD}"
    ```
    
    **Client (Credentials):**
    ```yaml
    auth:
      type: basic
      username: "client-user"
      password: "${env:CLIENT_PASSWORD}"
    ```

=== "JWT - Dynamic Generation"
    Generate JWT tokens on-the-fly with signing.
  
    **Server (Verification):**
    ```yaml
    auth:
      type: jwt
      claims:
        audience: ["slim-dataplane"]
        issuer: "slim-auth-service"
        subject: "dataplane-access"
        custom_claims:
          role: "dataplane-user"
          permissions: "read,write"
      duration: "1h"
      key:
        type: decoding  # Verify tokens
        algorithm: "ES256"
        format: pem
        key:
          file: "./keys/jwt-public.pem"
    ```
    
    **Client (Signing):**
    ```yaml
    auth:
      type: jwt
      claims:
        audience: ["remote-slim"]
        issuer: "local-slim"
        subject: "client-connection"
        custom_claims:
          client_id: "slim-instance-01"
      duration: "1h"
      key:
        type: encoding  # Sign tokens
        algorithm: "ES256"
        format: pem
        key:
          file: "./keys/jwt-private.pem"
    ```

=== "JWT - Shared Secret (HMAC)"
    Use shared secrets for JWT signing and verification.
  
    **Server (Verification):**
    ```yaml
    auth:
      type: jwt
      claims:
        audience: ["slim-dataplane"]
        issuer: "slim-client"
      key:
        type: decoding
        algorithm: "HS256"
        format: pem
        key:
          data: "my-secure-shared-secret"
          # Or: file: "/run/secrets/jwt-shared-secret"
    ```
    
    **Client (Signing):**
    ```yaml
    auth:
      type: jwt
      claims:
        audience: ["slim-dataplane"]
        issuer: "slim-client"
      duration: "1h"
      key:
        type: encoding
        algorithm: "HS256"  # Must match server
        format: pem
        key:
          data: "my-secure-shared-secret"  # Must match server
          # Or: file: "/run/secrets/jwt-shared-secret"
    ```

=== "Static JWT Token (Client Only)"
    Use pre-generated JWT tokens from files.
  
    ```yaml
    auth:
      type: static_jwt
      file: "/run/secrets/jwt-token"
      duration: "1h"  # Cache validity before re-reading file
    ```
    
    !!! note
        Static JWT authentication is only available for clients.

=== "JWT - Autoresolve"
    Automatically determine encoding/decoding based on context.
  
    ```yaml
    auth:
      type: jwt
      claims:
        audience: ["remote-slim"]
      duration: "1h"
      key:
        type: autoresolve  # Auto-detect encoding vs decoding
    ```

=== "No Authentication"
    ```yaml
    auth:
      type: none
    ```

### JWT Key Configuration

JWT keys support multiple formats and algorithms.

#### Key Types

The following key types are supported:

- `encoding` for signing JWTs (typically client-side)
- `decoding` for verifying JWTs (typically server-side)
- `autoresolve` automatically determine based on context

#### Key Formats

=== "PEM Format"
    ```yaml
    key:
      type: encoding
      algorithm: "RS256"
      format: pem
      key:
        file: "./keys/private.pem"
        # OR inline:
        # data: |
        #   -----BEGIN PRIVATE KEY-----
        #   ...
        #   -----END PRIVATE KEY-----
    ```

=== "JWK Format"
    ```yaml
    key:
      type: decoding
      algorithm: "RS256"
      format: jwk
      key:
        file: "./keys/public.jwk"
        # OR inline:
        # data: '{"kty":"RSA","n":"...","e":"AQAB"}'
    ```

=== "JWKS Format"
    ```yaml
    key:
      type: decoding
      algorithm: "RS256"
      format: jwks
      key:
        file: "./keys/jwks.json"
        # OR inline:
        # data: '{"keys":[{"kty":"RSA","n":"...","e":"AQAB"}]}'
    ```

!!! tip "Shared Secret Security"
    - Both client and server must use the **same** shared secret
    - Both must use the **same** HMAC algorithm (HS256, HS384, or HS512)
    - Consider using environment variable substitution: `data: "${env:JWT_SECRET}"`
    - For production, store secrets in secure secret management systems

#### SPIRE Authentication

SPIRE does not have a separate `auth` type. Instead, SPIRE provides authentication through the following mechanisms:

- TLS mutual authentication using `tls.source: { type: spire }` for certificate-based authentication
- JWT SVIDs using `auth: { type: jwt }` or `auth: { type: static_jwt }` with SPIRE-issued JWT tokens

See the [Native SPIRE Integration](#native-spire-integration) section for complete examples.

## Server Configuration

Servers define endpoints that the SLIM instance listens on for incoming connections.

### Server Endpoint Configuration

#### Network Address

```yaml
dataplane:
  servers:
    - # REQUIRED: Listen address
      endpoint: "0.0.0.0:46357"
      
      # TLS configuration (see TLS Configuration section)
      tls:
        insecure: false
        source:
          type: file
          cert: "./certs/server-cert.pem"
          key: "./certs/server-key.pem"
      
      # Authentication (see Authentication Configuration section)
      auth:
        type: none
```

#### Unix Socket

```yaml
dataplane:
  servers:
    - # REQUIRED: Unix socket path
      endpoint: "unix:///var/run/slim/dataplane.sock"
      
      # TLS must be set to insecure for unix sockets
      tls:
        insecure: true
```

### Server Connection Settings

```yaml
servers:
  - endpoint: "0.0.0.0:46357"
    
    # HTTP/2 configuration
    # Default: true
    http2_only: true
    
    # Maximum size (in MiB) of messages
    # Default: 4
    max_frame_size: 4
    
    # Connection limits
    # Default: 100
    max_concurrent_streams: 100
    # Default: null (unlimited)
    max_header_list_size: 16384 # 16 KiB
    
    # Buffer sizes for gRPC server
    # Default: 1048576 (1 MiB) for both
    read_buffer_size: 1048576 # 1 MiB
    write_buffer_size: 1048576 # 1 MiB
    
    # Connection keepalive settings
    keepalive:
      max_connection_idle: "3600s" # Close idle connections after 1 hour
      max_connection_age: "7200s" # Maximum connection lifetime
      max_connection_age_grace: "300s" # Grace period before force close
      time: "120s" # Keepalive ping interval
      timeout: "20s" # Keepalive ping timeout
    
    # Arbitrary user-provided metadata (optional)
    metadata:
      role: "ingress"
      replicas: 3
      environment: "production"
      tags:
        - "dataplane"
        - "grpc"
```

## Client Configuration

Clients define outbound connections that the SLIM instance establishes to other services.

### Client Endpoint Configuration

#### Network Address

```yaml
dataplane:
  clients:
    - # REQUIRED: Target endpoint
      endpoint: "http://remote-slim:46357"
      
      # TLS configuration (see TLS Configuration section)
      tls:
        insecure: false
        ca_source:
          type: file
          path: "./certs/ca-cert.pem"
      
      # Authentication (see Authentication Configuration section)
      auth:
        type: none
```

#### Unix Socket

```yaml
dataplane:
  clients:
    - # REQUIRED: Unix socket path
      endpoint: "unix:///var/run/slim/remote-node.sock"
      
      # TLS must be set to insecure for unix sockets
      tls:
        insecure: true
```

### Client Connection Settings

```yaml
clients:
  - endpoint: "http://remote-slim:46357"
    
    # Optional TLS SNI server name override
    # Default: null (uses host from endpoint/origin)
    server_name: "service.example.com"
    
    # Optional origin for client requests
    origin: "https://my-client.example.com"
    
    # Connection timeouts (0s means no timeout)
    # Default: 0s for both
    connect_timeout: "10s"
    request_timeout: "30s"
    
    # Buffer configuration
    buffer_size: 8192
    
    # Custom headers
    headers:
      x-client-id: "slim-instance-01"
      x-environment: "production"
    
    # Rate limiting
    # Format: "<requests>/<duration_in_seconds>"
    rate_limit: "100/60" # 100 requests per minute
```

### HTTP Proxy Configuration

Proxy configuration supports both HTTP and HTTPS proxies with optional authentication.

=== "HTTP Proxy"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          proxy:
            url: "http://proxy.example.com:8080"
            username: "proxy-user"
            password: "${env:PROXY_PASSWORD}"
            headers:
              x-proxy-client: "slim-dataplane"
    ```

=== "HTTPS Proxy"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          proxy:
            url: "https://secure-proxy.example.com:8443"
            username: "${env:PROXY_USER}"
            password: "${env:PROXY_PASS}"
            # TLS configuration for proxy connection
            # (see TLS Configuration section)
            tls:
              insecure: false
              ca_source:
                type: file
                path: "./certs/proxy-ca.crt"
            headers:
              x-department: "engineering"
    ```

### Connection Keepalive

```yaml
clients:
  - endpoint: "remote-slim:46357"
    keepalive:
      tcp_keepalive: "60s"
      http2_keepalive: "60s"
      timeout: "10s"
      keep_alive_while_idle: false
```

### Backoff Configuration

=== "Exponential Backoff"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          backoff:
            type: exponential
            base: 100 # Base delay in milliseconds (default: 100)
            factor: 2 # Multiply delay by this factor each retry (default: 1)
            jitter: true # Add random variation (default: true)
            max_delay: "10s" # Maximum delay between retries (default: "1s")
            max_attempts: 10 # Maximum number of retry attempts (default: unlimited)
    ```

=== "Fixed Interval Backoff"
    ```yaml
    dataplane:
      clients:
        - endpoint: "remote-slim:46357"
          backoff:
            type: fixed_interval
            interval: "2s" # Wait 2 seconds between each retry (default: "1s")
            max_attempts: 5 # Maximum number of retry attempts (default: unlimited)
    ```

!!! info "Default Backoff"
    Default: exponential with base=100ms, factor=1, jitter=true, max_delay=1s, unlimited attempts

## Native SPIRE Integration

SLIM has native support for SPIFFE/SPIRE Workload API for automatic certificate management and zero-trust authentication.

### Server with SPIRE

```yaml
dataplane:
  servers:
    - endpoint: "0.0.0.0:46357"
      tls:
        # Get server certificate from SPIRE
        source:
          type: spire
          # Optional socket path (defaults to SPIFFE_ENDPOINT_SOCKET env var)
          socket_path: "/run/spire/sockets/agent.sock"
          # JWT SVID audiences
          jwt_audiences: ["slim", "dataplane"]
          # Optional target SPIFFE ID for JWT SVIDs
          target_spiffe_id: "spiffe://example.org/dataplane"
          # Optional trust domains override
          trust_domains: ["example.org", "partner.org"]
        
        # Verify client certificates using SPIRE bundle
        client_ca:
          type: spire
          socket_path: "/run/spire/sockets/agent.sock"
          trust_domains: ["example.org"]
      
      auth:
        type: jwt
        claims:
          audience: ["slim-cluster"]
        key:
          type: decoding
          algorithm: "RS256"
          format: jwks
          key:
            # JWT bundles from SPIRE
            file: "/run/spire/jwt-bundle.json"
```

### Client with SPIRE

```yaml
dataplane:
  clients:
    - endpoint: "remote-service:46357"
      tls:
        # Use SPIRE for client certificate
        source:
          type: spire
          socket_path: "/run/spire/sockets/agent.sock"
          jwt_audiences: ["slim"]
          target_spiffe_id: "spiffe://example.org/remote-service"
          trust_domains: ["example.org"]
        
        # Verify server using SPIRE bundle
        ca_source:
          type: spire
          socket_path: "/run/spire/sockets/agent.sock"
          trust_domains: ["example.org"]
      
      # Optional: Add SPIFFE ID to headers
      headers:
        x-spiffe-id: "${env:SPIFFE_ID}"
```

When configuring SPIRE, keep the following in mind:

- Automatic rotation: SPIRE automatically rotates certificates.
- Socket path: If not specified, uses `SPIFFE_ENDPOINT_SOCKET` environment variable.
- Trust domains: When not specified, SLIM derives from the current SVID.
- JWT audiences: Used when requesting JWT SVIDs from SPIRE.
- Zero-trust: SPIRE provides cryptographic workload identity without long-lived secrets.

## Reference: JWT Algorithms

### Symmetric Algorithms (HMAC - Shared Secret)

These algorithms use a shared secret for both signing and verification. Recommended use case is when both client and server can securely share a secret. The same secret is used for signing (client) and verification (server).

| Algorithm | Description | Key Size |
|-----------|-------------|----------|
| `HS256` | HMAC using SHA-256 ⭐ | Any |
| `HS384` | HMAC using SHA-384 | Any |
| `HS512` | HMAC using SHA-512 | Any |

### Asymmetric Algorithms (RSA - Public/Private Key)

The recommended use case is when client and server have different secrets. Client signs with private key, server verifies with public key.

| Algorithm | Description | Key Size |
|-----------|-------------|----------|
| `RS256` | RSA signature with SHA-256 ⭐ | 2048+ bits |
| `RS384` | RSA signature with SHA-384 | 2048+ bits |
| `RS512` | RSA signature with SHA-512 | 2048+ bits |

### Asymmetric Algorithms (RSA-PSS)

| Algorithm | Description | Key Size |
|-----------|-------------|----------|
| `PS256` | RSA-PSS signature with SHA-256 | 2048+ bits |
| `PS384` | RSA-PSS signature with SHA-384 | 2048+ bits |
| `PS512` | RSA-PSS signature with SHA-512 | 2048+ bits |

### Asymmetric Algorithms (ECDSA - Public/Private Key)

The recommended use case is when client and server have different secrets. Client signs with private key, server verifies with public key.

| Algorithm | Description | Curve |
|-----------|-------------|-------|
| `ES256` | ECDSA using P-256 and SHA-256 ⭐ | P-256 |
| `ES384` | ECDSA using P-384 and SHA-384 | P-384 |

### EdDSA

| Algorithm | Description |
|-----------|-------------|
| `EdDSA` | EdDSA signature algorithms |

## Configuration Value Substitution

SLIM supports dynamic configuration value substitution from environment variables and files.

### Environment Variable Substitution

Configuration values can reference environment variables using the `${env:VARIABLE_NAME}` syntax:

```yaml
tracing:
  log_level: "${env:LOG_LEVEL}"

runtime:
  n_cores: "${env:WORKER_THREADS}"

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:${env:LISTEN_PORT}"
          auth:
            type: basic
            username: "${env:AUTH_USERNAME}"
            password: "${env:AUTH_PASSWORD}"
```

### File Content Substitution

Configuration values can reference file contents using the `${file:PATH}` syntax:

```yaml
services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            source:
              type: file
              cert: "/etc/slim/certs/server.crt"
              key: "/etc/slim/certs/server.key"
          auth:
            type: basic
            # Load password from a secure file
            password: "${file:/run/secrets/admin_password}"
```

### Substitution Examples

=== "Kubernetes Secrets"
    ```yaml
    # Perfect for Kubernetes deployments with mounted secrets
    services:
      slim/0:
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                source:
                  type: file
                  cert: "/var/run/secrets/kubernetes.io/tls/tls.crt"
                  key: "/var/run/secrets/kubernetes.io/tls/tls.key"
              auth:
                type: jwt
                key:
                  type: decoding
                  algorithm: "RS256"
                  format: pem
                  key:
                    file: "/var/run/secrets/jwt/public.key"
    ```

=== "Docker Secrets"
    ```yaml
    # For Docker Swarm or Compose with secrets
    services:
      slim/0:
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              auth:
                type: basic
                username: "${env:AUTH_USERNAME}"
                # Docker secret mounted as file
                password: "${file:/run/secrets/db_password}"
    ```

=== "Environment Variables"
    ```yaml
    tracing:
      log_level: "${env:LOG_LEVEL}"
      opentelemetry:
        service_name: "${env:SERVICE_NAME}"
        environment: "${env:ENVIRONMENT}"
  
    services:
      slim/0:
        node_id: "${env:POD_NAME}"
        dataplane:
          servers:
            - endpoint: "0.0.0.0:${env:DATAPLANE_PORT}"
    ```

The following rules apply to substitution:

- Exact Replacement: The entire value must be a substitution expression.
    - Valid: `password: "${env:PASSWORD}"`
    - Invalid: `password: "prefix-${env:PASSWORD}-suffix"`
- Error Handling: If substitution fails, configuration loading will fail.
- File Content: Reads entire file content as string, including newlines.
- Security: File paths are relative to working directory or absolute.

## Duration Format

SLIM uses two different duration formats depending on the field.

### DurationString Format (Most Fields)

Most duration fields use a human-readable string format:

```yaml
# Supported units: y, w, d, h, m, s, ms
timeout: "30s"
max_age: "1h30m"
interval: "500ms"
connect_timeout: "1m30s"
```

**Examples:**

- `"10s"` - 10 seconds
- `"5m"` - 5 minutes  
- `"1h30m"` - 1 hour 30 minutes
- `"2d"` - 2 days
- `"100ms"` - 100 milliseconds

## Complete Configuration Examples

### Development Configuration

```yaml
# config/development.yaml
tracing:
  log_level: debug
  display_thread_names: true
  display_thread_ids: true

runtime:
  n_cores: 0
  thread_name: "slim-dev"
  drain_timeout: "5s"

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true
      clients: []
```

### Production Configuration with mTLS

```yaml
# config/production.yaml
tracing:
  log_level: info
  display_thread_names: false
  display_thread_ids: false
  opentelemetry:
    enabled: true
    service_name: "slim-dataplane"
    service_version: "v1.0.0"
    environment: "production"

runtime:
  n_cores: 0
  thread_name: "slim-prod"
  drain_timeout: "30s"

services:
  slim/0:
    node_id: "${env:NODE_ID}"
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: false
            source:
              type: file
              cert: "/etc/slim/certs/server.crt"
              key: "/etc/slim/certs/server.key"
            client_ca:
              type: file
              path: "/etc/slim/certs/ca.crt"
            tls_version: "tls1.3"
          keepalive:
            max_connection_idle: "1800s"
            time: "300s"
            timeout: "60s"

      clients:
        - endpoint: "peer1.example.com:46357"
          tls:
            insecure: false
            ca_source:
              type: file
              path: "/etc/slim/certs/ca.crt"
            source:
              type: file
              cert: "/etc/slim/certs/client.crt"
              key: "/etc/slim/certs/client.key"
          connect_timeout: "15s"
          request_timeout: "120s"
          backoff:
            type: exponential
            base: 100
            factor: 2
            jitter: true
            max_delay: "10s"
            max_attempts: 5

    controller:
      servers:
        - endpoint: "0.0.0.0:46358"
          tls:
            insecure: false
            source:
              type: file
              cert: "/etc/slim/certs/server.crt"
              key: "/etc/slim/certs/server.key"
```

### JWT Authentication Configuration

```yaml
# config/jwt-auth.yaml
tracing:
  log_level: info

runtime:
  n_cores: 4
  thread_name: "slim-jwt"
  drain_timeout: "15s"

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: false
            source:
              type: file
              cert: "./certs/server.crt"
              key: "./certs/server.key"
          auth:
            type: jwt
            claims:
              audience: ["slim-cluster"]
              issuer: "slim-auth"
              subject: "dataplane-access"
            key:
              type: decoding
              algorithm: "ES256"
              format: pem
              key:
                file: "./keys/jwt-public.pem"

      clients:
        - endpoint: "remote.example.com:46357"
          tls:
            ca_source:
              type: file
              path: "./certs/ca.crt"
          auth:
            type: jwt
            claims:
              audience: ["remote-slim"]
              issuer: "local-slim"
            duration: "2h"
            key:
              type: encoding
              algorithm: "ES256"
              format: pem
              key:
                file: "./keys/jwt-private.pem"
```

### Kubernetes Deployment Configuration

```yaml
# config/kubernetes.yaml
tracing:
  log_level: "${env:LOG_LEVEL}"
  opentelemetry:
    enabled: true
    service_name: "${env:SERVICE_NAME}"
    environment: "${env:ENVIRONMENT}"
    grpc:
      endpoint: "${env:OTEL_COLLECTOR_ENDPOINT}"

runtime:
  n_cores: "${env:WORKER_THREADS}"
  thread_name: "${env:SERVICE_NAME}"
  drain_timeout: "${env:SHUTDOWN_TIMEOUT}"

services:
  slim/0:
    node_id: "${env:POD_NAME}"
    dataplane:
      servers:
        - endpoint: "0.0.0.0:${env:DATAPLANE_PORT}"
          tls:
            insecure: false
            source:
              type: file
              cert: "/var/run/secrets/kubernetes.io/tls/tls.crt"
              key: "/var/run/secrets/kubernetes.io/tls/tls.key"
            client_ca:
              type: file
              path: "/var/run/secrets/kubernetes.io/ca/ca.crt"
          auth:
            type: jwt
            claims:
              audience: ["${env:JWT_AUDIENCE}"]
              issuer: "${env:JWT_ISSUER}"
            key:
              type: decoding
              algorithm: "RS256"
              format: pem
              key:
                file: "/var/run/secrets/jwt/public.key"

      clients:
        - endpoint: "${env:PEER_ENDPOINT}"
          proxy:
            url: "${env:HTTP_PROXY}"
            username: "${env:PROXY_USER}"
            password: "${env:PROXY_PASSWORD}"
          tls:
            ca_source:
              type: file
              path: "/var/run/secrets/kubernetes.io/ca/ca.crt"
            source:
              type: file
              cert: "/var/run/secrets/kubernetes.io/tls/tls.crt"
              key: "/var/run/secrets/kubernetes.io/tls/tls.key"
          headers:
            x-service-account: "${file:/var/run/secrets/kubernetes.io/serviceaccount/token}"
            x-cluster-id: "${env:CLUSTER_ID}"
          backoff:
            type: exponential
            base: 100
            factor: 2
            jitter: true
            max_delay: "30s"
            max_attempts: 10

    controller:
      servers:
        - endpoint: "0.0.0.0:${env:CONTROLLER_PORT}"
          tls:
            source:
              type: file
              cert: "/var/run/secrets/kubernetes.io/tls/tls.crt"
              key: "/var/run/secrets/kubernetes.io/tls/tls.key"
          auth:
            type: basic
            username: "${env:CONTROLLER_USER}"
            password: "${file:/var/run/secrets/controller/password}"
```

### Native SPIRE Zero Trust Configuration

```yaml
# config/spire-native.yaml
tracing:
  log_level: "${env:LOG_LEVEL}"
  opentelemetry:
    enabled: true
    service_name: "slim-spire-${env:SPIFFE_ID}"
    environment: "${env:ENVIRONMENT}"

runtime:
  n_cores: "${env:WORKER_THREADS}"
  thread_name: "slim-spire"
  drain_timeout: "30s"

services:
  slim/0:
    node_id: "${env:SPIFFE_ID}"
    
    dataplane:
      servers:
        - endpoint: "0.0.0.0:${env:DATAPLANE_PORT}"
          tls:
            insecure: false
            
            # Automatically rotated certificates from SPIRE
            source:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              jwt_audiences: ["slim", "dataplane"]
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
            
            # Client verification using SPIRE bundle
            client_ca:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
            
            tls_version: "tls1.3"
          
          auth:
            type: jwt
            claims:
              audience: ["spiffe://${env:SPIFFE_TRUST_DOMAIN}/slim-cluster"]
              issuer: "spiffe://${env:SPIFFE_TRUST_DOMAIN}/slim-issuer"
              subject: "${env:SPIFFE_ID}"
              custom_claims:
                spiffe_id: "${env:SPIFFE_ID}"
                trust_domain: "${env:SPIFFE_TRUST_DOMAIN}"
            key:
              type: decoding
              algorithm: "RS256"
              format: jwks
              key:
                file: "/run/spire/jwt-bundle.json"
          
          keepalive:
            max_connection_idle: "600s"
            time: "60s"
            timeout: "10s"

      clients:
        - endpoint: "${env:PEER_ENDPOINT}"
          tls:
            insecure: false
            
            source:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              target_spiffe_id: "${env:PEER_SPIFFE_ID}"
              jwt_audiences: ["slim"]
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
            
            ca_source:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
          
          headers:
            x-spiffe-id: "${env:SPIFFE_ID}"
            x-trust-domain: "${env:SPIFFE_TRUST_DOMAIN}"
          
          connect_timeout: "10s"
          request_timeout: "30s"
          
          backoff:
            type: exponential
            base: 100
            factor: 2
            jitter: true
            max_delay: "5s"
            max_attempts: 5
          
          auth:
            type: jwt
            claims:
              audience: ["${env:PEER_SPIFFE_ID}"]
              issuer: "${env:SPIFFE_ID}"
              subject: "${env:SPIFFE_ID}"
            duration: "5m"
            key:
              type: encoding
              algorithm: "RS256"
              format: pem
              key:
                file: "/run/spire/jwt-signing-key.pem"

    controller:
      servers:
        - endpoint: "0.0.0.0:${env:CONTROLLER_PORT}"
          tls:
            insecure: false
            source:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
            client_ca:
              type: spire
              socket_path: "/run/spire/sockets/agent.sock"
              trust_domains: ["${env:SPIFFE_TRUST_DOMAIN}"]
          
          auth:
            type: jwt
            claims:
              audience: ["spiffe://${env:SPIFFE_TRUST_DOMAIN}/slim-controller"]
              issuer: "spiffe://${env:SPIFFE_TRUST_DOMAIN}/slim-issuer"
              subject: "${env:SPIFFE_ID}"
            key:
              type: decoding
              algorithm: "RS256"
              format: jwks
              key:
                file: "/run/spire/jwt-bundle.json"
```

## Configuration Reference Tables

### Endpoint Configuration

The `endpoint` field can be configured as either a network address or a Unix socket:

- Network address: standard TCP address for gRPC/HTTP/2 connections (e.g., `0.0.0.0:8080`, `example.com:443`)
- Unix socket: local socket file path prefixed with `unix://` (e.g., `unix:///var/run/slim.sock`)

!!! warning "Unix Socket Limitations"
    When using Unix sockets, TLS and other transport-related options (such as `tls`, `keepalive`, `proxy`) are not supported and will be ignored. Unix sockets provide local inter-process communication without network transport.

### Server Configuration Options

| Field | Type | Required | Default | Description | Required When |
|-------|------|----------|---------|-------------|---------------|
| `endpoint` | string | ✅ | - | Listen address (network or unix socket) | Always |
| `tls.insecure` | boolean | ❌ | `false` | Disable TLS | - |
| `tls.source` | TlsSource | ⚠️ | `none` | Server certificate source | Required when `tls.insecure=false` |
| `tls.client_ca` | CaSource | ❌ | `none` | Client CA for mTLS | Optional (enables client cert verification) |
| `tls.tls_version` | string | ❌ | `"tls1.3"` | TLS protocol version | - |
| `http2_only` | boolean | ❌ | `true` | HTTP/2 only mode | - |
| `max_frame_size` | integer | ❌ | `4` | Max message size (MiB) | - |
| `max_concurrent_streams` | integer | ❌ | `100` | Max concurrent streams | - |
| `read_buffer_size` | integer | ❌ | `1048576` | Read buffer (bytes) | - |
| `write_buffer_size` | integer | ❌ | `1048576` | Write buffer (bytes) | - |
| `keepalive.max_connection_idle` | duration | ❌ | `"1h"` | Idle timeout | - |
| `keepalive.max_connection_age` | duration | ❌ | `"2h"` | Max connection age | - |
| `keepalive.time` | duration | ❌ | `"2m"` | Keepalive interval | - |
| `keepalive.timeout` | duration | ❌ | `"20s"` | Keepalive timeout | - |
| `auth` | AuthConfig | ❌ | `none` | Authentication config | - |
| `metadata` | object | ❌ | `null` | User metadata | - |

### Client Configuration Options

| Field | Type | Required | Default | Description | Required When |
|-------|------|----------|---------|-------------|---------------|
| `endpoint` | string | ✅ | - | Target endpoint (network or unix socket) | Always |
| `origin` | string | ❌ | `null` | Origin override | - |
| `server_name` | string | ❌ | `null` | SNI override | - |
| `tls.insecure` | boolean | ❌ | `false` | Disable TLS | - |
| `tls.insecure_skip_verify` | boolean | ❌ | `false` | Skip server verification | - |
| `tls.source` | TlsSource | ❌ | `none` | Client certificate | Optional (for mTLS) |
| `tls.ca_source` | CaSource | ❌ | `none` | Server CA verification | Optional (for server verification) |
| `tls.tls_version` | string | ❌ | `"tls1.3"` | TLS protocol version | - |
| `tls.include_system_ca_certs_pool` | boolean | ❌ | `true` | Include system CAs | - |
| `connect_timeout` | duration | ❌ | `"0s"` | Connection timeout | - |
| `request_timeout` | duration | ❌ | `"0s"` | Request timeout | - |
| `buffer_size` | integer | ❌ | `null` | Read buffer size | - |
| `headers` | map | ❌ | `{}` | Custom headers | - |
| `rate_limit` | string | ❌ | `null` | Rate limiting | - |
| `keepalive` | KeepaliveConfig | ❌ | `null` | Keepalive settings | - |
| `proxy` | ProxyConfig | ❌ | - | HTTP proxy config | - |
| `auth` | AuthConfig | ❌ | `none` | Authentication config | - |
| `backoff` | BackoffConfig | ❌ | exponential | Retry backoff | - |
| `metadata` | object | ❌ | `null` | User metadata | - |

### Authentication Types

| Type | Server | Client | Description | Required Fields |
|------|--------|--------|-------------|-----------------|
| `basic` | ✅ | ✅ | Username/password | `username`, `password` |
| `jwt` | ✅ | ✅ | Dynamic JWT generation | `key` (with `algorithm`, `format`, `key.file` or `key.data`) |
| `static_jwt` | ❌ | ✅ | Pre-generated JWT from file | `file` |
| `none` | ✅ | ✅ | No authentication | None |

!!! note
    SPIRE is not a separate authentication type. Instead, SPIRE provides authentication through the TLS layer and the JWT layer.

    SPIRE configuration is done in the `tls` section, not the `auth` section. See [TLS Configuration](#tls-configuration) and [Native SPIRE Integration](#native-spire-integration) for more information.

**JWT Key Requirements:**

- `key.type` - Required: `encoding`, `decoding`, or `autoresolve`
- `key.algorithm` - Required when `type` is `encoding` or `decoding`
- `key.format` - Required when `type` is `encoding` or `decoding` (values: `pem`, `jwk`, `jwks`)
- `key.key.file` or `key.key.data` - Required: one must be provided

### TLS Source Types

| Type | Required Fields | Optional Fields | Description |
|------|-----------------|-----------------|-------------|
| `file` | `cert`, `key` | - | Load certificates from files |
| `pem` | `cert`, `key` | - | Inline PEM certificate data |
| `spire` | - | `socket_path`, `jwt_audiences`, `target_spiffe_id`, `trust_domains` | SPIRE Workload API integration |
| `none` | - | - | No TLS source configured |

**SPIRE Field Details:**

- `socket_path` - Optional (defaults to `SPIFFE_ENDPOINT_SOCKET` env var)
- `jwt_audiences` - Optional (defaults to `["slim"]`)
- `target_spiffe_id` - Optional (for requesting specific SPIFFE ID)
- `trust_domains` - Optional (for X.509 bundle retrieval override)

### CA Source Types

| Type | Required Fields | Optional Fields | Description |
|------|-----------------|-----------------|-------------|
| `file` | `path` | - | Load CA certificates from file |
| `pem` | `data` | - | Inline PEM CA certificate data |
| `spire` | - | `socket_path`, `jwt_audiences`, `target_spiffe_id`, `trust_domains` | SPIRE trust bundle |
| `none` | - | - | No CA source configured |

**SPIRE Field Details:**

- `socket_path` - Optional (defaults to `SPIFFE_ENDPOINT_SOCKET` env var)
- `jwt_audiences` - Optional (defaults to `["slim"]`)
- `target_spiffe_id` - Optional (for requesting specific SPIFFE ID)
- `trust_domains` - Optional (for bundle retrieval override)

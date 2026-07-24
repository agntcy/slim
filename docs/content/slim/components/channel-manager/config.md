# SLIM Channel Manager: Configuration Reference

The Channel Manager is configured with a single YAML file passed via `--config-file`. All settings live under the `channel-manager` key.

## Minimal Configuration

```yaml
channel-manager:
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true

  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true

  local-name: "agntcy/ns/channel-manager"

  auth:
    type: shared_secret
    secret: "a-very-long-shared-secret-abcdef1234567890"
```

## Full Reference

```yaml
channel-manager:

  # ---------------------------------------------------------------------------
  # slim-connection: how the Channel Manager connects to the SLIM data plane node.
  # Transport is inferred from the endpoint scheme:
  #   http://, https://, bare host:port  → gRPC
  #   ws://, wss://                       → WebSocket (TLS when wss://)
  # ---------------------------------------------------------------------------
  slim-connection:
    endpoint: "http://127.0.0.1:46357"

    tls:
      # Set insecure: true to skip TLS verification (development only).
      insecure: true
      # For TLS-secured connections set insecure: false and supply a CA cert:
      # insecure: false
      # ca_file: "/path/to/ca.pem"

    # Authentication used when connecting to the SLIM node (if the node
    # requires it). Example: JWT bearer token from a file.
    # auth:
    #   type: static_jwt
    #   token_file: "/path/to/token"

    # TCP and HTTP/2 keepalive settings for long-lived connections.
    # keepalive:
    #   tcp_keepalive: "60s"
    #   http2_keepalive: "60s"
    #   timeout: "10s"

  # ---------------------------------------------------------------------------
  # api-server: the gRPC server the Channel Manager exposes for management
  # commands (slimctl channel-manager / cmctl).
  # ---------------------------------------------------------------------------
  api-server:
    endpoint: "127.0.0.1:10356"

    tls:
      # Set insecure: true for plaintext gRPC (development only).
      insecure: true
      # For TLS-secured API:
      # insecure: false
      # cert_file: "/path/to/server.pem"
      # key_file:  "/path/to/server-key.pem"

  # ---------------------------------------------------------------------------
  # local-name: the SLIM name this Channel Manager registers as.
  # Format: org/namespace/service  (three components — no clientId).
  # ---------------------------------------------------------------------------
  local-name: "agntcy/ns/channel-manager"

  # ---------------------------------------------------------------------------
  # auth: how this Channel Manager authenticates its own SLIM application
  # identity to the SLIM node.
  # ---------------------------------------------------------------------------
  auth:
    # Shared secret (development / internal networks only).
    type: shared_secret
    secret: "a-very-long-shared-secret-abcdef1234567890"

    # SPIRE Workload API (recommended for production).
    # type: spire
    # socket_path: "unix:/tmp/spire-agent/public/api.sock"

  # ---------------------------------------------------------------------------
  # channels: list of channels to create automatically on startup.
  # The Channel Manager creates each channel and invites all listed
  # participants before the gRPC API begins accepting connections.
  # ---------------------------------------------------------------------------
  channels:
    - name: "agntcy/ns/team-chat"
      # participants to invite when the channel is created.
      participants:
        - "agntcy/ns/agent-1"
        - "agntcy/ns/agent-2"
      # mls-enabled: true enables end-to-end encryption via the MLS protocol.
      # Defaults to true. Set false only when E2E encryption is not required.
      mls-enabled: true

    - name: "agntcy/ns/broadcast"
      participants: []
      mls-enabled: false
```

## Configuration Sections

### `slim-connection`

Controls how the Channel Manager connects to the SLIM data plane node. The endpoint scheme determines the transport:

| Scheme | Transport |
|--------|-----------|
| `http://` | gRPC (plaintext) |
| `https://` | gRPC over TLS |
| `ws://` | WebSocket (plaintext) |
| `wss://` | WebSocket over TLS |

### `api-server`

The gRPC server address where `slimctl channel-manager` and `cmctl` send management commands. Bind to `0.0.0.0:<port>` to accept connections from other hosts.

### `local-name`

The SLIM name the Channel Manager registers as. Use a unique three-component name (`org/namespace/service`) per deployment to avoid conflicts if running multiple Channel Manager instances.

### `auth`

Authentication options for the Channel Manager's SLIM application identity:

| Type | Field | Description |
|------|-------|-------------|
| `shared_secret` | `secret` | Symmetric key. Sufficient for development and trusted networks. |
| `spire` | `socket_path` | SPIRE Workload API socket. Recommended for production deployments. |

### `channels`

Channels listed here are created at startup. For each channel:

- `name` — the SLIM channel name (`org/namespace/channel`, three components)
- `participants` — SLIM application names to invite; the Channel Manager performs discovery and the full invitation handshake for each
- `mls-enabled` — whether to enable MLS end-to-end encryption for the channel (defaults to `true`)

Channels can also be created and participants managed at runtime using `slimctl channel-manager` commands after the service is running.

## cmctl Client Configuration

The standalone `cmctl` tool accepts a `--client-config` YAML file for TLS and auth settings when connecting to the Channel Manager's gRPC API:

```yaml
endpoint: "https://channel-manager.example.com:10356"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
auth:
  type: static_jwt
  token_file: "/path/to/token"
keepalive:
  tcp_keepalive: "60s"
  http2_keepalive: "60s"
  timeout: "10s"
```

Pass it with:

```bash
cmctl --client-config client-config.yaml list-channels
```

## Related

- [Installation Guide](./install.md) — Build and run the Channel Manager
- [Groups](../../architecture/sessions/group.md) — The group communication model and moderator role
- [Authentication](../../architecture/authentication.md) — TLS, mTLS, JWT, and SPIRE authentication options

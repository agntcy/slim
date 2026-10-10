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
      # ca_source:
      #   type: file
      #   path: "/path/to/ca.pem"

    # Authentication used when connecting to the SLIM node (if the node
    # requires it). Example: JWT bearer token from a file.
    # auth:
    #   type: static_jwt
    #   file: "/path/to/token"

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
      # source:
      #   type: file
      #   cert: "/path/to/server.pem"
      #   key:  "/path/to/server-key.pem"
      # For mTLS, also require client certificates. A client certificate's
      # SPIFFE ID then identifies the caller (see ownership.md):
      # client_ca:
      #   type: file
      #   path: "/path/to/client-ca.pem"

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

    # Self-issued JWT identity (did:key): signs EdDSA tokens with an Ed25519
    # key and accepts peers whose tokens verify against a JWKS.
    # type: jwt
    # private_key:
    #   file: "/path/to/cm-ed25519.pem"     # PKCS#8 PEM
    # trusted_keys:
    #   file: "/path/to/members.jwks.json"  # public keys of trusted peers

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

  # ---------------------------------------------------------------------------
  # persistence: optional state store so session state survives restarts.
  # Both path and encryption-passphrase support ${ENV_VAR} and ${file:/path}
  # substitution for secrets manager integration (Kubernetes Secret, Vault,
  # External Secrets Operator, etc.).
  # In production use a private directory with restrictive permissions and
  # never store the passphrase in the config file directly.
  # ---------------------------------------------------------------------------
  # persistence:
  #   path: /var/lib/channel-manager
  #   encryption-passphrase: ${env:CM_PASSPHRASE}
  #   insecure: false
  #   delete-sessions-on-shutdown: false
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
| `jwt` | `private_key`, `trusted_keys` | Ed25519 private key (PKCS#8 PEM) and a JWKS of trusted peers' public keys, each as `file:` or inline `data:`. The Channel Manager signs as the key's `did:key` and logs it at startup, for peers' allow-lists. |

### `channels`

Channels listed here are created at startup. For each channel:

- `name` — the SLIM channel name (`org/namespace/channel`, three components)
- `participants` — SLIM application names to invite; the Channel Manager performs discovery and the full invitation handshake for each
- `mls-enabled` — whether to enable MLS end-to-end encryption for the channel (defaults to `true`)

Channels can also be created and participants managed at runtime using `slimctl channel-manager` commands after the service is running.

### `persistence`

Optional. When present, channels survive a restart: the session layer keeps MLS group state and session records in an encrypted SQLite database under `path`, and the Channel Manager restores them on startup. Each channel's owner, its expiry time (for channels created with a TTL), and the grants already used against it are restored too, from a second encrypted database in the same directory, protected by the same passphrase. A channel whose TTL passed while the Channel Manager was down is deleted right after startup.

- `path` — directory for the encrypted databases
- `encryption-passphrase` — passphrase the encryption key is derived from; required unless `insecure: true`
- `insecure` — allow running without a passphrase; the key is then derived from the public `local-name` and protects against tampering only, not disclosure
- `delete-sessions-on-shutdown` — `true` (default) deletes all channels on a clean shutdown; `false` keeps them for the next start

Without this section everything, channels included, is kept in memory and lost on restart.

The Channel Manager refuses to start if `persistence` is configured but channel ownership can't be loaded: a restored channel without its owner would accept participant changes from anyone.

!!! warning "Run a single replica per state directory"
    Replicas don't share ownership or used-grant state. With two Channel Managers on the same channels, a single-use grant could be accepted once by each.

## Related

- [Installation Guide](./install.md) — Build and run the Channel Manager
- [Channel Ownership, Grants and Expiry](./ownership.md) — Owners, grants and time to live for channels created through the API
- [Groups](../../architecture/sessions/group.md) — The group communication model and moderator role
- [Authentication](../../architecture/authentication.md) — TLS, mTLS, JWT, and SPIRE authentication options

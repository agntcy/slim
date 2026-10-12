# Channel Manager

A Rust application for managing SLIM channels and participants. The Channel Manager creates and maintains SLIM sessions (channels) and handles participant invitations based on a configuration file.

## Overview

The Channel Manager:
- Connects to a SLIM node and creates channels defined in the configuration
- Invites participants to channels automatically on startup
- Exposes a gRPC API for dynamic channel and participant management

## Building

```bash
cargo build --release
```

## Configuration

Create a YAML configuration file (see [example-channel-manager-config.yaml](example-channel-manager-config.yaml)):

```yaml
channel-manager:
  # Connection configuration for the SLIM data-plane node.
  # Transport is inferred from the endpoint scheme:
  #   ws://, wss://      → websocket (TLS when wss)
  #   http://, https://, bare host:port → grpc
  slim-connection:
    endpoint: "http://127.0.0.1:46357"
    tls:
      insecure: true

  # gRPC service address for accepting channel-manager commands.
  api-server:
    endpoint: "127.0.0.1:10356"
    tls:
      insecure: true

  # Name of the channel manager in SLIM
  local-name: "agntcy/otel/channel-manager"

  # Shared secret for the SLIM app identity.
  auth:
    type: shared_secret
    secret: "your-shared-secret-here"

  # Channels to create on startup.
  channels:
    - name: "agntcy/otel/channel"
      participants:
        - "agntcy/otel/exporter-traces"
        - "agntcy/otel/receiver"
      mls-enabled: true
```

## Running

Start the channel manager with a configuration file:

```bash
./channel-manager --config-file config.yaml
```

## Managing Channels

Use `slimctl channel-manager` (alias `cm`). Every change is signed by a channel's owner or an operator, with their Ed25519 key (`--key`) or elsewhere and forwarded:

```bash
# The did:key of the admin key, to list in the config's operators
slimctl cm did-key --key admin.pem

# Create a channel owned by the admin, and invite a client
slimctl cm create-channel org/ns/team-chat --key admin.pem --audience org/ns/channel-manager
slimctl cm add-participant org/ns/team-chat org/ns/client-1 --key admin.pem

# Inspect
slimctl cm list-channels
slimctl cm list-participants org/ns/team-chat
```

See the [ownership documentation](../../docs/content/slim/components/channel-manager/ownership.md).

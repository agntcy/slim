# Channel Manager

A Rust application for managing SLIM channels and participants. The Channel Manager creates and maintains SLIM sessions (channels) and handles participant invitations based on a configuration file.

## Overview

The Channel Manager:
- Connects to a SLIM node and creates channels defined in the configuration
- Invites participants to channels automatically on startup
- Exposes a gRPC API for dynamic channel and participant management

## Configuration

Create a YAML configuration file (see [example-channel-manager-config.yaml](example-channel-manager-config.yaml)):

```yaml
channel-manager:
  # Connection configuration for SLIM node
  connection-config:
    address: "http://127.0.0.1:46357"

  # gRPC service address for accepting commands
  service-address: "127.0.0.1:10356"

  # Name of the channel manager in SLIM
  local-name: "agntcy/otel/channel-manager"

  # Shared secret for MLS and identity provider
  shared-secret: "your-shared-secret-here"

# Channels to create on startup
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

# Channel Manager CTL (cmctl)

A command-line client for interacting with the Channel Manager gRPC service.

## Usage

```bash
cmctl [OPTIONS] <COMMAND>
```

### Options

- `--server <ADDRESS>`: Simple gRPC server address (default: `http://localhost:10356`)
- `--client-config <FILE>`: Path to a YAML file with full gRPC client configuration (TLS, auth, keepalive, proxy, etc.). Uses the same `ClientConfig` format as the SLIM data-plane. When provided, `--server` is ignored.

### Available Commands

#### Create a new channel
```bash
cmctl create-channel org/ns/channel
```

Create a channel with MLS disabled:
```bash
cmctl --disable-mls create-channel org/ns/channel
```

#### Delete a channel
```bash
cmctl delete-channel org/ns/channel
```

#### Add a participant to a channel
```bash
cmctl add-participant org/ns/channel agntcy/ns/participant
```

#### Remove a participant from a channel
```bash
cmctl delete-participant org/ns/channel agntcy/ns/participant
```

#### List all channels
```bash
cmctl list-channels
```

#### List participants in a channel
```bash
cmctl list-participants org/ns/channel
```

### Examples

Connect to a different server:
```bash
cmctl --server "http://192.168.1.100:10356" list-channels
```

Connect using full client config (with TLS, auth, etc.):
```bash
cmctl --client-config client-config.yaml list-channels
```

Example `client-config.yaml`:
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

Create a channel and add participants:
```bash
# Create channel with MLS enabled (default)
cmctl create-channel org/ns/team-chat

# Add participants
cmctl add-participant org/ns/team-chat org/ns/participant-1
cmctl add-participant org/ns/team-chat org/ns/participant-2

# List participants
cmctl list-participants org/ns/team-chat
```

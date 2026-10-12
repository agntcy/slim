# SLIM Channel Manager: Installation Guide

The Channel Manager is available as a pre-built Docker image or can be built from source.

## Docker

```bash
docker pull ghcr.io/agntcy/slim/channel-manager:2.0.0
docker run --rm -v ./config.yaml:/config.yaml ghcr.io/agntcy/slim/channel-manager:2.0.0 --config-file /config.yaml
```

## Build from Source

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (the workspace pins a specific version via `rust-toolchain.toml`)
- The SLIM repository cloned locally

## Build from Source

Clone the repository if you have not already:

```bash
git clone https://github.com/agntcy/slim.git
cd slim
```

Build the Channel Manager binary in release mode:

```bash
cargo build --release -p agntcy-slim-channel-manager
```

This produces the `channel-manager` server binary in `target/release/`. To manage channels from the command line, use [`slimctl channel-manager`](../cli/reference/channel-manager/index.md).

## Running the Channel Manager

Create a configuration file (see [Configuration Reference](./config.md)) and start the server:

```bash
./target/release/channel-manager --config-file config.yaml
```

On a successful start you will see log output confirming the SLIM connection and gRPC server address:

```
INFO channel_manager: Connected to SLIM node at http://127.0.0.1:46357
INFO channel_manager: gRPC API listening on 127.0.0.1:10356
```

## Verifying the Installation

Use `slimctl channel-manager` to confirm the Channel Manager is responding:

```bash
slimctl channel-manager list-channels
```

It connects to `127.0.0.1:10356` by default. Use `--server` to point at a different address.

## Installing slimctl

`slimctl` is required to manage the Channel Manager from the command line. See the [SLIM CLI Installation](../cli/install.md) guide.

## Next Steps

- [Configuration Reference](./config.md) — Configure the SLIM connection, gRPC API server, authentication, and startup channels
- [Creating a Session](../sdk/tutorials/tutorial-session.md) — Create a group session and invite participants
- [Receiving a Session](../sdk/tutorials/tutorial-receive.md) — Listen for session invitations and receive messages

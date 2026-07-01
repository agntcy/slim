# SLIM Channel Manager: Installation Guide

The Channel Manager is distributed as a Rust binary built from source. There is no pre-built binary or container image at this time.

## Prerequisites

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

This produces two binaries in `target/release/`:

| Binary | Description |
|--------|-------------|
| `channel-manager` | The Channel Manager server |
| `cmctl` | Standalone gRPC client for the Channel Manager API |

To build both in one command:

```bash
cargo build --release -p agntcy-slim-channel-manager --bins
```

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

Use `slimctl channel-manager` (or the standalone `cmctl` binary) to confirm the Channel Manager is responding:

```bash
slimctl channel-manager list-channels
```

Or using `cmctl` directly:

```bash
./target/release/cmctl list-channels
```

Both connect to `127.0.0.1:10356` by default. Use `--endpoint` (slimctl) or `--server` (cmctl) to point at a different address.

## Installing slimctl

`slimctl` is required to manage the Channel Manager from the command line. See the [SLIM CLI Installation](../cli/install.md) guide.

## Next Steps

- [Configuration Reference](./config.md) — Configure the SLIM connection, gRPC API server, authentication, and startup channels
- [Group Communication Tutorial](../sdk/tutorial-group.md) — Walkthrough of setting up and using a group channel with the Channel Manager

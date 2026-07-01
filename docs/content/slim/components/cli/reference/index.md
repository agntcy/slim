# slimctl

The `slimctl` command-line tool for operating SLIM. It lets you start and manage SLIM nodes, inspect connections and routes, manage Controller configuration, and interact with the Channel Manager — all from a single binary.

## Usage

```
slimctl [OPTIONS] <COMMAND>
```

## Commands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`slim`](./slim/index.md) | `s` | Start and manage a local SLIM node |
| [`node`](./node/index.md) | `n`, `instance`, `i` | Interact directly with a SLIM node |
| [`controller`](./controller/index.md) | `c`, `ctrl` | Interact with the SLIM Control Plane |
| [`channel-manager`](./channel-manager/index.md) | `cm` | Interact with the Channel Manager |
| [`bench`](./bench/index.md) | — | Benchmark SLIM messaging performance |
| [`config`](./config/index.md) | — | Manage `slimctl` client configuration |
| [`version`](./version.md) | — | Print version and build information |

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | *(varies by command)* | gRPC API endpoint (`host:port`) |
| `--timeout` | — | `15s` | gRPC request timeout (e.g. `30s`, `1m`) |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Use TLS but skip certificate verification |

The default value of `--server` depends on which command is used:

| Command | Default server |
|---------|---------------|
| `slim`, `node` | `127.0.0.1:46358` |
| `controller` | `127.0.0.1:50051` |
| `channel-manager` | `127.0.0.1:10356` |

Most options can also be set via environment variables or a [config file](./config/index.md).

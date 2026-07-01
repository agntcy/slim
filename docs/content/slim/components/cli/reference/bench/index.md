# slimctl bench

Benchmark SLIM messaging performance against a running SLIM node. Inspired by the design of `nats bench`.

Start subscribers before publishers — subscribers must be ready and waiting before the publisher begins sending.

## Usage

```
slimctl bench [OPTIONS] <COMMAND>
```

## Subcommands

| Command | Description |
|---------|-------------|
| [`sub`](./sub.md) | Run subscriber workers for a point-to-point benchmark |
| [`pub`](./pub.md) | Run publisher workers for a point-to-point benchmark |
| [`channel`](./channel/index.md) | Run a group session (channel) benchmark |

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--log-level` | `info` | Log level for SLIM internals (`error`, `warn`, `info`, `debug`, `trace`) |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | `127.0.0.1:46358` | gRPC API endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

## Naming Convention

Benchmark workers register under names derived from `--prefix`:

| Worker type | Name pattern |
|-------------|--------------|
| Subscriber *i* | `<prefix>/sub-<i>` |
| Publisher *i* | `<prefix>/pub-<i>` |

For example, with `--prefix bench/test --count 2`, the two subscribers register as `bench/test/sub-0` and `bench/test/sub-1`.

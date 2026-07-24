# slimctl bench channel

Benchmark group session (channel) performance: one publisher broadcasts to N subscribers.

## Usage

```
slimctl bench channel <COMMAND>
```

## Subcommands

| Command | Description |
|---------|-------------|
| [`sub`](./sub.md) | Start subscriber workers that join a channel |
| [`pub`](./pub.md) | Run the channel publisher |

## Inherited Options

Options inherited from [`slimctl bench`](../index.md):

| Flag | Default | Description |
|------|---------|-------------|
| `--log-level` | `info` | Log level for SLIM internals |

Options inherited from [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

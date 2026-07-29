# slimctl bench pub

Run publisher workers for a point-to-point benchmark. Start subscribers first with [`bench sub`](./sub.md).

## Usage

```
slimctl bench pub [OPTIONS]
```

## Examples

Run 4 concurrent publishers sending 1 000 000 messages of 512 bytes:

```bash
slimctl bench pub --count 4 --msgs 1000000 --size 512
```

Measure request-reply latency (requires `bench sub --reply` running):

```bash
slimctl bench pub --request --msgs 10000
```

Run in reliable mode with acknowledgements:

```bash
slimctl bench pub --reliable --msgs 100000
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--count` | `1` | Number of concurrent publisher apps |
| `--msgs` / `-n` | `100000` | Total messages to publish across all publishers |
| `--size` | `128` | Payload size in bytes. Accepts unit suffixes: `kb`/`mb`/`gb` (SI ×1000) or `kib`/`mib`/`gib` (IEC ×1024). |
| `--server` | `http://localhost:46357` | SLIM server URL |
| `--secret` | *(test default)* | Shared secret for authentication (minimum 32 characters) |
| `--prefix` | `bench/test` | Name prefix in `org/namespace` format. Must match the subscriber's `--prefix`. |
| `--request` | `false` | Request-reply mode: wait for an echo reply after each publish. Requires [`bench sub --reply`](./sub.md). |
| `--reliable` | `false` | Reliable mode: retransmit unacknowledged messages and wait for ACK before sending the next. Default is fire-and-forget. |
| `--start-index` | `0` | Index of the first publisher in this process. Use with `--count` to distribute publishers across multiple processes. |
| `--csv` | — | Append results to a CSV file at the given path |

## Inherited Options

Options inherited from [`slimctl bench`](./index.md):

| Flag | Default | Description |
|------|---------|-------------|
| `--log-level` | `info` | Log level for SLIM internals |

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

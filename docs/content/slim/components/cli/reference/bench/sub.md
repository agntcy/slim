# slimctl bench sub

Run subscriber workers for a point-to-point benchmark. Start these before running [`bench pub`](./pub.md).

## Usage

```
slimctl bench sub [OPTIONS]
```

## Examples

Run 4 concurrent subscribers receiving 1 000 000 messages of 512 bytes:

```bash
slimctl bench sub --count 4 --msgs 1000000 --size 512
```

Run in echo (reply) mode to measure request-reply latency:

```bash
slimctl bench sub --reply
```

Run indefinitely until interrupted:

```bash
slimctl bench sub --msgs 0
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--count` | `1` | Number of concurrent subscriber apps |
| `--msgs` / `-n` | `100000` | Total messages to receive across all subscribers. `0` runs until interrupted. |
| `--size` | `128` | Payload size in bytes. Accepts unit suffixes: `kb`/`mb`/`gb` (SI ×1000) or `kib`/`mib`/`gib` (IEC ×1024). |
| `--server` | `http://localhost:46357` | SLIM server URL |
| `--secret` | *(test default)* | Shared secret for authentication (minimum 32 characters) |
| `--prefix` | `bench/test` | Name prefix in `org/namespace` format. Must match the publisher's `--prefix`. |
| `--reply` | `false` | Echo each received message back to the sender (for latency measurement with `bench pub --request`) |
| `--start-index` | `0` | Index of the first subscriber in this process. Use with `--count` to distribute subscribers across multiple processes. |
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

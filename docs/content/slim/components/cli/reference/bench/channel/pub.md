# slimctl bench channel pub

Run the channel publisher: creates a group session, invites all subscribers, and broadcasts messages to the entire group.

## Usage

```
slimctl bench channel pub [OPTIONS]
```

## Examples

Broadcast to 3 subscribers:

```bash
slimctl bench channel pub --count 3 --msgs 100000
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--count` | `2` | Number of subscribers to invite into the channel. Must match [`bench channel sub --count`](./sub.md). |
| `--msgs` / `-n` | `100000` | Total messages to broadcast |
| `--size` | `128` | Payload size in bytes. Accepts unit suffixes: `kb`/`mb`/`gb` (SI ×1000) or `kib`/`mib`/`gib` (IEC ×1024). |
| `--server` | `http://localhost:46357` | SLIM server URL |
| `--secret` | *(test default)* | Shared secret for authentication (minimum 32 characters) |
| `--prefix` | `bench/test` | Name prefix in `org/namespace` format. Must match the subscriber's `--prefix`. |
| `--reliable` | `false` | Reliable mode: retransmit unacknowledged messages and wait for ACK before sending the next. Default is fire-and-forget. |
| `--csv` | — | Append results to a CSV file at the given path |

## Inherited Options

Options inherited from [`slimctl bench channel`](./index.md) and [`slimctl bench`](../index.md):

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

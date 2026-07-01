# slimctl bench channel sub

Start subscriber workers that join a group channel. Run these before [`bench channel pub`](./pub.md).

## Usage

```
slimctl bench channel sub [OPTIONS]
```

## Examples

Start 3 channel subscribers:

```bash
slimctl bench channel sub --count 3 --msgs 100000
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--count` | `2` | Number of subscriber apps to join the channel. Must match `bench channel pub --count`. |
| `--msgs` / `-n` | `100000` | Messages to receive per subscriber before reporting. `0` runs until interrupted. |
| `--size` | `128` | Payload size in bytes. Accepts unit suffixes: `kb`/`mb`/`gb` (SI ×1000) or `kib`/`mib`/`gib` (IEC ×1024). |
| `--server` | `http://localhost:46357` | SLIM server URL |
| `--secret` | *(test default)* | Shared secret for authentication (minimum 32 characters) |
| `--prefix` | `bench/test` | Name prefix in `org/namespace` format. Must match the publisher's `--prefix`. |
| `--start-index` | `0` | Index of the first subscriber in this process. Use with `--count` to distribute subscribers across multiple processes. |
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

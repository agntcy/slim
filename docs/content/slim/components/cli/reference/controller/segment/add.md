# slimctl controller segment add

Create a new network segment in the Controller. Only available when the Controller is running in API-managed mode. After creating a segment, use `slimctl controller link add --segment <NAME>` to populate it with domain links.

## Usage

```
slimctl controller segment add <NAME>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `NAME` | Segment name |

## Examples

```bash
slimctl controller segment add customer-1
```

## Inherited Options

Options inherited from [`slimctl controller segment`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | — | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

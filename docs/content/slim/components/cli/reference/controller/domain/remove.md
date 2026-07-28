# slimctl controller domain remove

Remove a domain from the Controller, disconnecting all nodes that belong to it. Only available when the Controller is running in API-managed mode.

**Aliases:** `rm`

## Usage

```
slimctl controller domain remove <DOMAIN_NAME>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `DOMAIN_NAME` | Name of the domain to remove |

## Examples

```bash
slimctl controller domain remove cluster-a
```

## Inherited Options

Options inherited from [`slimctl controller domain`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | — | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

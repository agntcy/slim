# slimctl controller link remove

Remove a topology link between two domains. Only available when the Controller is running in API-managed mode.

**Aliases:** `rm`

## Usage

```
slimctl controller link remove [OPTIONS] <DOMAIN_A> <DOMAIN_B>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `DOMAIN_A` | First domain name |
| `DOMAIN_B` | Second domain name |

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--segment` | `-s` | `default` | Segment the link belongs to |

## Examples

```bash
slimctl controller link remove cluster-a cluster-b
```

Remove a link from a named segment:

```bash
slimctl controller link remove cluster-a cluster-b --segment customer-1
```

## Inherited Options

Options inherited from [`slimctl controller link`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | — | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

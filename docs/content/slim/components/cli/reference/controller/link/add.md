# slimctl controller link add

Add a topology link between two domains. Only available when the Controller is running in API-managed mode.

## Usage

```
slimctl controller link add [OPTIONS] <DOMAIN_A> <DOMAIN_B>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `DOMAIN_A` | First domain name |
| `DOMAIN_B` | Second domain name |

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--segment` | `-s` | `default` | Segment to add the link to |

## Examples

Add a link between two domains in the default segment:

```bash
slimctl controller link add cluster-a cluster-b
```

Add a link within a named segment:

```bash
slimctl controller link add cluster-a cluster-b --segment customer-1
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

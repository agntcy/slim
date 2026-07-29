# slimctl controller domain add

Register a new domain with the Controller and set its shared-secret registration credential. Only available when the Controller is running in API-managed mode (no `topology` configured in the Controller config file).

## Usage

```
slimctl controller domain add <DOMAIN_NAME> <SECRET>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `DOMAIN_NAME` | Name of the domain to register |
| `SECRET` | Shared secret nodes in this domain must present on registration |

## Examples

```bash
slimctl controller domain add cluster-a "secret-for-cluster-a-abcdefghi-1234567890"
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

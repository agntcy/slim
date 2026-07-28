# slimctl controller domain

List, add, and remove routing domains and the nodes they contain. The `add` and `remove` subcommands are available in API-managed mode only (when no `topology` is set in the Controller config).

## Usage

```
slimctl controller domain <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`list`](./list.md) | `ls` | List all routing domains and their nodes |
| [`add`](./add.md) | — | Register a domain with a shared secret (API-managed mode only) |
| [`remove`](./remove.md) | `rm` | Remove a domain, disconnecting all its nodes (API-managed mode only) |

## Inherited Options

Options inherited from [`slimctl controller`](../index.md) and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | — | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

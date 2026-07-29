# slimctl controller segment

List, add, and remove network segments (routing domains) and the domains they contain. The `add` and `remove` subcommands are available in API-managed mode only (when no `topology` is set in the Controller config).

## Usage

```
slimctl controller segment <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`list`](./list.md) | `ls` | List all network segments and their domains |
| [`add`](./add.md) | — | Create a new segment (API-managed mode only) |
| [`remove`](./remove.md) | `rm` | Remove a segment and all its links (API-managed mode only) |

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

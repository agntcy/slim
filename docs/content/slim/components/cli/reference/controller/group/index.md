# slimctl controller group

List routing groups and the nodes they contain.

## Usage

```
slimctl controller group <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`list`](./list.md) | `ls` | List all routing groups |

## Inherited Options

Options inherited from [`slimctl controller`](../index.md) and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

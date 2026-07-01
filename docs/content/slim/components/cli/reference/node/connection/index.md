# slimctl node connection

Inspect active connections on a SLIM node.

**Aliases:** `conn`

## Usage

```
slimctl node connection <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`list`](./list.md) | `ls` | List active connections on the node |

## Inherited Options

Options inherited from [`slimctl node`](../index.md) and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:46358` | Address of the SLIM node control endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

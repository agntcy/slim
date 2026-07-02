# slimctl controller segment list

List all network segments (routing domains) and the groups they contain.

**Aliases:** `ls`

## Usage

```
slimctl controller segment list
```

## Examples

```bash
slimctl controller segment list
```

## Options

No options.

## Inherited Options

Options inherited from [`slimctl controller segment`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

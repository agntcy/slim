# slimctl controller connection list

List active connections on a node, queried via the Controller.

**Aliases:** `ls`

## Usage

```
slimctl controller connection list --node-id <NODE_ID>
```

## Examples

```bash
slimctl controller connection list --node-id slim/a
```

## Options

| Flag | Short | Required | Description |
|------|-------|----------|-------------|
| `--node-id` | `-n` | **Yes** | ID of the node to query |

## Inherited Options

Options inherited from [`slimctl controller connection`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

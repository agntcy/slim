# slimctl node connection list

List active connections on a SLIM node.

**Aliases:** `ls`

## Usage

```
slimctl node connection list
```

## Examples

List connections on the default local node:

```bash
slimctl node connection list
```

List connections on a remote node:

```bash
slimctl node connection list --server 192.168.1.10:46358
```

## Options

No options. Use the global `--server` flag to target a specific node.

## Inherited Options

Options inherited from [`slimctl node connection`](./index.md), [`slimctl node`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:46358` | Address of the SLIM node control endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

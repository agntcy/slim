# slimctl controller link list

List inter-node links registered at the Controller.

**Aliases:** `ls`

## Usage

```
slimctl controller link list [OPTIONS]
```

## Examples

List all links:

```bash
slimctl controller link list
```

Filter links from a specific origin node:

```bash
slimctl controller link list --origin-node-id slim/a
```

Filter links to a specific target node:

```bash
slimctl controller link list --target-node-id slim/b
```

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--origin-node-id` | `-o` | — | Filter by source (origin) node ID |
| `--target-node-id` | `-t` | — | Filter by destination (target) node ID |
| `--all` | `-a` | `false` | Include links in all states (pending, failed, deleted). Default shows only applied links. |

## Inherited Options

Options inherited from [`slimctl controller link`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

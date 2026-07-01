# slimctl controller route list

List routes managed by the Controller. Can show routes for a specific node or an aggregated view across the entire network.

**Aliases:** `ls`

## Usage

```
slimctl controller route list [OPTIONS]
```

## Examples

List all routes at the Controller (network-wide view):

```bash
slimctl controller route list
```

List routes for a specific node:

```bash
slimctl controller route list --node-id slim/a
```

Filter by origin and destination node:

```bash
slimctl controller route list --origin-node-id slim/a --target-node-id slim/b
```

Show all routes including pending, failed, and deleted:

```bash
slimctl controller route list --all
```

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--node-id` | `-n` | — | Show routes for a specific node. Conflicts with `--origin-node-id`, `--target-node-id`, and `--all`. |
| `--origin-node-id` | `-o` | — | Filter by source (origin) node ID (network-wide view only) |
| `--target-node-id` | `-t` | — | Filter by destination (target) node ID (network-wide view only) |
| `--all` | `-a` | `false` | Include routes in all states (pending, failed, deleted). Default shows only applied routes. |

## Inherited Options

Options inherited from [`slimctl controller route`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

# slimctl controller

Interact with the SLIM Control Plane to manage nodes, routes, connections, links, groups, and network segments across your SLIM deployment.

**Aliases:** `c`, `ctrl`

**Default server:** `127.0.0.1:50051`

## Usage

```
slimctl controller <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`node`](./node/index.md) | `n`, `nodes`, `instance` | List nodes registered with the Controller |
| [`connection`](./connection/index.md) | `conn` | List active connections via the Controller |
| [`route`](./route/index.md) | — | Manage routes via the Controller |
| [`link`](./link/index.md) | — | List inter-node links |
| [`group`](./group/index.md) | — | List routing groups |
| [`segment`](./segment/index.md) | — | List network segments |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

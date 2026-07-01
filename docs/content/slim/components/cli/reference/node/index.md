# slimctl node

Interact directly with a SLIM node's control API, bypassing the central Controller. Use this during local development or when no Controller is deployed.

**Aliases:** `n`, `instance`, `i`

**Default server:** `127.0.0.1:46358`

## Usage

```
slimctl node <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`route`](./route/index.md) | — | Manage the node's routing table |
| [`connection`](./connection/index.md) | `conn` | Inspect active connections on the node |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | `127.0.0.1:46358` | Address of the SLIM node control endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

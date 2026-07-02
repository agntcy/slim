# slimctl config

Manage the `slimctl` client configuration. Settings are stored in `~/.slimctl/config.yaml` and apply as defaults to all subsequent commands.

## Usage

```
slimctl config <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`list`](./list.md) | `ls` | Show current configuration values |
| [`set`](./set.md) | — | Set a configuration value |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | `127.0.0.1:46358` | gRPC API endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

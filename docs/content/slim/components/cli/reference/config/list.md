# slimctl config list

Show the current configuration values read from the configuration file.

**Aliases:** `ls`

## Usage

```
slimctl config list
```

## Examples

```bash
slimctl config list
```

## Options

No options.

## Inherited Options

Options inherited from [`slimctl config`](./index.md) and [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file to read |
| `--server` | `-s` | `127.0.0.1:46358` | gRPC API endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

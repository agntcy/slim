# slimctl version

Print version and build information for the installed `slimctl` binary.

## Usage

```
slimctl version
```

## Examples

```bash
slimctl version
```

Example output:

```
Version     1.2.0
Build Date  2025-11-01
Git SHA     abc1234
Profile     release
```

## Options

No options.

## Inherited Options

Options inherited from [`slimctl`](./index.md):

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

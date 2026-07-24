# slimctl controller node list

List all SLIM nodes registered with the Controller.

**Aliases:** `ls`

## Usage

```
slimctl controller node list
```

## Examples

```bash
slimctl controller node list
```

Example output:

```
2 node(s) found
Node ID: slim/b  status: CONNECTED
  Connection details:
  - Endpoint: 127.0.0.1:46457
    MtlsRequired: false
    ExternalEndpoint: slim-b.default.svc.cluster.local:46457
Node ID: slim/a  status: CONNECTED
  Connection details:
  - Endpoint: 127.0.0.1:46357
    MtlsRequired: false
    ExternalEndpoint: slim-a.default.svc.cluster.local:46357
```

## Options

No options.

## Inherited Options

Options inherited from [`slimctl controller node`](./index.md), [`slimctl controller`](../index.md), and [`slimctl`](../../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:50051` | Controller gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

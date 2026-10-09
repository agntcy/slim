# slimctl channel-manager list-channels

List all channels managed by this Channel Manager instance, with each channel's owner and expiry time when it has them (see [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md)).

**Aliases:** `lc`

## Usage

```
slimctl channel-manager list-channels
```

## Examples

```bash
slimctl channel-manager list-channels
```

```text
Channels (2):
  - agntcy/tasks/review-42 (owner: did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK, expires: 2026-10-08T18:00:00Z)
  - agntcy/team/broadcast
```

## Options

No options.

## Inherited Options

Options inherited from [`slimctl channel-manager`](./index.md) and [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | — | `127.0.0.1:10356` | Channel Manager gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

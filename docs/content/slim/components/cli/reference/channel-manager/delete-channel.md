# slimctl channel-manager delete-channel

Delete a group channel. A channel with an owner can only be deleted with a delete request its owner signed: see [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md).

**Aliases:** `dc`

## Usage

```
slimctl channel-manager delete-channel <CHANNEL> [OPTIONS]
```

## Examples

```bash
slimctl channel-manager delete-channel agntcy/team/general
```

With the owner's signed delete request, read from a file or from standard input:

```bash
slimctl channel-manager delete-channel agntcy/team/general --owner-proof-file delete.json
```

## Options

| Argument | Required | Description |
|----------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `--owner-proof-file <PATH>` | No | File holding a delete request signed by the channel's owner; `-` reads standard input. Needed when the channel has an owner |

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

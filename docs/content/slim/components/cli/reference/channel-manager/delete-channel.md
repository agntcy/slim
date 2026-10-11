# slimctl channel-manager delete-channel

Delete a group channel, with a delete request signed by its owner or by an operator: see [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md#delete-request). `slimctl` forwards the signed request without reading it.

**Aliases:** `dc`

## Usage

```
slimctl channel-manager delete-channel <CHANNEL> --signed-request-file <PATH>
```

## Examples

With the signed delete request in a file, as the channel's owner or an operator produced it:

```bash
slimctl channel-manager delete-channel agntcy/team/general --signed-request-file delete.json
```

## Options

| Argument | Required | Description |
|----------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `--signed-request-file <PATH>` | **Yes** | File holding the delete request signed by the channel's owner or an operator; `-` reads standard input |

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

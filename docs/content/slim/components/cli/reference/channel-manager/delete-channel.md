# slimctl channel-manager delete-channel

Delete a group channel, with a delete request signed by its owner or by an operator: signed here with `--key`, or signed elsewhere and forwarded with `--signed-request-file`. See [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md#delete-request).

**Aliases:** `dc`

## Usage

```
slimctl channel-manager delete-channel <CHANNEL> (--key <PATH> --audience <AUDIENCE> | --signed-request-file <PATH>)
```

## Examples

Signed with the admin's key:

```bash
slimctl channel-manager delete-channel agntcy/team/general --key admin.pem --audience agntcy/prod/channel-manager
```

With a delete request signed elsewhere, read from a file:

```bash
slimctl channel-manager delete-channel agntcy/team/general --signed-request-file delete.json
```

## Options

| Argument | Required | Description |
|----------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `--key <PATH>` | One of these two | Ed25519 private key (PKCS#8 PEM) of the channel's owner or an operator, to sign the request with |
| `--signed-request-file <PATH>` | One of these two | File holding a delete request signed elsewhere; `-` reads standard input |
| `--audience <AUDIENCE>` | With `--key` | The Channel Manager the request is for: its [`audience`](../../../channel-manager/config.md#audience), its `local-name` by default. Defaults to `$SLIMCTL_CM_AUDIENCE` |

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

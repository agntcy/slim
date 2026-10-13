# slimctl channel-manager create-channel

Create a group channel. Whoever signs the create request owns the channel: sign it here with `--key`, or forward one signed elsewhere with `--signed-request-file`. See [Channel Manager: Concepts and Ownership](../../../channel-manager/ownership.md).

**Aliases:** `cc`

## Usage

```
slimctl channel-manager create-channel <CHANNEL> (--key <PATH> --audience <AUDIENCE> | --signed-request-file <PATH>) [OPTIONS]
```

## Examples

Create a channel owned by the admin key, with MLS (the default):

```bash
slimctl channel-manager create-channel agntcy/team/general --key admin.pem --audience agntcy/prod/channel-manager
```

A task channel deleted after an hour, whose owner is asked to approve participant changes others request:

```bash
slimctl channel-manager create-channel agntcy/tasks/review-42 \
  --ttl-seconds 3600 --owner-callback-name agntcy/people/alice \
  --key admin.pem --audience agntcy/prod/channel-manager
```

## Options

| Argument / Flag | Default | Required | Description |
|-----------------|---------|----------|-------------|
| `<CHANNEL>` | — | **Yes** | Channel name in `org/namespace/channel` format |
| `--key <PATH>` | — | One of these two | Ed25519 private key (PKCS#8 PEM) to sign the request with. Its did:key becomes the owner |
| `--signed-request-file <PATH>` | — | One of these two | File holding a create request signed elsewhere, covering these exact settings; `-` reads standard input |
| `--audience <AUDIENCE>` | `$SLIMCTL_CM_AUDIENCE` | With `--key` | The Channel Manager the request is for: its [`audience`](../../../channel-manager/config.md#audience), its `local-name` by default |
| `--disable-mls` | `false` | No | Disable MLS end-to-end encryption for this channel |
| `--owner-callback-name <NAME>` | — | No | SLIM name (`org/namespace/app`) at which the owner is asked to approve participant changes others request without a grant |
| `--ttl-seconds <SECONDS>` | — | No | Delete the channel automatically this many seconds after creation |

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

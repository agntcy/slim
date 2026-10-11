# slimctl channel-manager create-channel

Create a new group channel. With a create request signed by its owner, the channel has that owner; without one it has none. Your login to the Channel Manager never makes you an owner. See [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md).

**Aliases:** `cc`

## Usage

```
slimctl channel-manager create-channel <CHANNEL> [OPTIONS]
```

## Examples

Create a channel with MLS encryption enabled (default):

```bash
slimctl channel-manager create-channel agntcy/team/general
```

Create a channel without MLS encryption:

```bash
slimctl channel-manager create-channel agntcy/team/broadcast --disable-mls
```

Create an owned task channel that is deleted after an hour, and that asks its owner to approve participant changes others request. `create.json` is the owner's signed create request for exactly these settings:

```bash
slimctl channel-manager create-channel agntcy/tasks/review-42 \
  --ttl-seconds 3600 --owner-callback-name agntcy/people/alice \
  --owner-proof-file create.json
```

## Options

| Argument / Flag | Default | Required | Description |
|-----------------|---------|----------|-------------|
| `<CHANNEL>` | — | **Yes** | Channel name in `org/namespace/channel` format |
| `--disable-mls` | `false` | No | Disable MLS end-to-end encryption for this channel (MLS is enabled by default) |
| `--owner-callback-name <NAME>` | — | No | SLIM name (`org/namespace/app`) at which the owner is asked to approve participant changes others request without a grant. Needs `--owner-proof-file` |
| `--ttl-seconds <SECONDS>` | — | No | Delete the channel automatically this many seconds after creation |
| `--owner-proof-file <PATH>` | — | No | File holding the create request signed by the channel's owner, covering these exact settings; `-` reads standard input. Its signer becomes the owner |

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

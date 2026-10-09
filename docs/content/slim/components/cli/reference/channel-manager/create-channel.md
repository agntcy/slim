# slimctl channel-manager create-channel

Create a new group channel. When the Channel Manager's API requires authentication, you become the channel's owner: see [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md).

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

Create a task channel that is deleted after an hour, and that asks you to approve participant changes others request:

```bash
slimctl channel-manager create-channel agntcy/tasks/review-42 \
  --ttl-seconds 3600 --owner-callback-name agntcy/people/alice
```

## Options

| Argument / Flag | Default | Required | Description |
|-----------------|---------|----------|-------------|
| `<CHANNEL>` | — | **Yes** | Channel name in `org/namespace/channel` format |
| `--disable-mls` | `false` | No | Disable MLS end-to-end encryption for this channel (MLS is enabled by default) |
| `--owner-callback-name <NAME>` | — | No | SLIM name (`org/namespace/app`) at which you, as the owner, are asked to approve participant changes others request without a grant |
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

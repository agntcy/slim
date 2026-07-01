# slimctl channel-manager create-channel

Create a new group channel.

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

## Options

| Argument / Flag | Default | Required | Description |
|-----------------|---------|----------|-------------|
| `<CHANNEL>` | — | **Yes** | Channel name in `org/namespace/channel` format |
| `--disable-mls` | `false` | No | Disable MLS end-to-end encryption for this channel (MLS is enabled by default) |

## Inherited Options

Options inherited from [`slimctl channel-manager`](./index.md) and [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server` | `-s` | `127.0.0.1:10356` | Channel Manager gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

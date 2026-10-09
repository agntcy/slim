# slimctl channel-manager delete-participant

Remove a participant from a channel. When MLS is enabled, the Channel Manager distributes updated key material to remaining members.

**Aliases:** `dp`

## Usage

```
slimctl channel-manager delete-participant <CHANNEL> <PARTICIPANT> [OPTIONS]
```

On a channel with an owner, anyone other than the owner needs the owner's authorization: a grant passed with `--grant-file`, or, without one, the owner's approval, which the Channel Manager requests if the owner gave a callback name. See [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md).

## Examples

```bash
slimctl channel-manager delete-participant agntcy/team/general agntcy/agents/assistant-1
```

With a grant from the channel's owner, read from a file or from standard input:

```bash
slimctl channel-manager delete-participant agntcy/team/general agntcy/agents/assistant-1 --grant-file grant.json
```

## Options

| Argument / Flag | Required | Description |
|-----------------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `<PARTICIPANT>` | **Yes** | Participant application name in `org/namespace/app` format |
| `--grant-file <PATH>` | No | File holding a grant signed by the channel's owner authorizing this change, with action `delete`; `-` reads standard input |

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

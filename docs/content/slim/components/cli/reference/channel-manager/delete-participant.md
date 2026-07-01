# slimctl channel-manager delete-participant

Remove a participant from a channel. When MLS is enabled, the Channel Manager distributes updated key material to remaining members.

**Aliases:** `dp`

## Usage

```
slimctl channel-manager delete-participant <CHANNEL> <PARTICIPANT>
```

## Examples

```bash
slimctl channel-manager delete-participant agntcy/team/general agntcy/agents/assistant-1
```

## Options

| Argument | Required | Description |
|----------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `<PARTICIPANT>` | **Yes** | Participant application name in `org/namespace/app` format |

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

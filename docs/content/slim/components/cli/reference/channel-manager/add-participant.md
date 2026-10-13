# slimctl channel-manager add-participant

Add a participant to a group channel. On a channel with an owner, the change needs the owner's grant: signed here with `--key`, or signed elsewhere and passed with `--grant-file`. See [Channel Manager: Concepts and Ownership](../../../channel-manager/ownership.md).

**Aliases:** `ap`

## Usage

```
slimctl channel-manager add-participant <CHANNEL> <PARTICIPANT> (--key <PATH> | --grant-file <PATH>)
```

## Examples

Invite a client with a grant signed by the admin key:

```bash
slimctl channel-manager add-participant agntcy/team/general agntcy/agents/assistant-1 --key admin.pem
```

With a grant from the channel's owner, read from a file:

```bash
slimctl channel-manager add-participant agntcy/team/general agntcy/agents/assistant-1 --grant-file grant.json
```

## Options

| Argument / Flag | Required | Description |
|-----------------|----------|-------------|
| `<CHANNEL>` | **Yes** | Channel name in `org/namespace/channel` format |
| `<PARTICIPANT>` | **Yes** | Participant name in `org/namespace/app` format |
| `--key <PATH>` | One of these two | The channel owner's Ed25519 private key (PKCS#8 PEM), to sign a grant for this change with |
| `--grant-file <PATH>` | One of these two | File holding a grant signed elsewhere for this change, with action `add`; `-` reads standard input |

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

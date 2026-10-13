# slimctl channel-manager

Manage the group channels of a SLIM Channel Manager. Every change is signed by a channel's owner or an operator: here with their Ed25519 key (`--key`), or elsewhere and forwarded. `slimctl` never sends an unsigned change. See [Channel Manager: Concepts and Ownership](../../../channel-manager/ownership.md#clients).

**Aliases:** `cm`

**Default server:** `127.0.0.1:10356`

## Usage

```
slimctl channel-manager <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`create-channel`](./create-channel.md) | `cc` | Create a channel, owned by whoever signs the request |
| [`delete-channel`](./delete-channel.md) | `dc` | Delete a channel, with a delete request its owner or an operator signed |
| [`add-participant`](./add-participant.md) | `ap` | Add a participant, with a grant from the channel's owner |
| [`delete-participant`](./delete-participant.md) | `dp` | Remove a participant, with a grant from the channel's owner |
| [`list-channels`](./list-channels.md) | `lc` | List all channels |
| [`list-participants`](./list-participants.md) | `lp` | List participants in a channel |
| [`did-key`](./did-key.md) | — | Print the did:key of an Ed25519 key |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | — | `127.0.0.1:10356` | Channel Manager gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

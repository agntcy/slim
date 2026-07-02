# slimctl channel-manager

Interact with the SLIM Channel Manager service to create and manage group channels and their participants.

**Aliases:** `cm`

**Default server:** `127.0.0.1:10356`

## Usage

```
slimctl channel-manager <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`create-channel`](./create-channel.md) | `cc` | Create a new channel |
| [`delete-channel`](./delete-channel.md) | `dc` | Delete a channel |
| [`add-participant`](./add-participant.md) | `ap` | Add a participant to a channel |
| [`delete-participant`](./delete-participant.md) | `dp` | Remove a participant from a channel |
| [`list-channels`](./list-channels.md) | `lc` | List all channels |
| [`list-participants`](./list-participants.md) | `lp` | List participants in a channel |

## Inherited Options

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file |
| `--server` | `-s` | `127.0.0.1:10356` | Channel Manager gRPC endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

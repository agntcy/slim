# slimctl channel-manager

Inspect the group channels a SLIM Channel Manager manages, and delete one with a signed request. `slimctl` never creates channels or changes their participants on its own authority: applications do that through the Channel Manager API with the owner's signature. See [Channel Ownership, Grants and Expiry](../../../channel-manager/ownership.md#clients).

**Aliases:** `cm`

**Default server:** `127.0.0.1:10356`

## Usage

```
slimctl channel-manager <COMMAND>
```

## Subcommands

| Command | Aliases | Description |
|---------|---------|-------------|
| [`delete-channel`](./delete-channel.md) | `dc` | Delete a channel, with a delete request its owner or an operator signed |
| [`list-channels`](./list-channels.md) | `lc` | List all channels |
| [`list-participants`](./list-participants.md) | `lp` | List participants in a channel |

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

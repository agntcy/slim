# Operations and Troubleshooting

What the Channel Manager reports about itself, how to inspect a running one, and what each refusal means.

## Logs

The Channel Manager logs at `info` by default, and `RUST_LOG` overrides that for everything:

```bash
RUST_LOG=agntcy_slim_channel_manager=debug,channel_manager=debug ./channel-manager --config-file config.yaml
```

### Startup

A healthy start reports, in order, what the Channel Manager is and what it will accept. The samples below show each line's message and fields; the subscriber also prefixes a timestamp, the thread name and the source location:

```text
INFO Configuration loaded config_file=config.yaml
INFO Channel manager identity did_key=did:key:z6Mk...
INFO Owner approval identity did_key=did:key:z6Mk...
INFO Connected to SLIM node endpoint=http://127.0.0.1:46357 conn_id=1
INFO SLIM app created local_name=agntcy/ns/channel-manager
INFO Sessions found in persistent storage count=2
INFO Restored session from persistent storage channel=agntcy/team/general
INFO Signed create and delete requests must name this audience audience=agntcy/ns/channel-manager
INFO Refusing unsigned creates and changes to channels with no owner
INFO Operator: may delete any channel operator=did:key:z6Mk...
INFO Starting gRPC server endpoint=127.0.0.1:10356
```

Read it as a configuration check, because several lines appear only when the corresponding thing is configured:

| Line | Appears when | Tells you |
|---|---|---|
| `Channel manager identity` | `auth` is a `jwt` identity | The `did:key` peers put in their allow-lists |
| `Owner approval identity` | `auth` or `approval-auth` is a `jwt` identity | The `did:key` owner endpoints should pin |
| `Sessions found in persistent storage` | always — `count=0` without `persistence` | How many channels came back |
| `Restored session from persistent storage` | once per channel restored | Which channels came back |
| `Signed create and delete requests must name this audience` | always | The value clients must sign as `audience` |
| `Refusing unsigned creates and changes to channels with no owner` | `require-signatures: true` | Signatures are mandatory |
| `Operator: may delete any channel` | once per configured operator | The full operator list, auditable from the logs |

If a line you expect is absent, the setting is absent too. No `Owner approval identity` means the approval app has no did:key identity and cannot be pinned; no operator lines mean an ownerless channel cannot be deleted once signatures are required.

`Starting gRPC server` is the last line before the API accepts connections. Everything before it — the SLIM connection, the app registration, restoring persisted sessions, and in config mode creating and reconciling every declared channel — has already succeeded by then.

In config mode, reconciliation is reported per channel:

```text
INFO Channel restored from DB; reconciling participants channel=agntcy/ns/team-chat
INFO Removing participant no longer in config channel=agntcy/ns/team-chat participant=agntcy/ns/agent-3
INFO Channel ready channel=agntcy/ns/team-chat participants=["agntcy/ns/agent-1", "agntcy/ns/agent-2"]
INFO Channel removed from config; deleting session channel=agntcy/ns/old-chat
```

Startup warnings worth alerting on: `Failed to restore sessions`, `failed to restore session`, `Skipping restored session with no string name in destination`, and `failed to pin default gateway to conn N`. Each leaves the Channel Manager running with less state than expected.

Two startup lines look alarming and are not. `dropping owner record of a channel that was not restored` and `dropping expiry record of a channel that was not restored` are logged at `INFO` when a channel was deleted while the Channel Manager was down: the records are dropped deliberately, so that an owner cannot attach itself to a later channel of the same name.

### Requests

Successful changes are logged with the caller's verified identity, when there is one:

```text
INFO Created channel agntcy/team/general caller=Some(CallerIdentity { subject: "spiffe://example.org/ns/default/sa/orchestrator" })
INFO Added participant agntcy/agents/assistant-1 to channel agntcy/team/general caller=Some(...)
INFO Removed participant agntcy/agents/assistant-1 from channel agntcy/team/general caller=Some(...)
INFO Deleted channel agntcy/team/general caller=Some(...) signed_by=Some("did:key:z6Mk...")
INFO Listing channels, count: 2 caller=Some(...)
INFO Listing participants for channel agntcy/team/general, count: 2 caller=Some(...)
```

`caller=None` means the request carried neither verified token claims with a `sub` nor a client certificate with a SPIFFE ID. That is not an error, and such calls are still served — the caller's identity authorizes nothing either way. `signed_by` on a deletion is whose signature was accepted: the channel's owner, or an operator.

!!! warning "A refused request is not logged"
    Authorization failures — an invalid grant, a replayed nonce, a wrong audience, a denied approval — are returned to the caller in `error_msg` and produce **no log line of their own**. Capture client-side errors if you need an audit trail of refusals. At `debug` level you at least see that a request arrived (`Received create_channel request`), though not its outcome.

What does get logged on failure is the Channel Manager's own trouble, at `ERROR`: `Failed to create channel`, `Failed to record state of channel`, `Failed to roll back channel`, `Failed to delete channel`, `Failed to clean up duplicate session`. These carry the underlying error, which the response deliberately does not.

### Expiry and shutdown

The reaper runs every 30 seconds:

```text
INFO Deleted expired channel channel=agntcy/team/standup
WARN Failed to delete expired channel channel=agntcy/team/standup error=...
```

A failed deletion is not retried: the channel is dropped from the list either way, and its records are forgotten, so the warning appears once rather than every 30 seconds.

A clean shutdown reports what it did with the channels:

```text
INFO Shutdown signal received
INFO Shutting down...
INFO All sessions deleted
INFO Shutdown complete
```

`All sessions deleted` means the channels were removed from SLIM — the default, `delete-sessions-on-shutdown: true`. `All sessions closed` means they were left in place for the next start, which is what a persistent deployment wants.

## Inspecting a running Channel Manager

Neither list command signs anything, so both work without a key:

```bash
slimctl channel-manager list-channels
slimctl channel-manager list-participants agntcy/team/general
```

```text
Channels (2):
  - agntcy/team/general (owner: did:key:z6Mkhe..., expires: 2026-10-14T08:00:00Z)
  - agntcy/team/broadcast
```

This is the main operational view, and three things can be read from it:

- **A channel with no owner** — `agntcy/team/broadcast` above — accepts changes from anyone who can reach the API, and will become unchangeable the moment `require-signatures` is turned on. Audit for these before turning it on.
- **An owner that is not a `did:key`** is a channel [frozen by a login identity](./security.md#channels-frozen-by-a-login-identity): its participants can no longer be changed, and only an operator can delete it.
- **An expiry in the past** on a channel still listed is normal for up to 30 seconds, until the reaper's next pass.

Deleting a channel is the one operational action that needs a key, and the only one an operator can perform:

```bash
slimctl channel-manager delete-channel agntcy/team/general \
  --key operator.pem --audience agntcy/ns/channel-manager
```

[Managing Channels with slimctl](./slimctl.md) covers the administrator's commands in full.

## Troubleshooting

### It will not start

| Message | Cause | Fix |
|---|---|---|
| `local-name must be in org/namespace/app format, got: ...` | `local-name` does not have exactly three components | Use `org/namespace/app` |
| `channel[0].name must be in org/namespace/channel format, got: ...` | A configured channel's name is not three components | Same shape as above |
| `channel[0].participants cannot be empty` | A configured channel lists no participants | Give it participants, or remove the channel and create it through the API |
| `persistence requires encryption-passphrase; set insecure: true to allow unencrypted storage (not recommended for production)` | `persistence` without a passphrase | Supply `encryption-passphrase`, ideally as `${env:...}` |
| `persistence.insecure and persistence.encryption-passphrase are mutually exclusive` | Both set | Keep the passphrase, drop `insecure` |
| `channel-manager.audience cannot be empty` | `audience:` present but empty | Give it a value or remove the key to default to `local-name` |
| `channel-manager.operators cannot contain an empty entry` | An empty string in `operators` | Remove it |
| `failed to load channel ownership and grant state` | The state store could not be read — wrong passphrase, or a corrupt or unreadable directory | Deliberate: a restored channel without its owner would accept changes from anyone. Fix the passphrase or the volume |
| `failed to open channel-manager state store` | `persistence.path` is not writable by the container's user | Fix volume ownership and permissions |
| `failed to connect to SLIM node` | The data plane is unreachable, or TLS does not verify | Check `slim-connection.endpoint` and its `tls` settings |
| A `jwt` identity refuses to start | `trusted_keys` does not include the Channel Manager's own public key | Add it: MLS verifies its own token against that JWKS |
| `Failed to reconcile channels from config` | A configured channel could not be created, or a participant could not be invited | Config-mode invites require the participants to be online and reachable. Check the names and that they are running |

### A change is refused

Every message below comes back in the response's `error_msg`. For the four mutating commands `slimctl` prints it after `Command failed:`; the two list commands print it on its own.

| Message | Cause | Fix |
|---|---|---|
| `channel manager is running in config mode; modify the config file and restart to change channels or participants` | The config file declares channels, so the API serves no changes | Edit the file and restart, or run a Channel Manager with no `channels:` for API-mode use |
| `channel <name> already exists` | A channel of that name is already moderated here | Pick another name, or delete the existing one |
| `channel <name> not found` | No such channel here | Check `list-channels`; it may have expired |
| `invalid channel name: ...`, `invalid participant name: ...`, `invalid owner callback name: ...` | A name is not `org/namespace/name` | Use three components |
| `ttl-seconds must be positive` | `ttl_seconds` was `0` | Omit the field for a channel that does not expire |
| `this channel manager requires a create request signed by the channel's owner` | `require-signatures` is on and the create was unsigned | Sign the create request |
| `owner-callback-name needs a signed create request: a channel with no owner has no one to ask` | `owner_callback_name` without `owner_proof` | Sign the create request, or drop the callback name |
| `signed create request does not match this request` | The proof's settings differ from the request's | Build the request from the values that were signed |
| `signed create request is for "x", not this channel manager ("y")` | Wrong `audience` | Sign with the `audience` logged at startup |
| `signed create request is valid for too long: not_after may be at most 24 hours ahead` | `not_after` more than 24 hours out | Sign for a shorter window |
| `... has expired` | The statement's `not_after` has passed | Re-sign. If it keeps happening on fresh statements, the clocks disagree — check NTP on both machines |
| `... has already been used` | The statement's nonce was spent on this channel | Every statement needs a fresh nonce; a retry must be a new statement, not a resend |
| `could not record the ...'s use: ...` | The nonce could not be written to the state store | Refused deliberately, to keep it non-replayable. Fix the store (disk, permissions, passphrase) |
| `invalid grant: the signature does not verify against the signer's key` | The grant was not signed by this channel's owner — commonly an operator's key, or the wrong owner key | Use the channel's owner's key; `list-channels` shows which `did:key` that is |
| `invalid grant: unsupported owner identifier format: ...` | The channel's owner is not a `did:key` — a channel frozen by a login identity | Recreate the channel with a signed create request; an operator can delete the old one |
| `invalid grant: malformed: ...` | Not the expected JSON, an unknown `action`, or a NUL byte in a field | Check the grant against [the wire format](./ownership.md#grant) |
| `grant does not authorize this change` | The grant's channel, invitee or action is not the one requested | A grant is specific: `add` cannot be spent on a removal, and vice versa |
| `channel <name> requires a grant from its owner` | No grant presented, and the channel has no callback name | Present a grant, or recreate the channel with `owner_callback_name` |
| `channel owner did not answer the approval request in time` | No answer within 60 seconds | Check the owner's endpoint is running and subscribed to its callback name |
| `could not ask the channel owner for approval: approval call failed: ...` | The owner's endpoint is unreachable or its handler failed | As above. Verify the endpoint's app is `Bidirectional` and its SLIM identity is trusted by the approval app |
| `could not ask the channel owner for approval: invalid owner callback name: ...` | The recorded callback name is not a valid SLIM name | Recreate the channel with a valid name |
| `could not ask the channel owner for approval: malformed approval response: ...` | The endpoint answered with something that is not an `ApprovalResponse`, or with no decision | Fix the handler: it must return a `grant` or a `denied` |
| `channel owner denied the request: <reason>` | The owner declined | Not a fault; the reason comes from the owner's endpoint |
| `channel <name> has no owner to sign for changes, and this channel manager refuses unsigned ones` | `require-signatures` is on and the channel has no owner | The channel cannot be changed. Recreate it with a signed create request |
| `channel <name> can only be deleted with a request signed by its owner` | No `owner_proof` on a delete for an owned channel | Sign the delete request with the owner's or an operator's key |
| `channel <name> has no owner: only a delete request signed by an operator can delete it` | `require-signatures` is on, the channel has no owner | Delete it with an operator's signed request |
| `channel <name> has no owner and this channel manager has no operators, so it can't be deleted` | As above, but a signed request was sent and there is no operator to verify it against | Configure an operator and restart |
| `signed delete request is for a different channel` | The proof names another channel | Sign a request for this channel |
| `failed to invite participant <p> to channel <c>: ...` | The SLIM invitation or its key rotation failed | Usually the participant is offline or unreachable. Confirm it is running and subscribed |
| `failed to create channel <name>` | The SLIM session could not be created | The cause is in the Channel Manager's `ERROR` log, not in the response |
| `failed to record state of channel <name>` | The owner or expiry could not be persisted; the channel was rolled back | Fix the state store, then create the channel again |

### Behaviour that looks wrong but is not

**An expired channel is still listed.** The reaper runs every 30 seconds. A channel can outlive its time to live by that much.

**The Channel Manager is in every participant list.** It is the channel's moderator, and therefore a member of the group.

**The owner has to present a grant for their own change.** There is no exemption. The Channel Manager cannot recognize the owner on a connection, because the connection's identity is not the owner's key.

**A channel recreated under the same name rejects a grant that looks fresh.** Spent nonces survive channel deletion, deliberately: a recreated channel must still refuse statements already used against the name.

**The caller is authenticated but still cannot change anything.** Authentication says who connected. Only a signature authorizes a change. See [Security Model](./security.md).

## Related

- [API Reference](./api.md) — Which check produces which error
- [Security Model](./security.md) — Why each refusal exists
- [Deployment Guide](./deploy.md) — Probes, persistence and single-replica operation
- [Managing Channels with slimctl](./slimctl.md) — The administrator's commands

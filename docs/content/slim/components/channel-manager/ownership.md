# Channel Ownership, Grants and Expiry

Channels created through the Channel Manager API can have an owner who controls who joins, and a time to live after which they are deleted. None of this applies to channels declared in the config file (config mode): those have no owner and no expiry, and write operations are refused in config mode, as before.

## Owners

A channel's owner is whoever signs its create request, identified by the key they sign with: a `did:key` encoding an Ed25519 public key. The signed request is passed with `--owner-proof-file`, and must cover the exact settings the channel is created with.

The caller's authenticated identity never makes anyone an owner. The API server's authentication (a token's `sub`, or the SPIFFE ID of an mTLS client certificate) says which user or workload opened the connection, not which application owns the channel. That identity is still logged, and passed to owners as the requester of an approval. A channel created without a signed request has no owner, and anyone who can reach the API can change it.

`slimctl channel-manager list-channels` shows each channel's owner.

## Changing an owned channel

Every change to an owned channel needs its owner's signature, including changes the owner makes:

- **Participants:** each addition or removal needs a grant, a statement signed by the owner authorizing one specific change, passed with `--grant-file`. If the request carries no grant and the owner gave `--owner-callback-name` when creating the channel, the Channel Manager asks the owner for one, as described below.
- **Deleting the channel:** needs the owner's signed delete request, passed with `--owner-proof-file`.

A grant must name the channel, the participant and the action (`add` or `delete`) of the request it is used for. Grants and signed requests must not be expired, and each is accepted only once. On a channel with no owner, participants are added and removed, and the channel deleted, without any of them. With [`require-signatures`](#requiring-signatures), they can't be at all.

### Channels owned by a login identity

Earlier versions recorded the creator's authenticated identity as the owner. Such a channel keeps that owner, but no signature can be checked against an OIDC subject or a SPIFFE ID, so its participants can no longer be changed. An [operator](#operators) can delete it, and it still expires if it was created with a time to live.

## Requiring signatures

By default a channel can still be created without a signature, and then has no owner, so existing clients keep working while they move to signed requests. With [`require-signatures`](./config.md#require-signatures) set, the Channel Manager refuses an unsigned create, and refuses every change to a channel that has no owner, such as one created before it was set. Only an operator can delete such a channel.

## Operators

An operator is a did:key listed in the Channel Manager's [`operators`](./config.md#operators). Their signed delete request, in the same format as an owner's and passed the same way with `--owner-proof-file`, deletes any channel: one whose owner's key is lost, one frozen by a login identity owner, or one with no owner. Operators don't own channels, and can't add or remove participants.

## Signed formats

The default verifier expects an owner to be a `did:key` encoding an Ed25519 public key. Every signed statement is a JSON document whose `signature` is the standard, padded base64 encoding of an Ed25519 signature over its signed bytes: the UTF-8 bytes of a literal tag, then the fields listed, in that order, joined with a NUL byte, with no trailing NUL. Numbers are in decimal. `not_after` is in Unix seconds. `nonce` must be unique per statement. No field may contain a NUL byte.

### Grant

A grant is this JSON document:

```json
{
  "channel": "agntcy/team/general",
  "invitee": "agntcy/agents/assistant-1",
  "action": "add",
  "role": "member",
  "not_after": 1767225600,
  "nonce": "4f9c2e7a1b",
  "signature": "<base64>"
}
```

Signed bytes: `SLIM-CHANNEL-GRANT/1`, `channel`, `invitee`, `action`, `role`, `not_after`, `nonce`. `role` is carried but not enforced.

### Create request

A signed create request names its signer as `owner`, and the Channel Manager it is meant for as `audience`:

```json
{
  "audience": "agntcy/prod/channel-manager",
  "channel": "agntcy/team/general",
  "owner": "did:key:z6Mk...",
  "mls_enabled": true,
  "ttl_seconds": 86400,
  "owner_callback_name": "agntcy/owners/alice-desktop",
  "not_after": 1767225600,
  "nonce": "b81d4f0c9e",
  "signature": "<base64>"
}
```

Signed bytes: `SLIM-CHANNEL-CREATE/1`, `audience`, `channel`, `owner`, `mls_enabled` (`true` or `false`), `ttl_seconds`, `owner_callback_name`, `not_after`, `nonce`.

`ttl_seconds` and `owner_callback_name` are optional. When unset, omit them from the JSON, and sign each as the empty string. The signature is checked with the key in `owner`, and `channel`, `mls_enabled`, `ttl_seconds` and `owner_callback_name` must equal the request's settings.

`audience` is the Channel Manager's [`audience`](./config.md#audience), its `local-name` by default, so a request signed for one Channel Manager is refused by another. `not_after` may be at most 24 hours ahead.

### Delete request

```json
{
  "audience": "agntcy/prod/channel-manager",
  "channel": "agntcy/team/general",
  "not_after": 1767225600,
  "nonce": "07a3e5d2f1",
  "signature": "<base64>"
}
```

Signed bytes: `SLIM-CHANNEL-DELETE/1`, `audience`, `channel`, `not_after`, `nonce`. The signature is checked against the channel's owner, then against each operator. As for a create request, `audience` must name this Channel Manager and `not_after` may be at most 24 hours ahead.

### Other verifiers

Deployments embedding the Channel Manager can plug in a different verifier for other formats or key schemes, for example one that resolves an agent's binding certificate to its human owner.

## Owner approval

To ask the owner, the Channel Manager sends an `ApprovalRequest` over SLIM, using SlimRPC, to the owner's callback name: service `channel_manager.proto.v1.ChannelOwnerApproval`, method `RequestApproval`. The owner's endpoint replies with an `ApprovalResponse` carrying either a grant or a denial. Both messages are defined in the Channel Manager's protobuf definitions.

The request comes from the SLIM name `<local-name>-approval`, which the Channel Manager registers with the same credentials as its `local-name`.

An owner endpoint should accept a request only from the Channel Manager, checked by its verified identity rather than by the sender's name or the request's `requester` field. In a SlimRPC handler that is `ctx.session().peer_identity()` (`peer_subject()` in the bindings): the subject of the identity token the Channel Manager's approval app set up the session with. It is set only for sessions using MLS, which SlimRPC calls do.

Because the owner's endpoint connects to SLIM outbound, it needs no inbound ports. An owner who doesn't reply within 60 seconds, or can't be reached, has denied the request. A grant the owner returns is checked exactly like one passed with `--grant-file`.

## Expiry

A channel created with `--ttl-seconds` is deleted, along with its sessions' MLS state, within about 30 seconds after its time to live passes. `list-channels` shows when each channel expires.

## Restarts and replicas

With [persistence](./config.md#persistence) configured, owners, expiry times and already-used grants survive a restart along with the channels. Run a single Channel Manager replica per state directory: replicas don't share this state.

# Channel Ownership, Grants and Expiry

Channels created through the Channel Manager API can have an owner who controls who joins, and a time to live after which they are deleted. None of this applies to channels declared in the config file (config mode): those have no owner and no expiry, and write operations are refused in config mode, as before.

## Owners

When the Channel Manager's API server requires authentication (JWT, OIDC or SPIRE), whoever creates a channel becomes its owner, identified by the subject their credential verifies: the `sub` claim, or the SPIFFE ID for SPIRE. Without API authentication there is no caller to identify, so channels have no owner and anyone who can reach the API can change them.

`slimctl channel-manager list-channels` shows each channel's owner.

## Changing participants on an owned channel

The owner adds and removes participants directly. Anyone else needs the owner's authorization, in one of two forms:

- **A grant**: a statement signed by the owner authorizing one specific change, passed with `--grant-file`.
- **Approval on request**: if the request carries no grant and the owner gave `--owner-callback-name` when creating the channel, the Channel Manager asks the owner, as described below.

A grant must name the channel, the participant and the action (`add` or `delete`) of the request it is used for, must not be expired, and is accepted only once. On a channel with no owner, participants are added and removed without a grant.

## Grant format

The default verifier expects the owner's subject to be a `did:key` encoding an Ed25519 public key, and a grant to be this JSON document:

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

- `not_after` is in Unix seconds. `nonce` must be unique per grant. `role` is carried but not enforced.
- `signature` is the standard, padded base64 encoding of an Ed25519 signature over the UTF-8 bytes of the literal `SLIM-CHANNEL-GRANT/1`, then `channel`, `invitee`, `action`, `role`, `not_after` (in decimal) and `nonce`, in that order, joined with a NUL byte. There is no trailing NUL.

Deployments embedding the Channel Manager can plug in a different verifier for other grant formats or key schemes, for example one that resolves an agent's binding certificate to its human owner.

## Owner approval

To ask the owner, the Channel Manager sends an `ApprovalRequest` over SLIM, using SlimRPC, to the owner's callback name: service `channel_manager.proto.v1.ChannelOwnerApproval`, method `RequestApproval`. The owner's endpoint replies with an `ApprovalResponse` carrying either a grant or a denial. Both messages are defined in the Channel Manager's protobuf definitions.

Because the owner's endpoint connects to SLIM outbound, it needs no inbound ports. An owner who doesn't reply within 60 seconds, or can't be reached, has denied the request. A grant the owner returns is checked exactly like one passed with `--grant-file`.

## Expiry

A channel created with `--ttl-seconds` is deleted, along with its sessions' MLS state, within about 30 seconds after its time to live passes. `list-channels` shows when each channel expires.

## Restarts and replicas

With [persistence](./config.md#persistence) configured, owners, expiry times and already-used grants survive a restart along with the channels. Run a single Channel Manager replica per state directory: replicas don't share this state.

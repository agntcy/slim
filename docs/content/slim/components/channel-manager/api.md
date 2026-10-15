# API Reference

The Channel Manager exposes one gRPC service, `channel_manager.proto.v1.ChannelManagerService`, and is the client of one SLIM-based contract, `ChannelOwnerApproval`. Both are defined in `crates/proto/proto/channel-manager/v1/commands.proto`.

## Conventions

**Endpoint.** The gRPC service listens wherever [`api-server.endpoint`](./config.md#api-server) says, with the TLS and client-certificate settings configured there. `slimctl` and the examples throughout these pages use `127.0.0.1:10356`, which is also `slimctl`'s default.

**Errors are in the response body, not the gRPC status.** Every RPC returns `OK` with a response whose `success` is `false` and whose `error_msg` carries the reason. A non-`OK` gRPC status means the call never reached the handler: the connection, TLS, or the API server's own authentication rejected it. Clients must check `success`, not just the status.

```python
response = stub.AddParticipant(request)
if not response.success:
    raise RuntimeError(response.error_msg)
```

**Names.** Channel, participant and callback names are SLIM names with exactly three components, `org/namespace/name`. A name with any other shape is refused with `invalid channel name: ...`, `invalid participant name: ...` or `invalid owner callback name: ...`.

**Signed material is opaque bytes.** `owner_proof` and `grant` are `bytes` on the wire, because their format belongs to the verifier the Channel Manager was built with. With the default verifier they are the UTF-8 bytes of the JSON documents described in [Concepts and Ownership](./ownership.md#signed-formats). A deployment that plugs in another verifier changes what these bytes mean and nothing else about the API.

**Config mode.** In [config mode](./ownership.md#config-mode-and-api-mode) the four mutating RPCs all fail with `channel manager is running in config mode; modify the config file and restart to change channels or participants`. Only the two list RPCs are served.

## Generating client stubs

The protos are published as the buf module [`buf.build/agntcy/slim`](https://buf.build/agntcy/slim), or can be compiled from the repository:

```bash
protoc --python_out=. --pyi_out=. \
  -I crates/proto/proto \
  crates/proto/proto/channel-manager/v1/commands.proto
```

```bash
protoc --go_out=. --go_opt=paths=source_relative \
  --go-grpc_out=. --go-grpc_opt=paths=source_relative \
  -I crates/proto/proto \
  crates/proto/proto/channel-manager/v1/commands.proto
```

Only the message and gRPC plugins are needed. `ChannelOwnerApproval` is deliberately not declared as a `service`, so no stub is generated for it; see [the approval contract](#the-channelownerapproval-contract).

## ChannelManagerService

| RPC | Request | Response | Mutating |
|---|---|---|---|
| `CreateChannel` | `CreateChannelRequest` | `CommandResponse` | yes |
| `DeleteChannel` | `DeleteChannelRequest` | `CommandResponse` | yes |
| `AddParticipant` | `AddParticipantRequest` | `CommandResponse` | yes |
| `DeleteParticipant` | `DeleteParticipantRequest` | `CommandResponse` | yes |
| `ListChannels` | `ListChannelsRequest` | `ListChannelsResponse` | no |
| `ListParticipants` | `ListParticipantsRequest` | `ListParticipantsResponse` | no |

### CreateChannel

Creates a group channel with the Channel Manager as its moderator, and records its owner and expiry.

`CreateChannelRequest`:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel's SLIM name, `org/namespace/channel`. |
| `mls_enabled` | `bool` | Enable MLS end-to-end encryption for the channel. Set `true` unless end-to-end encryption is genuinely not wanted: without it, membership changes rotate no keys. |
| `owner_callback_name` | `optional string` | SLIM name at which the owner can be asked to approve participant changes requested without a grant. Without one, such requests are refused outright. Requires `owner_proof`: a channel with no owner has no one to ask. |
| `ttl_seconds` | `optional uint64` | Seconds until the channel is deleted automatically, with its participants' membership and MLS state. Must be positive. Unset means the channel lives until it is deleted explicitly. |
| `owner_proof` | `optional bytes` | The owner's signature over this request. Its signer becomes the channel's owner, and it must cover every field above. Unset means the channel has no owner — unless [`require-signatures`](./config.md#require-signatures) is set, when the request is refused. |

The signed create request must name this Channel Manager's [`audience`](./config.md#audience), must describe exactly the settings being requested, must not have expired, must not be valid for more than 24 hours, and must not have been used before.

Order of checks: the channel's existence and the request's shape are checked first, then the signature, then the channel is created in SLIM. A refused request leaves nothing behind.

Errors:

| `error_msg` | Cause |
|---|---|
| `channel <name> already exists` | A channel of that name is already moderated here. Also returned when two concurrent creates race. |
| `ttl-seconds must be positive` | `ttl_seconds` was `0`. Omit the field instead. |
| `invalid channel name: ...` | `channel_name` is not a three-component SLIM name. |
| `invalid owner callback name: ...` | `owner_callback_name` is not a three-component SLIM name. |
| `this channel manager requires a create request signed by the channel's owner` | No `owner_proof`, and `require-signatures` is set. |
| `owner-callback-name needs a signed create request: a channel with no owner has no one to ask` | `owner_callback_name` without `owner_proof`. |
| `invalid signed create request: unsupported owner identifier format: ...` | The `owner` in the proof is not a `did:key` encoding an Ed25519 public key. |
| `invalid signed create request: malformed: ...` | The proof is not the expected JSON, carries a field containing a NUL byte, has an empty `owner_callback_name` instead of omitting it, or has an undecodable signature. |
| `invalid signed create request: the signature does not verify against the signer's key` | The signature does not match the canonical bytes, or the proof names an `owner` other than its signer. |
| `signed create request does not match this request` | The proof's `channel`, `mls_enabled`, `ttl_seconds` or `owner_callback_name` differ from the request's. |
| `signed create request is for "x", not this channel manager ("y")` | The proof's `audience` is not this Channel Manager's `audience`. |
| `signed create request is valid for too long: not_after may be at most 24 hours ahead` | `not_after` is more than 24 hours in the future. |
| `signed create request has expired` | `not_after` has passed. |
| `signed create request has already been used` | The proof's nonce was spent on this channel before. |
| `could not record the signed create request's use: ...` | The nonce could not be written to the state store. The request is refused rather than accepted un-recorded. |
| `failed to create channel <name>` | The SLIM session could not be created. The underlying error is logged at `ERROR`. |
| `failed to record state of channel <name>` | The owner or expiry could not be persisted; the channel was rolled back rather than left ownerless. |

### DeleteChannel

Deletes a channel and its SLIM session, and forgets its owner and expiry records.

`DeleteChannelRequest`:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel to delete. |
| `owner_proof` | `optional bytes` | A delete request signed by the channel's owner, or by an [operator](./config.md#operators). Required when the channel has an owner, and when `require-signatures` is set. |

The proof is verified against the channel's owner first, then against each configured operator, so a failure is reported against the owner where there is one. It must name this Channel Manager's `audience` and this channel, must not have expired, must be valid for at most 24 hours, and must not have been used before.

Errors:

| `error_msg` | Cause |
|---|---|
| `channel <name> can only be deleted with a request signed by its owner` | The channel has an owner and no `owner_proof` was sent. |
| `channel <name> has no owner: only a delete request signed by an operator can delete it` | `require-signatures` is set, the channel has no owner, and no `owner_proof` was sent. |
| `channel <name> has no owner and this channel manager has no operators, so it can't be deleted` | `require-signatures` is set, the channel has no owner, an `owner_proof` *was* sent, and no operators are configured, so there is no identity to verify it against. The channel cannot be deleted until one is configured. |
| `invalid signed delete request: ...` | The proof verified against neither the owner nor any operator. The reason given is the first failure — the owner's when the channel has one, otherwise the first operator's — in the same three forms as for `CreateChannel`. |
| `signed delete request is for a different channel` | The proof's `channel` is not `channel_name`. |
| `signed delete request is for "x", not this channel manager ("y")` | Wrong `audience`. |
| `signed delete request is valid for too long: not_after may be at most 24 hours ahead` | `not_after` more than 24 hours ahead. |
| `signed delete request has expired` | `not_after` has passed. |
| `signed delete request has already been used` | The proof's nonce was spent on this channel before. |
| `could not record the signed delete request's use: ...` | The nonce could not be persisted. |

A channel that does not exist is reported through the session layer's own error, which is returned verbatim.

### AddParticipant and DeleteParticipant

Invites a participant into a channel, or removes one, performing the full SLIM group state update and MLS key rotation.

`AddParticipantRequest` and `DeleteParticipantRequest` have the same fields:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel to change. |
| `participant_name` | `string` | The SLIM application being added or removed. |
| `grant` | `optional bytes` | A grant from the channel's owner authorizing this exact change. Required when the channel has an owner, the owner's own changes included. Unset means no grant is presented: the owner is asked for one if the channel has a callback name. |

The grant must verify against the owner's key, must name this channel, this participant and this action (`add` for `AddParticipant`, `delete` for `DeleteParticipant`), must not have expired, and must not have been used before. Grants are not capped at 24 hours: an owner hands them out for later use.

The channel's existence is checked before authorization, and the participant's name is parsed after it, so an unauthorized request never reveals whether a name is well formed.

Errors:

| `error_msg` | Cause |
|---|---|
| `channel <name> not found` | No such channel here. |
| `channel <name> has no owner to sign for changes, and this channel manager refuses unsigned ones` | `require-signatures` is set and the channel has no owner. Such a channel can only be deleted, by an operator. |
| `channel <name> requires a grant from its owner` | No grant presented, and the channel has no `owner_callback_name`, so the owner cannot be asked. |
| `channel owner did not answer the approval request in time` | The owner did not answer within 60 seconds. |
| `could not ask the channel owner for approval: invalid owner callback name: ...` | The recorded callback name is not a valid SLIM name. |
| `could not ask the channel owner for approval: approval call failed: ...` | The owner's endpoint could not be reached, or its handler failed. |
| `could not ask the channel owner for approval: malformed approval response: ...` | The answer did not decode as an `ApprovalResponse`, or carried no decision. |
| `channel owner denied the request: <reason>` | The owner declined, with their reason. |
| `invalid grant: unsupported owner identifier format: ...` | The channel's owner is not a `did:key` the verifier understands — for example a channel [owned by a login identity](./ownership.md#channels-owned-by-a-login-identity). |
| `invalid grant: malformed: ...` | The grant is not the expected JSON, has an unknown `action`, or contains a NUL byte in a field. |
| `invalid grant: the signature does not verify against the signer's key` | The grant was not signed by this channel's owner, or was tampered with. |
| `grant does not authorize this change` | The grant verified, but its `channel`, `invitee` or `action` is not the one being requested. |
| `grant has expired` | The grant's `not_after` has passed. |
| `grant has already been used` | The grant's nonce was spent on this channel before. |
| `could not record the grant's use: ...` | The nonce could not be persisted. |
| `invalid participant name: ...` | `participant_name` is not a three-component SLIM name. |
| `failed to invite participant <p> to channel <c>: ...` | The SLIM invitation or its key rotation failed — commonly an offline participant. |
| `failed to remove participant <p> from channel <c>: ...` | The SLIM removal failed. |

### ListChannels

`ListChannelsRequest` has no fields.

`ListChannelsResponse`:

| Field | Type | Semantics |
|---|---|---|
| `success` | `bool` | Always `true`. |
| `error_msg` | `optional string` | Unset. |
| `channel_name` | `repeated string` | Deprecated: channel names only, kept for existing callers. |
| `channels` | `repeated ChannelInfo` | Each channel with its owner and expiry. |

`ChannelInfo`:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel's SLIM name. |
| `owner` | `optional string` | The owner's identifier, a `did:key` with the default verifier. Unset for a channel with no owner, including every config-mode channel. |
| `expires_at` | `optional uint64` | Unix seconds at which the channel expires. Unset if it has no time to live. An expired channel may still be listed until the reaper's next pass. |

New clients should read `channels`; `channel_name` carries the same names for clients written before `ChannelInfo` existed.

### ListParticipants

`ListParticipantsRequest`:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel to inspect. |

`ListParticipantsResponse`:

| Field | Type | Semantics |
|---|---|---|
| `success` | `bool` | `false` when the channel is unknown or its roster could not be read. |
| `error_msg` | `optional string` | `channel <name> not found`, or `failed to list participants for channel <name>: ...`. |
| `participant_name` | `repeated string` | The channel's current members, the Channel Manager itself included. |

### CommandResponse

Returned by the four mutating RPCs.

| Field | Type | Semantics |
|---|---|---|
| `success` | `bool` | Whether the change was made. |
| `error_msg` | `optional string` | Why it was not. Set exactly when `success` is `false`. |

## The ChannelOwnerApproval contract

When a participant change is requested with no grant and the channel has an `owner_callback_name`, the Channel Manager calls the owner over SLIM — not gRPC — using SlimRPC:

| | |
|---|---|
| Service | `channel_manager.proto.v1.ChannelOwnerApproval` |
| Method | `RequestApproval` |
| Request | `ApprovalRequest` |
| Response | `ApprovalResponse` |
| Called on | the channel's `owner_callback_name` |
| Called from | the SLIM name `<local-name>-approval` |
| Deadline | 60 seconds, after which the change is denied |

The Channel Manager is always the client of this contract and never its server, which is why the proto declares no `service` for it: there is no generated stub, and an owner's endpoint registers a handler for the two names above directly.

`ApprovalRequest`:

| Field | Type | Semantics |
|---|---|---|
| `channel_name` | `string` | The channel the change is for. |
| `participant_name` | `string` | The participant being added or removed. |
| `action` | `ParticipantAction` | `PARTICIPANT_ACTION_ADD` or `PARTICIPANT_ACTION_DELETE`. |
| `requester` | `optional string` | The verified identity of whoever asked, if the Channel Manager has one: a token's `sub` or a client certificate's SPIFFE ID. Unset when the caller presented neither. |

`ApprovalResponse` carries a `decision` oneof:

| Field | Type | Semantics |
|---|---|---|
| `grant` | `bytes` | A grant signed by the owner authorizing exactly the requested change. Verified exactly like a grant presented on the original request. |
| `denied` | `string` | The owner declined, with a human-readable reason, which is returned to the requester. |

An empty `decision` is a malformed response, and denies the change.

!!! warning "`requester` is a claim, not an authorization"
    `requester` tells the owner who the Channel Manager believes asked. It is only as trustworthy as the Channel Manager itself, and an owner's endpoint must not use it to decide who may talk to it. Authenticate the caller instead: `ctx.peer_subject()` returns the verified subject of the session's peer, in Rust and in the bindings alike. See [Trusting the Channel Manager](./security.md#trusting-the-channel-manager).

### Implementing an owner endpoint

The owner's endpoint is a SLIM application that connects outbound, so it needs no inbound ports. It must be created with `Direction::Bidirectional` — answering a call means both receiving and sending — and it registers one unary handler:

```rust
use agntcy_slim_channel_manager::approval::{APPROVAL_METHOD, APPROVAL_SERVICE};
use agntcy_slim_channel_manager::proto::approval_response::Decision;
use agntcy_slim_channel_manager::proto::{ApprovalRequest, ApprovalResponse, ParticipantAction};
use prost::Message;
use slim_rpc::{Context, RpcError};

server.register_unary_unary(
    APPROVAL_SERVICE,
    APPROVAL_METHOD,
    move |bytes: Vec<u8>, ctx: Context| {
        let owner = owner.clone();
        async move {
            // Only the channel manager may ask, as verified -- never the
            // `requester` field, which the request only claims.
            if ctx.peer_subject().as_deref() != Some(CHANNEL_MANAGER_DID_KEY) {
                return Err(RpcError::permission_denied("not the channel manager"));
            }

            let request = ApprovalRequest::decode(bytes.as_slice())
                .map_err(|e| RpcError::invalid_argument(e.to_string()))?;
            let action = match request.action() {
                ParticipantAction::Add => "add",
                ParticipantAction::Delete => "delete",
                ParticipantAction::Unspecified => {
                    return Err(RpcError::invalid_argument("no action"));
                }
            };

            let decision = if owner.allows(&request) {
                Decision::Grant(owner.grant(&request.channel_name, &request.participant_name, action))
            } else {
                Decision::Denied("not on the guest list".to_string())
            };
            Ok(ApprovalResponse { decision: Some(decision) }.encode_to_vec())
        }
    },
);
```

In the Python and Go bindings the same registration takes a handler object rather than a closure, because `register_unary_unary` crosses the FFI boundary: subclass `UnaryUnaryHandler` and implement `handle(request: bytes, context) -> bytes`. The [walkthrough](./install.md#step-5-answer-approval-requests-as-the-owner) has a complete endpoint in each language.

## Related

- [Installation and Walkthrough](./install.md) — Every call above, in order, with working code
- [Concepts and Ownership](./ownership.md#signed-formats) — The bytes `owner_proof` and `grant` carry
- [Security Model](./security.md) — What authorizes each call
- [Operations and Troubleshooting](./operations.md#troubleshooting) — Every error message, with its fix

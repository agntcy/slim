# Security Model

The Channel Manager separates two questions that are easy to conflate:

- **Who is calling?** Answered by authentication — a token, a client certificate, a SLIM identity. The answer is recorded and reported, and it decides nothing about a channel.
- **What authorizes this change?** Answered by a signature from the channel's owner, or from an operator. The answer is the only thing that lets a change through.

Keeping them apart is what makes a channel's owner meaningful. An identity that opened a connection cannot sign anything later, cannot be delegated, and cannot be verified by a third party. An owner's key can do all three.

## What each authentication option identifies

The Channel Manager authenticates in three independent places. They answer different questions and are configured separately.

| Where | Configured by | What it identifies | What it is used for |
|---|---|---|---|
| The gRPC API server | [`api-server.tls`](./config.md#api-server), plus the server's auth middleware | The caller of an RPC: a verified token's `sub`, or the SPIFFE ID in a verified client certificate | Logged with every request; passed to owners as the `requester` of an approval |
| The SLIM connection | [`auth`](./config.md#auth) | The Channel Manager's own SLIM application identity | Admission to the data plane and to the groups it moderates; peers that keep an allow-list match on it |
| The owner approval app | [`approval-auth`](./config.md#approval-auth), falling back to `auth` | The `<local-name>-approval` application that asks owners for approval | How an owner's endpoint recognizes the Channel Manager |

The caller's identity is read from the token claims first and from the client certificate only if there are none: a token names the caller, while a certificate may identify no more than the workload carrying the call. When neither is present, the caller has no identity — the call is still served, and the approval request the owner sees simply carries no `requester`.

!!! warning "A caller's identity is never ownership"
    A channel's owner is whoever signed its create request, and nothing else. A caller who authenticates as `spiffe://example.org/ns/default/sa/orchestrator` and creates a channel without a signed request has created a channel with **no owner**, not a channel they own.

## What authorizes a change

Authorization depends on whether the channel has an owner, and on whether [`require-signatures`](#require-signatures) is set.

| Channel | Create | Add or remove a participant | Delete |
|---|---|---|---|
| Owned | — | the owner's grant, presented or obtained by approval | the owner's or an operator's signed delete request |
| No owner | nothing | nothing | nothing |
| No owner, `require-signatures` | refused | refused | an operator's signed delete request only |
| Owned by a login identity | — | impossible | an operator's signed delete request only |
| Config mode | refused | refused | refused |

Two properties are worth stating explicitly, because they are easy to assume the other way round:

**The owner is not exempt.** An owner changing their own channel presents a grant like anyone else. The Channel Manager cannot recognize the owner on a connection, so there is nothing to exempt.

**An operator is not a super-owner.** An operator's authority is exactly one verb: delete. They cannot add a participant, cannot remove one, and cannot take ownership of anything.

## Replay protection

Every signed statement is a bearer credential: whoever holds the bytes can use them. Four mechanisms bound what a captured one is worth.

### Single use

Each statement carries a `nonce`. When a statement is accepted, its nonce is recorded against the channel it was used on, and a second use is refused with `... has already been used`. Nonces are scoped per channel, so an owner choosing a nonce cannot burn a grant for a channel they do not own, and the record survives channel deletion: a channel recreated under the same name still refuses grants already spent against it.

The nonce is recorded last, after every other check has passed, so a statement rejected for any other reason is not burned. With [persistence](./config.md#persistence) configured, the nonce is made durable *before* the statement counts as accepted; if it cannot be written, the statement is refused rather than accepted in a way that would be replayable after a restart.

### Expiry

Every statement carries `not_after`, in Unix seconds, after which it is refused with `... has expired`. Nonces are remembered only until their statement's `not_after` has passed — after that the statement is refused as expired anyway, so the record can be dropped.

Sign for the shortest window the workflow tolerates. `slimctl` signs for five minutes, since it sends immediately. A grant an owner issues for later use can be longer-lived, which is the point of grants.

### The 24-hour cap on signed requests

A signed create or delete request may not have a `not_after` more than 24 hours ahead; one that does is refused with `... is valid for too long: not_after may be at most 24 hours ahead`. This bounds how long a leaked but unused request stays dangerous, and bounds how long its nonce has to be remembered.

Grants are deliberately **not** capped. An owner hands a grant out so it can be used later, possibly by someone who is not online yet, and the cap would defeat that. A grant's blast radius is one change to one channel, which is why the looser bound is acceptable.

### Audience

A signed create or delete request names, in `audience`, the Channel Manager it is meant for. A request signed for one is refused by another with `... is for "x", not this channel manager ("y")`.

`audience` defaults to `local-name`. Set it explicitly when several deployments would otherwise share a name — development and production both running `agntcy/ns/channel-manager`, say — so that a request captured in one cannot be replayed against the other:

```yaml
channel-manager:
  local-name: "agntcy/ns/channel-manager"
  audience: "agntcy/prod/channel-manager"
```

Grants carry no audience. They are already bound to a channel, a participant and an action, and a channel name is unique to the Channel Manager that moderates it.

### Domain separation

Each kind of statement is signed over a distinct tag — `SLIM-CHANNEL-GRANT/1`, `SLIM-CHANNEL-CREATE/1`, `SLIM-CHANNEL-DELETE/1` — followed by its fields joined with NUL bytes. A signature made for one kind therefore cannot verify as another, even with the same key, channel and nonce. Any field containing a NUL byte is refused, so the field boundaries cannot be shifted to make one statement's bytes read as another's.

## Channels frozen by a login identity

Earlier releases recorded the creator's authenticated identity as the channel's owner. Such a channel still has that owner on record — an OIDC subject or a SPIFFE ID — but no signature can be verified against it, because it is not a key. Every participant change on it is therefore refused: with `invalid grant: unsupported owner identifier format: ...` when a grant is presented, and with `channel <name> requires a grant from its owner` when none is.

This is a deliberate dead end rather than a fallback to unsigned changes: the alternative would be to treat "the owner cannot sign" as "anyone may change it". The channel still expires if it was created with a time to live, and an [operator](#operators) can delete it. Recreate it with a signed create request to get a key-owned channel.

## Operators

An operator is a `did:key` in [`operators`](./config.md#operators). Their signed delete request deletes any channel, whatever its owner.

Operators exist because owners' keys get lost, people leave, and channels outlive the applications that created them. Without them, a channel whose owner's key is gone could never be deleted — and `require-signatures` would make that permanent.

Treat an operator key as infrastructure, not as a user credential:

- Hold it where the Channel Manager's own key material is held, with the same approvals around its use. It is a standing authority to delete any channel.
- Keep the list short, and keep it reviewable: every operator is logged at startup as `Operator: may delete any channel`.
- Rotating one is a config change plus a restart. Drop the old `did:key`, add the new one.
- Never put an operator's key on the machine that runs the Channel Manager "for convenience". A signed delete request is only as safe as the separation between the key and the service that honours it.

## require-signatures

With [`require-signatures`](./config.md#require-signatures) off — the default — a client may still create a channel with no signature, and such a channel accepts unsigned changes from anyone who can reach the API. This exists so that clients written before signed requests keep working while they are migrated.

With it on, the Channel Manager:

- refuses a create request without `owner_proof`;
- refuses every participant change on a channel that has no owner, including channels created before the setting was turned on;
- allows only an operator to delete such a channel.

Turn it on once every client signs its creates. Until then, `list-channels` shows which channels have no owner — those are the ones that will stop being changeable. Turning it on does not retroactively give anything an owner; it only stops the Channel Manager from honouring changes nobody signed for.

## Trust each side needs

The model is deliberately light on registries. With the default verifier, an owner identifier is a `did:key`, which carries its own public key, so **the Channel Manager needs no key store and no lookup to verify an owner's signature**. It trusts a channel's owner because that owner's key created the channel, not because it was told to.

What does need configuring is the SLIM-level trust between the Channel Manager and the applications it talks to.

### Its own SLIM identity

With a `jwt` [`auth`](./config.md#auth), the Channel Manager signs EdDSA tokens with an Ed25519 key and accepts peers whose tokens verify against the JWKS in `trusted_keys`. That file must include the Channel Manager's own public key — MLS checks its own token against it — and it refuses to start otherwise. Peers that keep an allow-list need the Channel Manager's `did:key`, which it logs at startup as `Channel manager identity`.

With `spire`, both sides' identities come from the SPIFFE trust domain and there is no file to maintain. With `shared_secret`, every holder of the secret can be any identity: use it only in development.

### Trusting the Channel Manager

An owner's endpoint must accept approval requests only from the Channel Manager. Check that against the **verified identity of the SLIM session's peer**, not against the sender's name and not against the request's `requester` field:

```rust
if ctx.peer_subject().as_deref() != Some(CHANNEL_MANAGER_DID_KEY) {
    return Err(RpcError::permission_denied("not the channel manager"));
}
```

The peer identity is the subject of the identity token the Channel Manager's approval app set up the session with. It is populated only for MLS sessions, which SlimRPC calls are.

What that subject looks like depends on how the approval app authenticates, and this decides what an owner can usefully pin:

| `approval-auth` | `peer_subject()` is | Pin it? |
|---|---|---|
| `jwt` | the approval key's `did:key`, exactly as logged at startup under `Owner approval identity` | Yes — stable and specific |
| `spire` | the approval app's SPIFFE ID | Yes |
| `shared_secret` | the app's shared-secret id plus a per-instance suffix | No — the suffix changes per instance |

This is the reason to give the approval app a `jwt` identity of its own even when the Channel Manager connects to SLIM as a SPIRE workload: owner endpoints get one stable `did:key` to pin, and the Channel Manager keeps its workload identity for everything else.

### Trusting an owner's endpoint

The call goes the other way too: the Channel Manager's approval app must be able to establish a session with the owner's endpoint. With a `jwt` `approval-auth`, the endpoint's public key has to be in the approval app's `trusted_keys`, alongside its own.

## What the model does not protect against

Being explicit about the edges of the model:

**The Channel Manager is inside the channel's confidentiality boundary.** It is a member of each MLS group it moderates, which is what lets it rotate keys on a membership change. MLS protects channel traffic from the SLIM nodes, from the network, and from former and future members — not from the moderator. Run the Channel Manager at the trust level of the channels it moderates.

**A compromised Channel Manager cannot forge authority, but it can abuse the authority it has.** It holds no owner's key, so it cannot produce a grant, a create request or a delete request that any owner would recognize. It can refuse legitimate changes, delay them, report a false `requester` to an owner, and — as the moderator — change membership through SLIM directly. Signatures bound what it can claim, not what a compromised moderator can do.

**Signed statements are bearer credentials in transit.** Anyone who sees a grant before it is spent can spend it. Nonces and `not_after` bound the window; TLS on the API server, and a short `not_after`, close it. Do not log signed statements.

**Nothing is a rate limit.** Replay protection rejects reuse, not volume. Put the API server behind a network boundary and authenticate its callers, and bind `api-server.endpoint` no wider than it has to be.

**Ownership cannot be transferred.** There is no handover operation. A channel's owner is fixed when it is created; to change it, delete the channel and create it again.

## Related

- [Concepts and Ownership](./ownership.md) — The model these mechanisms implement, and the exact signed formats
- [API Reference](./api.md) — Which error each check produces
- [Deployment Guide](./deploy.md) — Choosing identities, `audience` and persistence for a real deployment
- [Authentication](../../architecture/authentication.md) — TLS, mTLS, JWT and SPIRE across SLIM

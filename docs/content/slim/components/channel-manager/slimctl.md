# Managing Channels with slimctl

Applications that hold a channel owner's key — SHADI, for instance — call the Channel Manager's gRPC API directly. Where no such application exists, an administrator does the same work with `slimctl channel-manager` and an Ed25519 key. This page is for that administrator.

The one rule to internalize: **`slimctl` never changes a channel on its own authority.** It has no identity the Channel Manager would honour. Every change it sends is signed — with a key it is given, or signed elsewhere and forwarded. If you have no key and nobody will sign for you, there is nothing `slimctl` can change.

For the exact flags of each subcommand, see the [command reference](../cli/reference/channel-manager/index.md). This page is about how to use them together.

## Get a key

Generate an Ed25519 key and keep it as you would any signing key — whoever holds it controls the channels it owns:

```bash
openssl genpkey -algorithm ed25519 -out admin.pem
chmod 600 admin.pem
```

Its `did:key` is the identifier the Channel Manager records as a channel's owner, and the form in which an operator is configured:

```bash
slimctl channel-manager did-key --key admin.pem
```

```text
did:key:z6MkneqwSGENkjKokLBYkhpY7B99mxrjVSLoECjsTR1z6iPU
```

`did-key` needs no Channel Manager and makes no connection: it reads the key and prints. Use it to fill in an [`operators`](./config.md#operators) list, or to check that a key file is the one you think it is.

## Point slimctl at the Channel Manager

`slimctl channel-manager` connects to `127.0.0.1:10356` by default. Override it per command with `--server`, or once in `~/.slimctl/config.yaml`:

```bash
slimctl channel-manager --server 192.168.1.100:10356 list-channels
```

The other inherited options matter when the API server is not plaintext on localhost: `--tls.ca_file`, `--tls.cert_file` and `--tls.key_file` for TLS and mTLS, `--basic-auth-creds` for basic authentication, and `--timeout` (15 seconds by default). None of these have anything to do with signing: they get the call to the Channel Manager, and the signature decides what it is allowed to do once it arrives.

## Know the audience

Signed create and delete requests name the Channel Manager they are meant for, so one signed for staging cannot be replayed against production. `--audience` supplies it, and `--key` requires it:

```bash
export SLIMCTL_CM_AUDIENCE=agntcy/ns/channel-manager
```

The value is the Channel Manager's [`audience`](./config.md#audience), which is its `local-name` unless configured otherwise, and which it logs at startup:

```text
INFO Signed create and delete requests must name this audience audience=agntcy/ns/channel-manager
```

Get it wrong and the request is refused with `signed create request is for "x", not this channel manager ("y")`. Grants carry no audience, so `add-participant` and `delete-participant` do not take the flag.

## Day-to-day commands

### Create a channel

```bash
slimctl channel-manager create-channel agntcy/team/general \
  --key admin.pem \
  --audience agntcy/ns/channel-manager \
  --ttl-seconds 86400 \
  --owner-callback-name agntcy/owners/alice-desktop
```

```text
Channel agntcy/team/general created, owned by did:key:z6MkneqwSGENkjKokLBYkhpY7B99mxrjVSLoECjsTR1z6iPU
```

The key that signs becomes the owner, so this is the command that decides who controls the channel for the rest of its life — there is no ownership transfer later. MLS is on unless `--disable-mls` is given. `--ttl-seconds` has the channel delete itself; `--owner-callback-name` is where an owner's endpoint can be asked to approve changes nobody holds a grant for, and is only accepted on a signed request.

### Add and remove participants

```bash
slimctl channel-manager add-participant agntcy/team/general agntcy/agents/assistant-1 --key admin.pem
slimctl channel-manager delete-participant agntcy/team/general agntcy/agents/assistant-1 --key admin.pem
```

```text
Participant agntcy/agents/assistant-1 added to channel agntcy/team/general
Participant agntcy/agents/assistant-1 removed from channel agntcy/team/general
```

`--key` here signs a grant, not a request: a statement authorizing this one change to this one channel. It must be the channel's **owner's** key. An operator's key will not do — operators delete channels and nothing else — and `slimctl` cannot tell the difference in advance, so a wrong key fails at the Channel Manager with `invalid grant: the signature does not verify against the signer's key`.

### Inspect

Neither list command signs anything, so both work without a key:

```bash
slimctl channel-manager list-channels
```

```text
Channels (2):
  - agntcy/team/general (owner: did:key:z6Mkhe..., expires: 2026-10-14T08:00:00Z)
  - agntcy/team/broadcast
```

A channel is shown with its owner and expiry when it has them. `agntcy/team/broadcast` above has neither: nobody signed its creation, so anyone who can reach the API can change it — and once [`require-signatures`](./security.md#require-signatures) is on, nobody can, except an operator deleting it. `list-channels` is how you find those channels before turning that setting on.

```bash
slimctl channel-manager list-participants agntcy/team/general
```

```text
Participants in channel agntcy/team/general (2):
  - agntcy/ns/channel-manager
  - agntcy/agents/assistant-1
```

The Channel Manager appears in every roster: it is the channel's moderator, and so a member of the group.

### Delete a channel

```bash
slimctl channel-manager delete-channel agntcy/team/general \
  --key admin.pem \
  --audience agntcy/ns/channel-manager
```

```text
Channel agntcy/team/general deleted
```

The Channel Manager verifies the signature against the channel's owner first and then against each configured operator, so the same command serves both roles. This is what an operator uses to clear away a channel whose owner's key is gone.

## Forwarding statements signed elsewhere

`--key` means the key is on the machine running `slimctl`. Often it should not be: it belongs to a person who approves changes rather than to whoever runs the command, or it lives in an HSM, or the approval is a ticket someone signs off.

For those cases `slimctl` forwards bytes it did not produce. The statement is sent exactly as given — `slimctl` does not parse it, so a malformed one is refused by the Channel Manager rather than by `slimctl`:

```bash
# A grant the channel's owner signed and sent you
slimctl channel-manager add-participant \
  agntcy/team/general agntcy/agents/assistant-2 \
  --grant-file grant.json

# A delete request signed on the machine that holds the key
sign-delete-request agntcy/team/general \
  | slimctl channel-manager delete-channel agntcy/team/general --signed-request-file -
```

`-` reads standard input, so a signing step can be piped straight in without a temporary file holding a credential. `--signed-request-file` covers `create-channel` and `delete-channel`; `--grant-file` covers `add-participant` and `delete-participant`. Each is mutually exclusive with `--key`.

Note what the two modes mean for `create-channel`: with `--key`, the channel's owner is the key `slimctl` signed with, and it reports that owner back to you. With `--signed-request-file`, the owner is whoever signed the file, and `slimctl` only reports `Channel agntcy/team/general created` — it never saw a key, so it cannot name them. Run `list-channels` to see who the owner is.

The [walkthrough](./install.md#step-3-create-an-owned-channel) has working code for producing these files in Rust, Python and Go.

## Statements signed here are short-lived

A statement `slimctl` signs is valid for five minutes, because it is sent immediately. Two consequences:

- **Clock skew matters.** A machine running minutes fast or slow produces statements the Channel Manager refuses as `... has expired`, or as `... is valid for too long` at the far extreme. Keep both ends on NTP.
- **It is not a credential to keep.** Nothing `slimctl` signs is worth storing or reusing. Every statement carries a fresh nonce, and the Channel Manager refuses a second use with `... has already been used` — a re-run of a failed command is a new statement, not a retry of the old one.

Statements signed elsewhere choose their own `not_after`. A grant may be long-lived, which is the point of grants; create and delete requests may not exceed 24 hours.

## Operating as an operator

An operator is a `did:key` in the Channel Manager's [`operators`](./config.md#operators) whose signed delete request deletes any channel. Setting one up:

```bash
openssl genpkey -algorithm ed25519 -out operator.pem
chmod 600 operator.pem
slimctl channel-manager did-key --key operator.pem
```

Put the printed `did:key` in the configuration and restart. It is logged at startup, so the list is auditable from the logs:

```text
INFO Operator: may delete any channel operator=did:key:z6Mk...
```

From then on, `delete-channel --key operator.pem --audience ...` deletes any channel, whatever its owner. An operator cannot add or remove participants, and owns nothing.

Treat the key accordingly: it is a standing authority to delete any channel, so it belongs with infrastructure credentials and not on the machine that runs the Channel Manager. [Operators](./security.md#operators) covers rotation and handling.

## Related

- [Command Reference](../cli/reference/channel-manager/index.md) — Every subcommand and flag
- [Installation and Walkthrough](./install.md) — The same operations with an application's key, in code
- [Security Model](./security.md) — What a signature authorizes, and what it does not
- [Operations and Troubleshooting](./operations.md#troubleshooting) — Every refusal, with its cause

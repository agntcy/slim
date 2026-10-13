# Deployment Guide

This page covers running the Channel Manager as a container and on Kubernetes: which identity to give it, how to persist the state that ownership depends on, why it runs as a single replica, and how to move an existing deployment onto signed requests.

[Installation](./install.md) covers building and running it locally; [Configuration Reference](./config.md) documents every setting.

## Container

The release image is published to the GitHub registry and runs the binary directly:

```bash
docker pull ghcr.io/agntcy/slim/channel-manager:2.0.0
docker run --rm \
  -v "$PWD/config.yaml:/config.yaml:ro" \
  -p 10356:10356 \
  -e CM_PASSPHRASE \
  ghcr.io/agntcy/slim/channel-manager:2.0.0 --config-file /config.yaml
```

The image is built `FROM gcr.io/distroless/base-nossl-debian12:nonroot`, so it has no shell and runs as a non-root user. Its entrypoint is `/channel-manager`, and the only argument it needs is `--config-file`. A `channel-manager-debug` image built on `debian:bookworm-slim` carries a shell and debug symbols, for when something has to be inspected in place.

Two things the container needs from outside:

- **The configuration file**, mounted read-only. Note that the API server binds to `127.0.0.1:10356` in the examples; inside a container that is unreachable from outside it, so set `api-server.endpoint` to `0.0.0.0:10356` and let the network boundary — not the bind address — decide who may reach it.
- **Secrets as environment variables.** Every value in the configuration file resolves `${env:VAR}` and `${file:/path}`, so no shared secret or passphrase has to be written into it.

```yaml
channel-manager:
  api-server:
    endpoint: "0.0.0.0:10356"
  persistence:
    path: /var/lib/slim/channel-manager
    encryption-passphrase: ${env:CM_PASSPHRASE}
```

## Kubernetes

A `slim-channel-manager` Helm chart is maintained alongside the other SLIM charts in [agntcy/slim-staging](https://github.com/agntcy/slim-staging), with the ArgoCD configuration that deploys them. Check that repository for the chart's current values; the shape it renders is:

| Object | Notes |
|---|---|
| `ConfigMap` | The whole `channel-manager` config, from `config.channel-manager` in the values, mounted at `/config.yaml` |
| `Deployment` | `replicaCount` (keep it at `1`), the image, and `--config-file /config.yaml` |
| `Service` | `ClusterIP` on `service.port`, `10356` |
| `PersistentVolumeClaim` | Only when `persistence.enabled`, mounted at `persistence.path.value` |
| `ServiceAccount`, `Ingress` | Optional, as usual |

Because the config is passed through from values verbatim, settings the chart has no dedicated key for — `audience`, `require-signatures`, `operators`, `approval-auth` — go under `config.channel-manager` directly:

```yaml
replicaCount: 1

config:
  channel-manager:
    slim-connection:
      endpoint: "http://slim:46357"
      tls:
        insecure: true
    api-server:
      endpoint: "0.0.0.0:10356"
      tls:
        insecure: true
    local-name: "agntcy/prod/channel-manager"
    audience: "agntcy/prod/channel-manager"
    require-signatures: true
    operators:
      - "did:key:z6Mk..."
    auth:
      type: spire
      socket_path: "unix:///run/spire/sockets/agent.sock"

persistence:
  enabled: true
  encryptionPassphrase:
    secretRef:
      name: channel-manager-passphrase
      key: encryption-passphrase
  deleteSessionsOnShutdown: false
  size: 1Gi
```

The chart's `persistence.encryptionPassphrase` takes the passphrase one of three ways: a literal `value` (testing only), the name of an `envVar` you supply through `extraEnv`, or a `secretRef`, which it injects as `CM_PASSPHRASE` and references from the config as `${env:CM_PASSPHRASE}`. Prefer the last.

Two deployment details to get right, whether you use the chart or your own manifests:

**Use the `Recreate` strategy.** The default `RollingUpdate` starts the new pod before the old one goes away. With a `ReadWriteOnce` volume the new pod cannot mount it, and the rollout stalls; with any volume, two Channel Managers briefly share state they are not designed to share.

```yaml
spec:
  strategy:
    type: Recreate
```

**There is no health endpoint.** The Channel Manager serves no gRPC health service, so probe the API port at the TCP level:

```yaml
readinessProbe:
  tcpSocket:
    port: 10356
livenessProbe:
  tcpSocket:
    port: 10356
  initialDelaySeconds: 10
```

Note what a ready port does and does not tell you: the gRPC server starts only after the SLIM connection is up, the app is registered, persisted sessions are restored, and — in config mode — every configured channel has been created and reconciled. An open port therefore means the Channel Manager finished starting. It does not mean the SLIM connection is still healthy now.

## Identities

The Channel Manager authenticates in up to three places, and they are chosen independently. [Security Model](./security.md#what-each-authentication-option-identifies) explains what each one identifies; this is how to configure them.

### Its SLIM application identity

=== "SPIRE (recommended)"

    The Channel Manager fetches an SVID from the SPIRE agent's Workload API socket, which has to be mounted into the pod. No key material is configured, and nothing has to be rotated by hand.

    ```yaml
    auth:
      type: spire
      socket_path: "unix:///run/spire/sockets/agent.sock"
      # jwt_audiences: ["slim"]
      # trust_domains: []
    ```

    Deploy SPIRE first; the [Kubernetes deployment guide](../../deploy/kubernetes.md) covers the `slim-spire` chart and the workload registration entries.

=== "JWT (did:key)"

    The Channel Manager signs EdDSA tokens with an Ed25519 key and accepts peers whose tokens verify against a JWKS. Use it where there is no SPIFFE infrastructure but peers keep allow-lists.

    ```yaml
    auth:
      type: jwt
      private_key:
        file: "/etc/channel-manager/cm-ed25519.pem"
      trusted_keys:
        file: "/etc/channel-manager/members.jwks.json"
    ```

    `trusted_keys` must contain the Channel Manager's own public key as well as its peers' — MLS verifies its own token against it, and it refuses to start otherwise. Its `did:key` is logged at startup as `Channel manager identity`; that is the value peers put in their allow-lists.

=== "Shared secret (development)"

    Every holder of the secret can present any identity, so this is for development and closed test networks only.

    ```yaml
    auth:
      type: shared_secret
      secret: ${env:SLIM_SHARED_SECRET}
    ```

### Its owner approval identity

The application that asks owners for approval, `<local-name>-approval`, uses `auth` unless [`approval-auth`](./config.md#approval-auth) gives it one of its own. Give it a `jwt` identity in production, even when `auth` is SPIRE:

```yaml
approval-auth:
  type: jwt
  private_key:
    file: "/etc/channel-manager/cm-approval-ed25519.pem"
  trusted_keys:
    file: "/etc/channel-manager/owners.jwks.json"
```

This is the one identity owner endpoints see, and a `jwt` identity makes it a stable `did:key` they can pin — logged at startup as `Owner approval identity`. With a shared secret it carries a per-instance suffix and cannot be pinned at all. `trusted_keys` here needs the owner endpoints' public keys, and its own.

### The API server's callers

`api-server.tls` decides how callers reach the API and how they are identified. For anything beyond a localhost socket, serve TLS and require client certificates:

```yaml
api-server:
  endpoint: "0.0.0.0:10356"
  tls:
    insecure: false
    source:
      type: file
      cert: "/etc/channel-manager/tls/server.pem"
      key: "/etc/channel-manager/tls/server-key.pem"
    client_ca:
      type: file
      path: "/etc/channel-manager/tls/client-ca.pem"
```

A client certificate's SPIFFE ID then identifies the caller, which is logged with every request and reported to owners as the `requester` of an approval. It authorizes nothing: a caller still needs the channel owner's signature.

## Choosing an audience

[`audience`](./config.md#audience) is what signed create and delete requests must name, and it defaults to `local-name`. The default is right when each deployment has a distinct `local-name`. Set it explicitly when they do not — when development, staging and production all run `agntcy/ns/channel-manager`, a request signed for one would otherwise be accepted by the others:

```yaml
local-name: "agntcy/ns/channel-manager"
audience: "agntcy/prod/channel-manager"
```

Choose it before clients start signing: changing it invalidates every statement signed for the old value. It is logged at startup, and is the value administrators pass as `--audience` or set in `SLIMCTL_CM_AUDIENCE`.

## Persistence

Without a `persistence` section everything is in memory: channels, their owners, their expiry times and the nonces of spent grants all disappear on restart. That is fine for a Channel Manager whose channels are recreated by whatever drives it, and wrong for one whose channels outlive a pod rolling.

```yaml
persistence:
  path: /var/lib/slim/channel-manager
  encryption-passphrase: ${env:CM_PASSPHRASE}
  delete-sessions-on-shutdown: false
```

With it configured, two encrypted SQLite databases live in `path` under the same passphrase: the session layer's, holding sessions and MLS group state, and the Channel Manager's own, holding owners, expiry times and spent nonces. On restart it restores the sessions, then keeps owners and expiry only for channels that actually came back, dropping the rest so an owner record cannot attach itself to a later channel of the same name. Channels whose time to live passed while it was down are deleted on the first reaper pass, immediately after startup.

Three settings worth deliberate choices:

- **`encryption-passphrase`** is required unless `insecure: true`. With `insecure`, the encryption key is derived from the public `local-name`, which gives tamper detection and no confidentiality. Supply the passphrase from a Secret, never from the config file.
- **`delete-sessions-on-shutdown`** defaults to `true`, which deletes every channel on a clean shutdown — the behaviour from before persistence existed. Set it to `false`, or persistence buys nothing across a planned restart.
- **`path`** must be a private directory on a volume that survives the pod, with restrictive permissions. It holds the MLS group state of every channel.

!!! warning "Ownership must be loadable"
    If `persistence` is configured but channel ownership cannot be loaded, the Channel Manager refuses to start, with `failed to load channel ownership and grant state`. A restored channel without its owner would accept participant changes from anyone — refusing to start is the safe failure.

### Why a single replica

Run exactly one Channel Manager per state directory. Replicas do not share ownership, expiry or spent-nonce state, so two Channel Managers serving the same channels would each accept the same single-use grant once, and each hold a moderator role the other knows nothing about.

This is a correctness constraint, not a performance one. For availability, rely on the scheduler restarting the single replica onto its volume, with `strategy: Recreate` and `delete-sessions-on-shutdown: false` so channels survive the gap. To scale across many channels, run several Channel Managers with distinct `local-name` and `audience` values, each with its own state directory, and give each a disjoint set of channels.

## Rolling out require-signatures

[`require-signatures`](./security.md#require-signatures) is off by default so that clients written before signed requests keep working. Turning it on is a config change, but the migration is a client one:

1. **Find the channels that have no owner.** `slimctl channel-manager list-channels` shows an owner for the channels that have one. Those without are the ones that will stop being changeable.
2. **Move clients onto signed create requests.** Each client needs an Ed25519 key, and signs with the deployment's `audience`. The [walkthrough](./install.md#signing-the-create-request-in-code) has the code.
3. **Recreate the ownerless channels.** There is no way to adopt one: delete it and create it again with a signed request. Plan for the interruption — recreating a channel re-invites its participants.
4. **Configure operators first.** Once signatures are required, an ownerless channel can only be deleted by an operator. With none configured, such a channel cannot be deleted at all: `channel <name> has no owner and this channel manager has no operators, so it can't be deleted`.
5. **Turn it on and restart.** Startup logs `Refusing unsigned creates and changes to channels with no owner`.

## Upgrading from releases where the caller was the owner

Earlier releases recorded the *caller's authenticated identity* as a channel's owner — an OIDC subject or a SPIFFE ID. Those channels keep that owner after the upgrade, and because no signature can be verified against an identity that is not a key, their participants can no longer be changed at all. Expect `invalid grant: unsupported owner identifier format: ...` on any attempt.

What still works on such a channel: it expires if it was created with a time to live, and an operator can delete it.

So, in order:

1. **Configure at least one operator before upgrading.** Without one there is no way to clean up the frozen channels.
2. **Inventory them.** After the upgrade, `list-channels` shows owners that are not `did:key:...` — those are the frozen ones.
3. **Recreate each as an owned channel**, with a signed create request from a key, and re-invite its participants.
4. **Delete the originals** with an operator's signed delete request.

Channels created with no owner at all are unaffected by the upgrade: they keep accepting unsigned changes until `require-signatures` is turned on.

## Related

- [Installation and Walkthrough](./install.md) — Build and run the Channel Manager, and manage an owned channel
- [Configuration Reference](./config.md) — Every setting in detail
- [Security Model](./security.md) — What each identity and signature means
- [Operations and Troubleshooting](./operations.md) — Startup logs, request logs and failure modes
- [Kubernetes Deployment](../../deploy/kubernetes.md) — SPIRE and the other SLIM charts

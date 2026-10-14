# Naming

In SLIM, all endpoints are identified by a routable hierarchical name. The first three components are readable names chosen by the application. The last is an ID that SLIM derives from the application's identity, so a receiver can check a sender's name against the identity it verifies.

## Client Names

Client names follow this pattern:

```text
organization/namespace/service/<id>
```

- **Organization**: Identifies the organization or team that owns the application.
- **Namespace**: Segments deployments within an organization — for example by region, environment, or tenant.
- **Service**: Identifies the logical service. All instances of the same service (e.g. multiple pods in Kubernetes) share the same `organization/namespace/service` prefix.
- **Client ID**: The XXH3-128 hash of the application's [identity](./authentication.md#application-identity), the `sub` it authenticates with. For a `did:key` identity, that is its [DID](https://www.w3.org/TR/did-core/), which encodes its public key.

Instances with their own identity have their own ID. Instances that share one, such as replicas running as the same SPIFFE ID or with the same key, also share their ID and full name.

## Anycast and Unicast Delivery

The four-component structure supports two delivery modes:

- **Anycast** (`organization/namespace/service`): Addressing only the first three components delivers the message to any one running instance of that service. SLIM routes to whichever instance is reachable. This is primarily used during session discovery — before a session has bound to a specific instance.
- **Unicast** (`organization/namespace/service/<id>`): Including the full four-component name delivers the message to the instance with that ID, or to one of them if several share it.

## Names and Identities

Nodes route on names and don't check who subscribes to one. Sessions do: with [identity binding](./authentication.md#name-ids-and-identity-binding) on, a peer's name ID must be the one its verified identity derives. XXH3 isn't a cryptographic hash, though, so a name ID is for routing; it doesn't prove who a peer is.

To be sure who is on the other end, pin the peer's identity rather than its name: its DID for a `did:key` identity. A pinned session admits the peer only if it is verified as that identity.

## Channel Names

Channels are named endpoints that multiple clients can join to receive every message published to that channel. They underpin [group sessions](./sessions/index.md#group-session) and multicast communication.

A channel name uses the same four-component structure. Each group is associated with two channel names: a **data channel** and a **control channel**. They share the same `organization/namespace/service` prefix, and their IDs are XXH3-128 hashes of the moderator's identity and the channel's name, so they are not fixed well-known values. With identity binding on, an invitee checks them against the moderator's verified identity.

For further details, refer to the [SLIM Specification](https://datatracker.ietf.org/doc/draft-mpsb-agntcy-slim/).
